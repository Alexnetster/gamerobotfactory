use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

struct ServerProcess {
    child: Child,
    port: u16,
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// 서버 바이너리를 포트 0(임의 할당)으로 띄우고, 표준출력의
/// `LISTENING_PORT={port}` 줄을 읽어 실제로 바인딩된 포트를 알아낸다.
/// 매 테스트가 자기 자신의 서버 인스턴스 + 자기 자신의 포트를 가지므로,
/// 테스트를 병렬로 돌려도(기본 `cargo test` 동작) 포트 충돌이 날 수
/// 없다 — 특정 순서/직렬 실행에 의존하지 않는다.
fn spawn_server() -> ServerProcess {
    let mut child = Command::new(env!("CARGO_BIN_EXE_server"))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start server binary");

    let stdout: ChildStdout = child.stdout.take().expect("child stdout was not piped");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("failed to read announce line from server stdout");
    let port: u16 = line
        .trim()
        .strip_prefix("LISTENING_PORT=")
        .unwrap_or_else(|| panic!("unexpected server announce line: {line:?}"))
        .parse()
        .expect("LISTENING_PORT value was not a valid port number");

    ServerProcess { child, port }
}

#[tokio::test]
async fn connects_and_receives_initial_snapshot_then_reacts_to_commands() {
    let server = spawn_server();

    let url = format!("ws://127.0.0.1:{}/ws", server.port);
    let (ws_stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("failed to connect to ws endpoint");
    let (mut write, mut read) = ws_stream.split();

    // 1) 최초 스냅샷을 받는다.
    let first = read.next().await.expect("stream ended early").expect("ws error");
    let Message::Text(text) = first else { panic!("expected text message") };
    let json: Value = serde_json::from_str(&text).expect("initial message should be valid JSON");
    assert_eq!(json["kind"], "Snapshot");
    assert_eq!(json["robots"].as_array().expect("robots should be an array").len(), 0);

    // 2) SetRobotCount 커맨드를 보낸다.
    write
        .send(Message::Text(r#"{"type":"SetRobotCount","count":2}"#.to_string()))
        .await
        .unwrap();

    // 3) 다음 틱 브로드캐스트(델타)에서 로봇 2대가 등장하는지 확인한다.
    //    틱 주기가 50ms이므로 몇 번의 메시지 안에는 반영되어야 한다.
    //    실제 JSON을 파싱해 `changed_robots` 배열 길이를 확인한다(부분
    //    문자열 매칭보다 정확하고, `removed_robot_ids` 같은 다른 키의
    //    이름에 우연히 걸릴 여지가 없다). 전체 폴링 루프를 데드라인으로
    //    감싸서, 델타 브로드캐스트가 회귀해도 무한정 멈춰있는 대신
    //    빠르게 실패하도록 한다.
    let saw_two_robots = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let Some(Ok(Message::Text(text))) = read.next().await else { return false };
            let Ok(json) = serde_json::from_str::<Value>(&text) else { continue };
            if json["kind"] == "Delta" {
                if let Some(changed) = json["changed_robots"].as_array() {
                    if changed.len() >= 2 {
                        return true;
                    }
                }
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(saw_two_robots, "expected a delta message reflecting 2 robots after SetRobotCount");
}

#[tokio::test]
async fn invalid_command_does_not_crash_the_connection() {
    let server = spawn_server();

    let url = format!("ws://127.0.0.1:{}/ws", server.port);
    let (ws_stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("failed to connect to ws endpoint");
    let (mut write, mut read) = ws_stream.split();

    let _first = read.next().await.expect("stream ended early");

    write.send(Message::Text("not valid json".to_string())).await.unwrap();
    write
        .send(Message::Text(r#"{"type":"ToggleConveyor"}"#.to_string()))
        .await
        .unwrap();

    // 잘못된 메시지 뒤에도 연결이 살아서 다음 델타를 계속 받는지 확인한다.
    let mut still_connected = false;
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_millis(200), read.next()).await {
            Ok(Some(Ok(_))) => {
                still_connected = true;
                break;
            }
            _ => continue,
        }
    }
    assert!(still_connected, "connection should survive an invalid command");
}

#[tokio::test]
async fn resume_with_a_valid_recent_session_id_is_acknowledged() {
    let server = spawn_server();
    let url = format!("ws://127.0.0.1:{}/ws", server.port);

    // 첫 연결: 세션 ID를 받아서 끊는다.
    let session_id = {
        let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await.expect("connect failed");
        let (_write, mut read) = ws_stream.split();
        let first = read.next().await.expect("stream ended early").expect("ws error");
        let Message::Text(text) = first else { panic!("expected text message") };
        let json: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["kind"], "Snapshot");
        json["session_id"].as_str().expect("snapshot should carry a session_id").to_string()
        // 여기서 ws_stream이 drop되며 연결이 끊긴다.
    };

    // 새 연결에서 방금 받은 session_id로 Resume을 보낸다 — 유예시간(30초)
    // 이내이므로 resumed:true를 받아야 한다.
    let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await.expect("connect failed");
    let (mut write, mut read) = ws_stream.split();
    let _snapshot = read.next().await.expect("stream ended early");

    write
        .send(Message::Text(format!(r#"{{"type":"Resume","session_id":"{session_id}"}}"#)))
        .await
        .unwrap();

    let mut saw_ack = false;
    for _ in 0..10 {
        let Some(Ok(Message::Text(text))) = read.next().await else { break };
        let json: Value = serde_json::from_str(&text).unwrap();
        if json["kind"] == "ResumeAck" {
            assert_eq!(json["resumed"], true);
            saw_ack = true;
            break;
        }
    }
    assert!(saw_ack, "expected a ResumeAck message");
}

#[tokio::test]
async fn resume_with_an_unknown_session_id_is_not_acknowledged() {
    let server = spawn_server();
    let url = format!("ws://127.0.0.1:{}/ws", server.port);

    let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await.expect("connect failed");
    let (mut write, mut read) = ws_stream.split();
    let _snapshot = read.next().await.expect("stream ended early");

    let bogus_id = uuid::Uuid::new_v4();
    write
        .send(Message::Text(format!(r#"{{"type":"Resume","session_id":"{bogus_id}"}}"#)))
        .await
        .unwrap();

    let mut saw_ack = false;
    for _ in 0..10 {
        let Some(Ok(Message::Text(text))) = read.next().await else { break };
        let json: Value = serde_json::from_str(&text).unwrap();
        if json["kind"] == "ResumeAck" {
            assert_eq!(json["resumed"], false);
            saw_ack = true;
            break;
        }
    }
    assert!(saw_ack, "expected a ResumeAck message with resumed:false");
}

// NOTE: an earlier version of this file had a black-box
// `lagged_client_resyncs_instead_of_disconnecting` test here that simply
// stopped reading the client socket for 3+ seconds and then asserted the
// connection was still alive. Code review (mutation testing: replacing the
// `Lagged` arm in `handle_socket` with a bare `break`) proved that test
// vacuous — it passed identically whether or not the resync behavior
// existed. Root cause: `tokio::sync::broadcast`'s buffer only overflows
// when the *server's* receiving task falls behind, which happens only if
// `socket.send()` blocks on a full OS send buffer. A client merely pausing
// its application-level reads doesn't do that — the OS keeps acking and
// buffering at the TCP layer far beyond what a few seconds of 20Hz deltas
// (~100-150 bytes each) can fill, so `Lagged` never actually fired in that
// test, in either the intact or the broken build.
//
// The real, deterministic coverage for the lag/resync path now lives as
// unit tests next to the code in `server/src/ws.rs`
// (`decide_broadcast_update`'s test module), which drives a real
// `tokio::sync::broadcast` channel past its capacity directly — no OS
// socket or wall-clock guessing involved.
