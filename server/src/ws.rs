use crate::game_state::GameState;
use crate::protocol::{to_snapshot, ClientCommand, ServerMessage};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::Arc;
use tokio::sync::Mutex;

pub type SharedState = Arc<Mutex<GameState>>;
pub type Broadcaster = tokio::sync::broadcast::Sender<crate::protocol::ServerMessage>;
pub type SessionHandle = Arc<Mutex<crate::session::SessionRegistry>>;

pub async fn ws_route(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    axum::extract::Extension(broadcaster): axum::extract::Extension<Broadcaster>,
    axum::extract::Extension(sessions): axum::extract::Extension<SessionHandle>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, broadcaster, sessions))
}

async fn handle_socket(mut socket: WebSocket, state: SharedState, broadcaster: Broadcaster, sessions: SessionHandle) {
    // 구독을 스냅샷 전송보다 먼저 시작한다 — 스냅샷 전송은 소켓 I/O라
    // await 지점에서 양보(yield)할 수 있고, 그 사이에 틱 루프가
    // 브로드캐스트를 하나 흘리면 그 델타는 이 커넥션에 영원히 유실된다
    // (틱 루프의 `last_snapshot`은 클라이언트별이 아니라 전역 공유
    // 기준선이므로, 한 번 놓친 변경은 다시 오지 않는다).
    let mut updates = broadcaster.subscribe();

    let own_session_id = {
        let mut registry = sessions.lock().await;
        registry.start_session(std::time::Instant::now())
    };

    {
        let snapshot = {
            let guard = state.lock().await;
            to_snapshot(&guard, own_session_id)
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
                            Ok(ClientCommand::Resume { session_id }) => {
                                let now = std::time::Instant::now();
                                let resumed = {
                                    let mut registry = sessions.lock().await;
                                    let within = registry.is_within_grace_period(session_id, now);
                                    if within {
                                        registry.touch(session_id, now);
                                    }
                                    within
                                };
                                let ack = ServerMessage::ResumeAck {
                                    v: crate::protocol::PROTOCOL_VERSION,
                                    session_id,
                                    resumed,
                                };
                                if send_message(&mut socket, &ack).await.is_err() {
                                    break;
                                }
                            }
                            Ok(command) => {
                                let mut guard = state.lock().await;
                                apply_command(&mut guard, command);
                            }
                            Err(err) => tracing::warn!(error = %err, "invalid client command, ignoring"),
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        tracing::warn!(error = %err, "websocket error, closing connection");
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
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(missed_messages = n, "client lagged behind; resyncing with a full snapshot");
                        let snapshot = {
                            let guard = state.lock().await;
                            to_snapshot(&guard, own_session_id)
                        };
                        if send_message(&mut socket, &snapshot).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
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
                tracing::warn!(?err, "SelectRobot rejected");
            }
        }
        ReleaseRobot => state.release_robot(),
        ToggleConveyor => state.toggle_conveyor(),
        SetRobotCount { count } => state.set_robot_count(count),
        TriggerArmAction { robot_id, task } => {
            if let Err(err) = state.trigger_arm_action(robot_id, task.into()) {
                tracing::warn!(?err, "TriggerArmAction rejected");
            }
        }
        // handle_socket intercepts Resume before calling apply_command, so this
        // arm should be unreachable in practice; kept only to satisfy the
        // exhaustive match now that ClientCommand has a Resume variant.
        Resume { .. } => unreachable!("Resume is intercepted in handle_socket before reaching apply_command"),
    }
}

async fn send_message(socket: &mut WebSocket, message: &ServerMessage) -> Result<(), axum::Error> {
    let text = serde_json::to_string(message).expect("ServerMessage always serializes");
    socket.send(Message::Text(text)).await
}
