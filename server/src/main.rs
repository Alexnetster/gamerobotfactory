mod game_state;
mod protocol;
mod delta;
mod ws;

use axum::{routing::get, Router};
use game_state::GameState;
use sim_core::grid::Grid;
use sim_core::sim::SimState;
use std::sync::Arc;
use tokio::sync::Mutex;
use ws::{ws_route, SharedState};

async fn health() -> &'static str {
    "ok"
}

fn initial_state() -> SharedState {
    let sim = SimState { grid: Arc::new(Grid::new(10, 10)), robots: Vec::new(), tick_count: 0 };
    Arc::new(Mutex::new(GameState::new(sim)))
}

/// 포트를 고정하지 않고 OS가 빈 포트를 골라주게 한다(`:0`) — 통합테스트
/// (Task 10)에서 여러 서버 인스턴스를 동시에 띄워도 포트 충돌이 나지
/// 않도록 하기 위함. 실제 바인딩된 포트는 표준출력에 기계가 파싱하기
/// 쉬운 한 줄(`LISTENING_PORT={port}`)로 알려준다.
#[tokio::main]
async fn main() {
    let state = initial_state();

    let app = Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_route))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind 127.0.0.1:0");
    println!("LISTENING_PORT={}", listener.local_addr().unwrap().port());
    axum::serve(listener, app).await.expect("server exited with an error");
}
