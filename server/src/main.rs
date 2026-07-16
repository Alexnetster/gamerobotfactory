mod config;
mod game_state;
mod protocol;
mod delta;
mod metrics;
mod persistence;
mod session;
mod ws;

use axum::extract::Extension;
use axum::response::IntoResponse;
use axum::{routing::get, Router};
use config::{get_config, post_config, AppConfig, ConfigHandle};
use delta::compute_delta;
use game_state::GameState;
use metrics::{metrics_route, Metrics, MetricsHandle};
use protocol::to_snapshot;
use sim_core::grid::Grid;
use sim_core::production::total_production;
use sim_core::sim::{tick, SimState};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tokio::sync::Mutex;
use ws::{ws_route, Broadcaster, SharedState};

type DbHandle = Arc<StdMutex<rusqlite::Connection>>;

async fn health() -> &'static str {
    "ok"
}

async fn stats_history(Extension(db): Extension<DbHandle>) -> impl IntoResponse {
    let rows = tokio::task::spawn_blocking(move || {
        let conn = db.lock().unwrap();
        persistence::recent_stats(&conn, 50)
    })
    .await;

    match rows {
        Ok(Ok(rows)) => axum::Json(rows).into_response(),
        Ok(Err(err)) => {
            tracing::error!(%err, "failed to read stats history");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
        }
        Err(join_err) => {
            tracing::error!(%join_err, "stats history query task panicked");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "internal error reading stats history".to_string())
                .into_response()
        }
    }
}

async fn robot_failures(Extension(db): Extension<DbHandle>) -> impl IntoResponse {
    let rows = tokio::task::spawn_blocking(move || {
        let conn = db.lock().unwrap();
        persistence::recent_failure_events(&conn, 50)
    })
    .await;

    match rows {
        Ok(Ok(rows)) => axum::Json(rows).into_response(),
        Ok(Err(err)) => {
            tracing::error!(%err, "failed to read robot failure history");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
        }
        Err(join_err) => {
            tracing::error!(%join_err, "robot failure history query task panicked");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "internal error reading robot failure history".to_string())
                .into_response()
        }
    }
}

fn initial_state() -> SharedState {
    let sim = SimState { grid: Arc::new(Grid::new(10, 10)), robots: Vec::new(), tick_count: 0 };
    Arc::new(Mutex::new(GameState::new(sim)))
}

const TICK_INTERVAL: Duration = Duration::from_millis(50); // 20Hz

/// `sim_core::sim::tick`을 패닉으로부터 격리한다. 패닉이 나면 이번 틱은
/// 건너뛰고(시뮬레이션 상태는 직전 틱 그대로 유지) 서버 프로세스와 다른
/// 연결은 영향받지 않는다. `SimState`(Arc<Grid> + Vec<Robot>, 둘 다 내부
/// 가변성 없음)는 현재 이 가정을 안전하게 만족하지만, 나중에 여기 어딘가에
/// 내부 가변성(Cell/RefCell/Mutex 등)이 들어오면 이 안전성 논리를 다시
/// 검토해야 한다 — `sim_core::sim`의 `safe_call` 주석과 같은 이유.
/// 의존: 이 크레이트가 `panic = "abort"` 프로파일을 쓰지 않는다는 것 —
/// 그 경우 `catch_unwind`는 컴파일 경고 없이 조용히 무력화된다.
fn safe_tick(sim: &SimState) -> Option<SimState> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick(sim))) {
        Ok(next) => Some(next),
        Err(_) => {
            tracing::error!("tick() panicked; skipping this tick, simulation state unchanged");
            None
        }
    }
}

