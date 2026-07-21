use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;

struct ServerProcess {
    child: Child,
    port: u16,
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn spawn_server_with_isolated_db(db_path: &std::path::Path) -> ServerProcess {
    let mut child = Command::new(env!("CARGO_BIN_EXE_server"))
        .env("GAMEROBOTFACTORY_DB_PATH", db_path)
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

fn temp_db_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("gamerobotfactory-test-{name}-{}.sqlite3", uuid::Uuid::new_v4()))
}

#[tokio::test]
async fn config_get_returns_default_then_reflects_post() {
    let db_path = temp_db_path("config");
    let server = spawn_server_with_isolated_db(&db_path);
    let base = format!("http://127.0.0.1:{}", server.port);

    let client = reqwest::Client::new();

    let default_config: serde_json::Value = client
        .get(format!("{base}/api/config"))
        .send()
        .await
        .expect("GET /api/config failed")
        .json()
        .await
        .expect("response was not valid JSON");
    assert_eq!(default_config["persist_every_n_ticks"], 20);

    let updated: serde_json::Value = client
        .post(format!("{base}/api/config"))
        .json(&serde_json::json!({ "persist_every_n_ticks": 1 }))
        .send()
        .await
        .expect("POST /api/config failed")
        .json()
        .await
        .expect("response was not valid JSON");
    assert_eq!(updated["persist_every_n_ticks"], 1);

    let confirmed: serde_json::Value = client
        .get(format!("{base}/api/config"))
        .send()
        .await
        .expect("GET /api/config failed")
        .json()
        .await
        .expect("response was not valid JSON");
    assert_eq!(confirmed["persist_every_n_ticks"], 1);

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn config_post_rejects_zero_interval() {
    let db_path = temp_db_path("config-reject");
    let server = spawn_server_with_isolated_db(&db_path);
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{base}/api/config"))
        .json(&serde_json::json!({ "persist_every_n_ticks": 0 }))
        .send()
        .await
        .expect("POST /api/config failed");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn stats_history_reflects_persisted_rows_after_running() {
    let db_path = temp_db_path("stats");
    let server = spawn_server_with_isolated_db(&db_path);
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();

    // 매 틱 적재하도록 설정을 낮춰서 대기 시간을 줄인다.
    client
        .post(format!("{base}/api/config"))
        .json(&serde_json::json!({ "persist_every_n_ticks": 1 }))
        .send()
        .await
        .expect("POST /api/config failed");

    tokio::time::sleep(Duration::from_millis(500)).await;

    let history: Vec<serde_json::Value> = client
        .get(format!("{base}/api/stats/history"))
        .send()
        .await
        .expect("GET /api/stats/history failed")
        .json()
        .await
        .expect("response was not valid JSON");
    assert!(!history.is_empty(), "expected at least one persisted stats row after running");

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn production_only_increases_after_a_robot_completes_a_full_work_cycle() {
    let db_path = temp_db_path("production-cycle");
    let server = spawn_server_with_isolated_db(&db_path);
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();

    client
        .post(format!("{base}/api/config"))
        .json(&serde_json::json!({ "persist_every_n_ticks": 1 }))
        .send()
        .await
        .expect("POST /api/config failed");

    // ToggleConveyor 없이도 서버 기본값(Conveyor::new()의 running: true)이
    // 이미 켜져 있으므로 로봇 수만 늘리면 곧바로 작업 사이클이 시작된다.
    // 픽업 지점까지의 이동 + PICK_TICKS(20) + 배치 지점까지의 이동 +
    // PLACE_TICKS(20)를 감안해 넉넉히 6초 기다린다.
    let (ws_stream, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}/ws", server.port))
        .await
        .expect("failed to connect WS");
    use futures_util::{SinkExt, StreamExt};
    let (mut write, _read) = ws_stream.split();
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({ "type": "SetRobotCount", "count": 3 }).to_string(),
        ))
        .await
        .expect("failed to send SetRobotCount");

    tokio::time::sleep(Duration::from_secs(6)).await;

    let history: Vec<serde_json::Value> = client
        .get(format!("{base}/api/stats/history"))
        .send()
        .await
        .expect("GET /api/stats/history failed")
        .json()
        .await
        .expect("response was not valid JSON");

    let total_production_ever: f64 = history.iter().filter_map(|row| row["total_production"].as_f64()).sum();
    assert!(total_production_ever > 0.0, "expected at least one robot to complete a full pick/place cycle within 6 seconds, got rows: {history:?}");

    // A full pick+place cycle takes at least PICK_TICKS + PLACE_TICKS ticks
    // (ignoring travel time, which only makes cycles slower/fewer). At 20Hz,
    // 6 seconds is at most 120 ticks, so no robot can complete more than
    // 120 / (PICK_TICKS + PLACE_TICKS) full cycles in this window even in
    // the best case (already standing on the pickup/place point every time).
    // +1 cycle of slack per robot absorbs legitimate timing jitter (test
    // scheduling delays, GET latency) without absorbing an order-of-magnitude
    // regression like crediting every robot on every tick.
    const ROBOT_COUNT: u32 = 3;
    const TEST_WINDOW_TICKS: u32 = 20 * 6; // 20Hz tick rate, 6s sleep
    let max_cycles_per_robot = TEST_WINDOW_TICKS / (sim_core::sim::PICK_TICKS + sim_core::sim::PLACE_TICKS) + 1;
    let max_plausible_production =
        ROBOT_COUNT as f64 * max_cycles_per_robot as f64 * sim_core::sim::UNIT_PER_CYCLE as f64;

    assert!(
        total_production_ever <= max_plausible_production,
        "production grew far faster than a real pick/place cycle allows ({total_production_ever} > {max_plausible_production}) — likely crediting robots that haven't actually completed a placement, got rows: {history:?}"
    );

    let _ = std::fs::remove_file(&db_path);
}

/// Prometheus 텍스트 포맷에서 `metric_name value` 형태의 한 줄을 찾아 값을
/// `i64`로 파싱한다. 단순히 이름이 텍스트에 등장하는지만 보면 카운터가
/// 실제로 증가했는지는 검증하지 못한다 — 레지스트리에 등록만 되어 있으면
/// 틱이 한 번도 안 돌아도 이름은 항상 출력에 나타나기 때문이다
/// (`metrics.rs`의 `fresh_metrics_encode_without_error_and_include_registered_names`
/// 테스트가 이미 그 사실을 증명한다). 그래서 이 헬퍼로 실제 숫자 값을 뽑아
/// `> 0`을 단언해야 틱 루프가 이 카운터를 실제로 증가시키고 있다는 것까지
/// 검증한 게 된다.
fn parse_metric_value(body: &str, metric_name: &str) -> i64 {
    let prefix = format!("{metric_name} ");
    body.lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("metric line for {metric_name} not found in body:\n{body}"))
        .split_whitespace()
        .nth(1)
        .unwrap_or_else(|| panic!("metric line for {metric_name} had no value token"))
        .parse()
        .unwrap_or_else(|err| panic!("metric value for {metric_name} was not an integer: {err}"))
}

