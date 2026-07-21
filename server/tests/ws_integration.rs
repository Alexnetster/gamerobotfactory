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

#[tokio::test]
async fn repair_robot_on_a_healthy_robot_is_rejected_without_crashing_the_connection() {
    // 실제로 로봇을 고장내서 성공 경로까지 테스트하지는 않는다 — 자연
    // 마모(2000틱=100초)+확률적 지연을 기다리는 건 느리고 취약한 테스트가
    // 된다(설계문서/Task 8의 교훈). 여기서는 거부 경로가 연결을 죽이지
    // 않는지만 실제 서버로 확인하고, 성공 경로는 game_state.rs의 결정적
    // 단위테스트가 이미 검증한다.
    //
    // 이 테스트 하나만으로는 "RepairRobot이 실제로 파싱돼서 거부됐다"와
    // "JSON이 애초에 RepairRobot으로 파싱 안 됐다"를 구분하지 못한다(둘 다
    // 연결이 안 죽는 건 똑같으므로) — 그 구분은 protocol.rs의
    // `repair_robot_command_round_trips_through_json`(파싱 자체를 증명)과
    // game_state.rs의 `repair_robot_rejects_a_non_failed_robot`(실제 거부
    // 사유를 증명)이 대신 담당한다. 이 테스트는 그 위에 "연결이 안 죽는다"만
    // 얹는 것이다.
    let server = spawn_server();

    let url = format!("ws://127.0.0.1:{}/ws", server.port);
    let (ws_stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("failed to connect to ws endpoint");
    let (mut write, mut read) = ws_stream.split();

    let _first = read.next().await.expect("stream ended early");

    write.send(Message::Text(r#"{"type":"SetRobotCount","count":1}"#.to_string())).await.unwrap();
    write.send(Message::Text(r#"{"type":"RepairRobot","robot_id":0}"#.to_string())).await.unwrap();
    write.send(Message::Text(r#"{"type":"ToggleConveyor"}"#.to_string())).await.unwrap();

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
    assert!(still_connected, "connection should survive a RepairRobot command rejected for a non-failed robot");
}

#[tokio::test]
async fn carrying_flag_flows_over_the_wire_during_a_work_cycle() {
    // 컨베이어는 기본적으로 running:true이므로, 로봇이 하나라도 생기면
    // 작업 사이클(Picking -> carrying:true -> Placing -> carrying:false)이
    // 별도 커맨드 없이 자동으로 시작된다. 델타 압축은 RobotView 전체를
    // PartialEq로 비교해 "안 바뀌면 안 보낸다"는 식으로 동작하므로, 이
    // 테스트는 carrying 필드가 실제로 (a) 와이어에 실리고 (b) 작업 사이클
    // 도중 true로 뒤집히는 것을 실제 서버 프로세스 + 실제 WS 클라이언트로
    // 확인한다(구조체 리터럴/단위테스트만으로는 델타 압축 배선 문제를
    // 못 잡는다).
    let server = spawn_server();

    let url = format!("ws://127.0.0.1:{}/ws", server.port);
    let (ws_stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("failed to connect to ws endpoint");
    let (mut write, mut read) = ws_stream.split();

    let _first = read.next().await.expect("stream ended early");

    write.send(Message::Text(r#"{"type":"SetRobotCount","count":1}"#.to_string())).await.unwrap();

    // Snapshot과 Delta 양쪽 모두에서 로봇 배열을 찾아, 그 중 하나라도
    // carrying:true를 가진 로봇을 보고하면 성공으로 친다. PICK_TICKS는
    // 20틱(20Hz 기준 약 1초)이지만 로봇이 픽업 지점까지 이동하는 시간이
    // 먼저 필요하므로 데드라인을 넉넉히 8초로 잡는다.
    let saw_carrying_true = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let Some(Ok(Message::Text(text))) = read.next().await else { return false };
            let Ok(json) = serde_json::from_str::<Value>(&text) else { continue };
            let robots = match json["kind"].as_str() {
                Some("Delta") => json["changed_robots"].as_array(),
                Some("Snapshot") => json["robots"].as_array(),
                _ => None,
            };
            if let Some(robots) = robots {
                if robots.iter().any(|r| r["carrying"] == Value::Bool(true)) {
                    return true;
                }
            }
        }
    })
    .await
    .unwrap_or(false);

    assert!(saw_carrying_true, "expected at least one message to report carrying:true for the robot during its work cycle");
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