/// 이전 틱과 이번 틱의 로봇별 상태를 ID 기준으로 비교해, 로그해야 할 전이
/// (Operational -> Failed, Repairing -> Operational)를 찾아낸다. 순수
/// 함수로 분리해둔 이유: 실제 tick()/rayon/결정적 확률 굴림 없이도 이
/// 로직 자체를 빠르고 결정적으로 단위테스트할 수 있게 하기 위함 — 실제
/// 확률적 사건이 벽시계 안에서 일어나길 기다리는 통합테스트는 느리고
/// 취약해지기 쉽다(Task 8 Lagged 리싱크 사건과 같은 이유). `ws.rs`의
/// `decide_broadcast_update`와 같은 패턴.
fn detect_status_transitions(
    previous_robots: &[protocol::RobotView],
    current_robots: &[protocol::RobotView],
    tick: u64,
) -> Vec<persistence::FailureEvent> {
    let mut events = Vec::new();
    for current in current_robots {
        let Some(previous) = previous_robots.iter().find(|p| p.id == current.id) else { continue };
        match (previous.status, current.status) {
            (protocol::WireStatus::Operational, protocol::WireStatus::Failed) => {
                events.push(persistence::FailureEvent {
                    tick,
                    robot_id: current.id,
                    event_type: "failed".to_string(),
                });
            }
            (protocol::WireStatus::Repairing { .. }, protocol::WireStatus::Operational) => {
                events.push(persistence::FailureEvent {
                    tick,
                    robot_id: current.id,
                    event_type: "repaired".to_string(),
                });
            }
            _ => {}
        }
    }
    events
}

/// 백그라운드에서 20Hz로 시뮬레이션을 전진시키고, 마지막으로 브로드캐스트한
/// 스냅샷과 비교한 델타를 연결된 모든 클라이언트에 보낸다. `state.lock()`
/// 가드는 이 블록 안(동기 연산: tick/생산량 집계/스냅샷 변환)에서만
/// 살아있고, `broadcaster.send(...)`는 가드가 이미 드롭된 뒤 호출된다 —
/// 브로드캐스트 채널의 `send`는 블로킹 I/O가 아니라 동기 함수이므로
/// 락을 오래 들고 있을 일이 없다(Task 7 리뷰에서 나온 불변식).
fn spawn_tick_loop(
    state: SharedState,
    broadcaster: Broadcaster,
    db: DbHandle,
    config: ConfigHandle,
    metrics: MetricsHandle,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        let mut last_snapshot = {
            let guard = state.lock().await;
            to_snapshot(&guard, uuid::Uuid::nil())
        };

        loop {
            interval.tick().await;

            let persist_every_n_ticks = { *config.lock().await }.persist_every_n_ticks;

            // 이 틱의 처리 시간을 측정한다: 락 획득부터(경합이 있으면 그 대기까지
            // 포함) tick/생산량 집계/스냅샷·델타 계산까지 — 디자인 문서의
            // "틱 처리 시간 p99 < 10ms" 목표가 재는 대상이 바로 이 구간이다.
            // 아래 `broadcaster.send(...)`(락 해제 후 fire-and-forget)와
            // `spawn_blocking` 영속화 디스패치(비동기, 락 밖)는 20Hz 틱 예산과
            // 직접 경합하는 동기 작업이 아니므로 측정 범위에서 제외한다.
            let tick_processing_start = std::time::Instant::now();
            let (message, next_snapshot, should_persist, stats_row, failure_events) = {
                let mut guard = state.lock().await;
                match safe_tick(&guard.sim) {
                    Some(next_sim) => guard.sim = next_sim,
                    None => metrics.tick_panics_total.inc(),
                }

                let mut total_production_value = 0.0_f32;
                if guard.conveyor.running {
                    let units: HashMap<u32, f32> = guard.sim.robots.iter().map(|r| (r.id, 0.01)).collect();
                    total_production_value = total_production(&guard.sim.robots, &units);
                }

                metrics.ticks_total.inc();
                metrics.robot_count.set(guard.sim.robots.len() as i64);
                metrics.robots_repairing.set(
                    guard
                        .sim
                        .robots
                        .iter()
                        .filter(|r| matches!(r.status, sim_core::sim::RobotStatus::Repairing { .. }))
                        .count() as i64,
                );

                let current_snapshot = to_snapshot(&guard, uuid::Uuid::nil());
                let (delta, failure_events) = match (&last_snapshot, &current_snapshot) {
                    (
                        protocol::ServerMessage::Snapshot { conveyor: prev_conveyor, robots: prev_robots, .. },
                        protocol::ServerMessage::Snapshot { tick: cur_tick, conveyor: cur_conveyor, robots: cur_robots, .. },
                    ) => {
                        let delta = compute_delta(*prev_conveyor, prev_robots, *cur_tick, *cur_conveyor, cur_robots);
                        let events = detect_status_transitions(prev_robots, cur_robots, *cur_tick);
                        (delta, events)
                    }
                    _ => (current_snapshot.clone(), Vec::new()),
                };

                for event in &failure_events {
                    if event.event_type == "failed" {
                        metrics.robot_failures_total.inc();
                    }
                }

                let should_persist = guard.sim.tick_count % persist_every_n_ticks == 0;
                let stats_row = persistence::StatsRow {
                    tick: guard.sim.tick_count,
                    robot_count: guard.sim.robots.len(),
                    conveyor_running: guard.conveyor.running,
                    total_production: total_production_value,
                };

                (delta, current_snapshot, should_persist, stats_row, failure_events)
            };
            metrics.tick_duration_seconds.observe(tick_processing_start.elapsed().as_secs_f64());

            last_snapshot = next_snapshot;
            let _ = broadcaster.send(message);

            if should_persist {
                let db = Arc::clone(&db);
                tokio::task::spawn_blocking(move || {
                    let conn = match db.lock() {
                        Ok(conn) => conn,
                        Err(err) => {
                            tracing::error!(%err, "db mutex poisoned; skipping this persist attempt");
                            return;
                        }
                    };
                    if let Err(err) = persistence::insert_stats(&conn, &stats_row) {
                        tracing::error!(%err, "failed to persist stats row");
                    }
                });
            }

            if !failure_events.is_empty() {
                let db = Arc::clone(&db);
                tokio::task::spawn_blocking(move || {
                    let conn = match db.lock() {
                        Ok(conn) => conn,
                        Err(err) => {
                            tracing::error!(%err, "db mutex poisoned; skipping failure event persist");
                            return;
                        }
                    };
                    for event in &failure_events {
                        if let Err(err) =
                            persistence::insert_failure_event(&conn, event.tick, event.robot_id, &event.event_type)
                        {
                            tracing::error!(%err, "failed to persist robot failure event");
                        }
                    }
                });
            }
        }
    });
}

