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

/// Ties the `connected_clients` gauge's lifetime to this connection's scope.
/// `handle_socket` has several exit points (an early `return` after a failed
/// initial snapshot send, and several `break`s inside the `tokio::select!`
/// loop for socket errors, normal close, and a closed broadcast channel).
/// Manually pairing `inc`/`dec` at each of those would silently leak the
/// gauge upward forever if any one of them were missed (flagged in Task 5's
/// code review) — RAII makes that impossible by construction, since `Drop`
/// runs no matter which path out of the function is taken, panics included.
struct ConnectionGuard {
    counter: prometheus::IntGauge,
}

impl ConnectionGuard {
    fn new(counter: &prometheus::IntGauge) -> Self {
        counter.inc();
        ConnectionGuard { counter: counter.clone() }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.counter.dec();
    }
}

pub async fn ws_route(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    axum::extract::Extension(broadcaster): axum::extract::Extension<Broadcaster>,
    axum::extract::Extension(sessions): axum::extract::Extension<SessionHandle>,
    axum::extract::Extension(metrics): axum::extract::Extension<crate::metrics::MetricsHandle>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, broadcaster, sessions, metrics))
}

async fn handle_socket(
    mut socket: WebSocket,
    state: SharedState,
    broadcaster: Broadcaster,
    sessions: SessionHandle,
    metrics: crate::metrics::MetricsHandle,
) {
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

    let _connection_guard = ConnectionGuard::new(&metrics.connected_clients);

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
                match decide_broadcast_update(update) {
                    BroadcastUpdate::Forward(message) => {
                        if send_message(&mut socket, &message).await.is_err() {
                            break;
                        }
                    }
                    BroadcastUpdate::Resync => {
                        let snapshot = {
                            let guard = state.lock().await;
                            to_snapshot(&guard, own_session_id)
                        };
                        if send_message(&mut socket, &snapshot).await.is_err() {
                            break;
                        }
                    }
                    BroadcastUpdate::Close => break,
                }
            }
        }
    }
}

/// What `handle_socket` should do in response to a single receive from the
/// per-connection broadcast subscriber. Pulled out of the `select!` arm so
/// the decision itself — forward / resync / close — can be unit-tested
/// against a real `tokio::sync::broadcast` channel without needing an
/// actual OS socket to fall behind (which, on a real TCP loopback
/// connection, effectively never happens within a normal test's
/// wall-clock budget: the OS send buffer is many times larger than what a
/// few seconds of 20Hz deltas produce, so `socket.send()` never blocks and
/// `updates.recv()` is never starved — see the code review that caught
/// this and led to this refactor).
#[derive(Debug, PartialEq)]
enum BroadcastUpdate {
    /// Forward this message to the client as-is.
    Forward(ServerMessage),
    /// The client fell behind the broadcast channel's buffer; resync with
    /// a fresh full snapshot instead of trying to replay what it missed.
    Resync,
    /// The broadcast channel itself is closed; end the connection.
    Close,
}

fn decide_broadcast_update(
    result: Result<ServerMessage, tokio::sync::broadcast::error::RecvError>,
) -> BroadcastUpdate {
    match result {
        Ok(message) => BroadcastUpdate::Forward(message),
        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
            tracing::warn!(missed_messages = n, "client lagged behind; resyncing with a full snapshot");
            BroadcastUpdate::Resync
        }
        Err(tokio::sync::broadcast::error::RecvError::Closed) => BroadcastUpdate::Close,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    fn sample_message(tick: u64) -> ServerMessage {
        ServerMessage::Delta { v: 1, tick, conveyor: None, changed_robots: vec![], removed_robot_ids: vec![] }
    }

    #[test]
    fn ok_message_is_forwarded_unchanged() {
        let message = sample_message(1);
        let decision = decide_broadcast_update(Ok(message.clone()));
        assert_eq!(decision, BroadcastUpdate::Forward(message));
    }

    #[test]
    fn closed_channel_ends_the_connection() {
        let decision = decide_broadcast_update(Err(broadcast::error::RecvError::Closed));
        assert_eq!(decision, BroadcastUpdate::Close);
    }

    /// This is the deterministic replacement for the wall-clock integration
    /// test: instead of hoping a real OS socket falls behind within a few
    /// seconds (it doesn't — see `decide_broadcast_update`'s doc comment),
    /// we drive a real `tokio::sync::broadcast` channel past its capacity
    /// directly and feed the genuine `RecvError::Lagged` it produces into
    /// the function under test.
    #[tokio::test]
    async fn real_lagged_broadcast_error_produces_a_resync_decision() {
        let (tx, mut rx) = broadcast::channel(32);

        // Send well past the channel's capacity without ever calling
        // `recv`, so the subscriber is guaranteed to have lagged.
        for tick in 0..40u64 {
            tx.send(sample_message(tick)).expect("at least one receiver is still subscribed");
        }

        let result = rx.recv().await;
        assert!(
            matches!(result, Err(broadcast::error::RecvError::Lagged(_))),
            "expected the receiver to report Lagged after overflowing the channel, got {result:?}"
        );

        let decision = decide_broadcast_update(result);
        assert_eq!(decision, BroadcastUpdate::Resync, "a lagged receiver must resync, not close or silently forward");
    }
}
