use crate::game_state::GameState;
use crate::protocol::{to_snapshot, ClientCommand, ServerMessage};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::Arc;
use tokio::sync::Mutex;

pub type SharedState = Arc<Mutex<GameState>>;
pub type Broadcaster = tokio::sync::broadcast::Sender<crate::protocol::ServerMessage>;

pub async fn ws_route(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    axum::extract::Extension(broadcaster): axum::extract::Extension<Broadcaster>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, broadcaster))
}

async fn handle_socket(mut socket: WebSocket, state: SharedState, broadcaster: Broadcaster) {
    // 구독을 스냅샷 전송보다 먼저 시작한다 — 스냅샷 전송은 소켓 I/O라
    // await 지점에서 양보(yield)할 수 있고, 그 사이에 틱 루프가
    // 브로드캐스트를 하나 흘리면 그 델타는 이 커넥션에 영원히 유실된다
    // (틱 루프의 `last_snapshot`은 클라이언트별이 아니라 전역 공유
    // 기준선이므로, 한 번 놓친 변경은 다시 오지 않는다).
    let mut updates = broadcaster.subscribe();

    {
        let snapshot = {
            let guard = state.lock().await;
            to_snapshot(&guard)
        };
        if send_message(&mut socket, &snapshot).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientCommand>(&text) {
                            Ok(command) => {
                                let mut guard = state.lock().await;
                                apply_command(&mut guard, command);
                            }
                            Err(err) => eprintln!("invalid client command, ignoring: {err}"),
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        eprintln!("websocket error, closing connection: {err}");
                        break;
                    }
                    None => break,
                }
            }
            update = updates.recv() => {
                match update {
                    Ok(message) => {
                        if send_message(&mut socket, &message).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                    // NOTE: this collapses tokio::sync::broadcast::error::RecvError::Lagged
                    // (transient buffer overrun) and ::Closed (channel gone) into the same
                    // "disconnect" behavior. A lagged client should ideally resync with a
                    // fresh to_snapshot() instead of being dropped — deferred to a future
                    // plan, since reconnect/resync semantics need real design work there.
                }
            }
        }
    }
}

fn apply_command(state: &mut GameState, command: ClientCommand) {
    use crate::protocol::ClientCommand::*;
    match command {
        SelectRobot { robot_id } => {
            if let Err(err) = state.select_robot(robot_id) {
                eprintln!("SelectRobot rejected: {err:?}");
            }
        }
        ReleaseRobot => state.release_robot(),
        ToggleConveyor => state.toggle_conveyor(),
        SetRobotCount { count } => state.set_robot_count(count),
        TriggerArmAction { robot_id, task } => {
            if let Err(err) = state.trigger_arm_action(robot_id, task.into()) {
                eprintln!("TriggerArmAction rejected: {err:?}");
            }
        }
    }
}

async fn send_message(socket: &mut WebSocket, message: &ServerMessage) -> Result<(), axum::Error> {
    let text = serde_json::to_string(message).expect("ServerMessage always serializes");
    socket.send(Message::Text(text)).await
}
