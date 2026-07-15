mod delta;
mod game_state;
mod protocol;

use axum::{routing::get, Router};

async fn health() -> &'static str {
    "ok"
}

/// 포트를 고정하지 않고 OS가 빈 포트를 골라주게 한다(`:0`) — 통합테스트
/// (Task 10)에서 여러 서버 인스턴스를 동시에 띄워도 포트 충돌이 나지
/// 않도록 하기 위함. 실제 바인딩된 포트는 표준출력에 기계가 파싱하기
/// 쉬운 한 줄(`LISTENING_PORT={port}`)로 알려준다.
#[tokio::main]
async fn main() {
    let app = Router::new().route("/health", get(health));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    println!("LISTENING_PORT={}", listener.local_addr().unwrap().port());
    axum::serve(listener, app).await.unwrap();
}