pub fn build_app(
    state: SharedState,
    broadcaster: Broadcaster,
    sessions: ws::SessionHandle,
    db: DbHandle,
    config: ConfigHandle,
    metrics: MetricsHandle,
) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_route))
        .route("/api/stats/history", get(stats_history))
        .route("/api/robots/failures", get(robot_failures))
        .route("/api/config", get(get_config).post(post_config))
        .route("/metrics", get(metrics_route))
        .with_state(state)
        .layer(axum::extract::Extension(broadcaster))
        .layer(axum::extract::Extension(sessions))
        .layer(axum::extract::Extension(db))
        .layer(axum::extract::Extension(config))
        .layer(axum::extract::Extension(metrics))
}

/// 포트를 고정하지 않고 OS가 빈 포트를 골라주게 한다(`:0`) — 통합테스트
/// (Task 10)에서 여러 서버 인스턴스를 동시에 띄워도 포트 충돌이 나지
/// 않도록 하기 위함. 실제 바인딩된 포트는 표준출력에 기계가 파싱하기
/// 쉬운 한 줄(`LISTENING_PORT={port}`)로 알려준다.
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let state = initial_state();
    let (broadcaster, _rx) = tokio::sync::broadcast::channel::<protocol::ServerMessage>(32);
    // 32 messages ≈ 1.6s of buffer at the 20Hz tick rate. Not load-tested;
    // revisit alongside a future plan's reconnect/resync work if lagged
    // disconnects turn out to be a real problem in practice.
    let sessions: ws::SessionHandle = Arc::new(Mutex::new(session::SessionRegistry::new()));

    let db_path = std::env::var("GAMEROBOTFACTORY_DB_PATH").unwrap_or_else(|_| "gamerobotfactory.sqlite3".to_string());
    let db: DbHandle = Arc::new(StdMutex::new(
        persistence::open_db(&db_path).expect("failed to open sqlite db"),
    ));
    let config: ConfigHandle = Arc::new(Mutex::new(AppConfig::default()));
    let metrics: MetricsHandle = Arc::new(Metrics::new());

    spawn_tick_loop(state.clone(), broadcaster.clone(), db.clone(), config.clone(), metrics.clone());

    let app = build_app(state, broadcaster, sessions, db, config, metrics);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind 127.0.0.1:0");
    println!("LISTENING_PORT={}", listener.local_addr().unwrap().port());
    axum::serve(listener, app).await.expect("server exited with an error");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_tick_passes_through_normal_ticks_unchanged() {
        let sim = SimState { grid: Arc::new(Grid::new(3, 3)), robots: Vec::new(), tick_count: 5 };
        let result = safe_tick(&sim);
        assert!(result.is_some());
        assert_eq!(result.unwrap().tick_count, 6);
    }

    #[test]
    fn catch_unwind_recovers_from_a_panic_the_same_way_safe_tick_does() {
        // sim_core::sim::tick() 자체를 결정적으로 패닉시키려면 이미 완성되어
        // 포트폴리오 리뷰까지 거친 Plan 1 라이브러리에 결함 주입 지점을 다시
        // 여는 작업이 필요한데, 이는 이 태스크 범위 밖이다. 대신 safe_tick이
        // 실제로 쓰는 것과 동일한 catch_unwind 복구 경로를 직접 검증한다 —
        // Plan 1의 safe_call 테스트, Plan 2의 goal-exception 재검증과 같은
        // 패턴(패턴 자체를 검증 + 정상 경로 테스트로 실제 배선까지 커버).
        let result: std::thread::Result<i32> = std::panic::catch_unwind(|| panic!("simulated tick fault"));
        assert!(result.is_err());
    }

    fn sample_robot_view(id: u32, status: protocol::WireStatus) -> protocol::RobotView {
        protocol::RobotView {
            id,
            pos: protocol::WireCellId { x: 0, y: 0 },
            pose: protocol::WirePose::Standing,
            leg_cycle_progress: 0.0,
            task: protocol::WireTask::Idle,
            status,
            durability_remaining: 1.0,
        }
    }

    #[test]
    fn detects_a_new_failure() {
        let previous = vec![sample_robot_view(1, protocol::WireStatus::Operational)];
        let current = vec![sample_robot_view(1, protocol::WireStatus::Failed)];

        let events = detect_status_transitions(&previous, &current, 42);

        assert_eq!(events, vec![persistence::FailureEvent { tick: 42, robot_id: 1, event_type: "failed".to_string() }]);
    }

    #[test]
    fn detects_a_completed_repair() {
        let previous = vec![sample_robot_view(1, protocol::WireStatus::Repairing { remaining_ticks: 1 })];
        let current = vec![sample_robot_view(1, protocol::WireStatus::Operational)];

        let events = detect_status_transitions(&previous, &current, 7);

        assert_eq!(events, vec![persistence::FailureEvent { tick: 7, robot_id: 1, event_type: "repaired".to_string() }]);
    }

    #[test]
    fn no_event_for_an_unrelated_change() {
        let previous = vec![sample_robot_view(1, protocol::WireStatus::Operational)];
        let current = vec![sample_robot_view(1, protocol::WireStatus::Operational)];

        let events = detect_status_transitions(&previous, &current, 1);

        assert!(events.is_empty());
    }

    #[test]
    fn no_event_for_a_removed_robot() {
        let previous = vec![sample_robot_view(1, protocol::WireStatus::Failed)];
        let current: Vec<protocol::RobotView> = vec![];

        let events = detect_status_transitions(&previous, &current, 1);

        assert!(events.is_empty(), "a removed robot should not generate a spurious event");
    }

    #[test]
    fn no_event_for_a_brand_new_robot() {
        let previous: Vec<protocol::RobotView> = vec![];
        let current = vec![sample_robot_view(5, protocol::WireStatus::Operational)];

        let events = detect_status_transitions(&previous, &current, 1);

        assert!(events.is_empty(), "a robot with no previous entry should not be treated as a transition");
    }
}
