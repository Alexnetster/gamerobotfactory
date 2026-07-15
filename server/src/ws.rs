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
    {
        let snapshot = {
            let guard = state.lock().await;
            to_snapshot(&guard)
        };
        if send_message(&mut socket, &snapshot).await.is_err() {
            return;
        }
    }

    let mut updates = broadcaster.subscribe();

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