#[tokio::test]
async fn metrics_endpoint_exposes_prometheus_text_with_tick_counter() {
    let db_path = temp_db_path("metrics");
    let server = spawn_server_with_isolated_db(&db_path);
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();

    // 20Hz(50ms/tick) 기준 400ms면 ~8틱 — 200ms(~4틱)보다 여유를 둬서
    // 스케줄링 지연으로 인한 flake 가능성을 줄인다.
    tokio::time::sleep(Duration::from_millis(400)).await;

    let response = client.get(format!("{base}/metrics")).send().await.expect("GET /metrics failed");
    assert!(response.status().is_success());
    let body = response.text().await.expect("failed to read metrics body");

    // robot_count는 값 자체가 아니라 지표가 노출되는지만 확인한다 — 로봇 수는
    // 0이 정상값일 수 있는 게이지라 값 검증 대상이 아니다.
    assert!(body.contains("gamerobotfactory_robot_count"));

    // ticks_total은 실제로 값이 0보다 큰지까지 확인해서, 틱 루프가 이 카운터를
    // 정말로 증가시키고 있다는 것을 검증한다(이름만 등장하는지 보는 것보다
    // 강한 단언).
    let ticks_total = parse_metric_value(&body, "gamerobotfactory_ticks_total");
    assert!(ticks_total > 0, "expected gamerobotfactory_ticks_total to have advanced past 0, got {ticks_total}");

    // tick_duration_seconds의 `_count` 서픽스가 0보다 큰지까지 확인해서, 이
    // 히스토그램이 틱 루프 안에서 실제로 `.observe(...)`되고 있다는 것을
    // 검증한다 — 등록만 되어 있으면(관측이 한 번도 없어도) `_count 0`이라는
    // 줄 자체는 항상 출력되므로, 이름 존재 여부만으로는 배선을 증명하지
    // 못한다(ticks_total에 대한 위 주석과 같은 이유).
    let tick_duration_count = parse_metric_value(&body, "gamerobotfactory_tick_duration_seconds_count");
    assert!(
        tick_duration_count > 0,
        "expected gamerobotfactory_tick_duration_seconds_count to have advanced past 0, got {tick_duration_count}"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn robot_failures_endpoint_returns_an_empty_list_when_nothing_has_failed() {
    // Note: this server starts with zero robots (nothing calls
    // SetRobotCount), so "no failure occurred" is trivially guaranteed by
    // there being no robots at all, not by a genuinely short observation
    // window. This test still earns its keep as the empty-list-default half
    // of the REST contract; the genuine persist -> read round trip is
    // covered by `robot_failures_endpoint_returns_previously_persisted_failure_events`
    // below, which pre-seeds a real row and reads it back through the actual
    // REST/JSON layer.
    let db_path = temp_db_path("robot-failures");
    let server = spawn_server_with_isolated_db(&db_path);
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();

    let history: Vec<serde_json::Value> = client
        .get(format!("{base}/api/robots/failures"))
        .send()
        .await
        .expect("GET /api/robots/failures failed")
        .json()
        .await
        .expect("response was not valid JSON");
    assert!(history.is_empty(), "no robot should have failed in a fresh, brief-lived server");

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn robot_failures_endpoint_returns_previously_persisted_failure_events() {
    // Waiting for a *real* failure to occur organically (natural wear takes
    // WEAR_LIMIT_TICKS at 20Hz, plus a probabilistic delay on top) is
    // exactly the flaky/slow pattern this project avoids. Instead, pre-seed
    // the isolated SQLite file directly with a known row before the server
    // (and its tick loop) ever starts, then prove the real REST-read path —
    // `persistence::recent_failure_events` -> serde_json -> axum::Json` —
    // actually returns it.
    //
    // We can't call `persistence::open_db`/`insert_failure_event` directly
    // from this integration test binary: `persistence` is a module private
    // to the `server` binary crate, not part of the `sim_core` lib target
    // that integration tests link against. So this replicates the exact
    // schema and insert statement from `server/src/persistence.rs` via
    // `rusqlite` directly (a normal, non-dev dependency of this package).
    let db_path = temp_db_path("robot-failures-seeded");
    {
        let conn = rusqlite::Connection::open(&db_path).expect("failed to open db for seeding");
        conn.execute(
            "CREATE TABLE IF NOT EXISTS robot_failure_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tick INTEGER NOT NULL,
                robot_id INTEGER NOT NULL,
                event_type TEXT NOT NULL
            )",
            [],
        )
        .expect("failed to create robot_failure_events table while seeding");
        conn.execute(
            "INSERT INTO robot_failure_events (tick, robot_id, event_type) VALUES (?1, ?2, ?3)",
            rusqlite::params![42i64, 7i64, "failed"],
        )
        .expect("failed to seed a robot_failure_events row");
        // conn drops here, before the server (and its own open_db/init_schema,
        // which is CREATE TABLE IF NOT EXISTS and so won't wipe this row) starts.
    }

    let server = spawn_server_with_isolated_db(&db_path);
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();

    let history: Vec<serde_json::Value> = client
        .get(format!("{base}/api/robots/failures"))
        .send()
        .await
        .expect("GET /api/robots/failures failed")
        .json()
        .await
        .expect("response was not valid JSON");

    assert_eq!(history.len(), 1, "expected exactly the one pre-seeded failure event, got {history:?}");
    assert_eq!(history[0]["tick"], 42);
    assert_eq!(history[0]["robot_id"], 7);
    assert_eq!(history[0]["event_type"], "failed");

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn metrics_endpoint_exposes_robot_failure_gauges_at_their_baseline() {
    let db_path = temp_db_path("robot-failure-metrics");
    let server = spawn_server_with_isolated_db(&db_path);
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let response = client.get(format!("{base}/metrics")).send().await.expect("GET /metrics failed");
    let body = response.text().await.expect("failed to read metrics body");

    // 실제로 고장이 발생하는 걸 기다리는 건(자연 마모로 WEAR_LIMIT_TICKS +
    // 확률적 지연) 이 테스트를 느리고 취약하게 만든다 — 대신 두 지표가
    // 노출되고 있고, 짧은 실행 동안 고장이 없었다는 정상적인 기저값(0)을
    // 보이는지만 확인한다. 값이 실제로 바뀌는 로직(detect_status_transitions)
    // 자체는 main.rs의 결정적 단위테스트가 이미 검증한다.
    assert!(body.contains("gamerobotfactory_robot_failures_total 0"));
    assert!(body.contains("gamerobotfactory_robots_repairing 0"));

    let _ = std::fs::remove_file(&db_path);
}
