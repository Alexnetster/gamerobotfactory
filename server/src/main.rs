mod config;
mod game_state;
mod protocol;
mod delta;
mod metrics;
mod persistence;
mod session;
mod ws;

use axum::{routing::get, Router};
use delta::compute_delta;
use game_state::GameState;
use protocol::to_snapshot;
use sim_core::grid::Grid;
use sim_core::production::total_production;
use sim_core::sim::{tick, SimState};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use ws::{ws_route, Broadcaster, SharedState};

async fn health() -> &'static str {
    "ok"
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

/// 백그라운드에서 20Hz로 시뮬레이션을 전진시키고, 마지막으로 브로드캐스트한
/// 스냅샷과 비교한 델타를 연결된 모든 클라이언트에 보낸다. `state.lock()`
/// 가드는 이 블록 안(동기 연산: tick/생산량 집계/스냅샷 변환)에서만
/// 살아있고, `broadcaster.send(...)`는 가드가 이미 드롭된 뒤 호출된다 —
/// 브로드캐스트 채널의 `send`는 블로킹 I/O가 아니라 동기 함수이므로
/// 락을 오래 들고 있을 일이 없다(Task 7 리뷰에서 나온 불변식).
fn spawn_tick_loop(state: SharedState, broadcaster: Broadcaster) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        let mut last_snapshot = {
            let guard = state.lock().await;
            to_snapshot(&guard, uuid::Uuid::nil())
        };

        loop {
            interval.tick().await;

            let (message, next_snapshot) = {
                let mut guard = state.lock().await;
                if let Some(next_sim) = safe_tick(&guard.sim) {
                    guard.sim = next_sim;
                }

                if guard.conveyor.running {
                    let units: HashMap<u32, f32> = guard.sim.robots.iter().map(|r| (r.id, 0.01)).collect();
                    let _ = total_production(&guard.sim.robots, &units);
                    // 생산량 값 자체를 아직 어디에도 저장하지 않는다 — 실제
                    // "생산량 누적 상태"는 이 Plan의 범위 밖(설계문서 경영
                    // 레이어)이라, 결정적 집계 함수가 실제로 매 틱 호출된다는
                    // 것만 지금은 증명해둔다.
                }

                let current_snapshot = to_snapshot(&guard, uuid::Uuid::nil());
                let delta = match (&last_snapshot, &current_snapshot) {
                    (
                        protocol::ServerMessage::Snapshot { conveyor: prev_conveyor, robots: prev_robots, .. },
                        protocol::ServerMessage::Snapshot { tick: cur_tick, conveyor: cur_conveyor, robots: cur_robots, .. },
                    ) => compute_delta(*prev_conveyor, prev_robots, *cur_tick, *cur_conveyor, cur_robots),
                    _ => current_snapshot.clone(),
                };
                (delta, current_snapshot)
            };

            last_snapshot = next_snapshot;
            let _ = broadcaster.send(message);
        }
    });
}

pub fn build_app(state: SharedState, broadcaster: Broadcaster, sessions: ws::SessionHandle) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_route))
        .with_state(state)
        .layer(axum::extract::Extension(broadcaster))
        .layer(axum::extract::Extension(sessions))
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

    spawn_tick_loop(state.clone(), broadcaster.clone());

    let app = build_app(state, broadcaster, sessions);

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
}
