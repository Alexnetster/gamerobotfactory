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
async fn metrics_endpoint_exposes_prometheus_text_with_tick_counter() {
    let db_path = temp_db_path("metrics");
    let server = spawn_server_with_isolated_db(&db_path);
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let response = client.get(format!("{base}/metrics")).send().await.expect("GET /metrics failed");
    assert!(response.status().is_success());
    let body = response.text().await.expect("failed to read metrics body");
    assert!(body.contains("gamerobotfactory_ticks_total"));
    assert!(body.contains("gamerobotfactory_robot_count"));

    let _ = std::fs::remove_file(&db_path);
}
