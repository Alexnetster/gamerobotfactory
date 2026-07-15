mod game_state;
mod protocol;
mod delta;
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
            to_snapshot(&guard)
        };

        loop {
            interval.tick().await;

            let (message, next_snapshot) = {
                let mut guard = state.lock().await;
                guard.sim = tick(&guard.sim);

                if guard.conveyor.running {
                    let units: HashMap<u32, f32> = guard.sim.robots.iter().map(|r| (r.id, 0.01)).collect();
                    let _ = total_production(&guard.sim.robots, &units);
                    // 생산량 값 자체를 아직 어디에도 저장하지 않는다 — 실제
                    // "생산량 누적 상태"는 이 Plan의 범위 밖(설계문서 경영
                    // 레이어)이라, 결정적 집계 함수가 실제로 매 틱 호출된다는
                    // 것만 지금은 증명해둔다.
                }

                let current_snapshot = to_snapshot(&guard);
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

pub fn build_app(state: SharedState, broadcaster: Broadcaster) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_route))
        .with_state(state)
        .layer(axum::extract::Extension(broadcaster))
}

/// 포트를 고정하지 않고 OS가 빈 포트를 골라주게 한다(`:0`) — 통합테스트
/// (Task 10)에서 여러 서버 인스턴스를 동시에 띄워도 포트 충돌이 나지
/// 않도록 하기 위함. 실제 바인딩된 포트는 표준출력에 기계가 파싱하기
/// 쉬운 한 줄(`LISTENING_PORT={port}`)로 알려준다.
#[tokio::main]
async fn main() {
    let state = initial_state();
    let (broadcaster, _rx) = tokio::sync::broadcast::channel::<protocol::ServerMessage>(32);
    // 32 messages ≈ 1.6s of buffer at the 20Hz tick rate. Not load-tested;
    // revisit alongside Task 9's reconnect/resync work if lagged
    // disconnects turn out to be a real problem in practice.

    spawn_tick_loop(state.clone(), broadcaster.clone());

    let app = build_app(state, broadcaster);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind 127.0.0.1:0");
    println!("LISTENING_PORT={}", listener.local_addr().unwrap().port());
    axum::serve(listener, app).await.expect("server exited with an error");
}
