use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};

struct ServerProcess {
    child: Child,
    port: u16,
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn spawn_server_with_static_dir(db_path: &std::path::Path, static_dir: &std::path::Path) -> ServerProcess {
    let mut child = Command::new(env!("CARGO_BIN_EXE_server"))
        .env("GAMEROBOTFACTORY_DB_PATH", db_path)
        .env("GAMEROBOTFACTORY_STATIC_DIR", static_dir)
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

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("gamerobotfactory-static-test-{name}-{}", uuid::Uuid::new_v4()))
}

#[tokio::test]
async fn serves_static_files_and_still_answers_health() {
    let db_path = temp_path("db").with_extension("sqlite3");
    let static_dir = temp_path("dist");
    std::fs::create_dir_all(&static_dir).expect("failed to create temp static dir");
    std::fs::write(static_dir.join("index.html"), "<html>hello from static test</html>")
        .expect("failed to write temp index.html");

    let server = spawn_server_with_static_dir(&db_path, &static_dir);
    let base = format!("http://127.0.0.1:{}", server.port);

    let index_body = reqwest::get(format!("{base}/")).await.unwrap().text().await.unwrap();
    assert!(index_body.contains("hello from static test"));

    let health_status = reqwest::get(format!("{base}/health")).await.unwrap().status();
    assert!(health_status.is_success());

    let _ = std::fs::remove_dir_all(&static_dir);
    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn missing_static_dir_returns_404_without_crashing_other_routes() {
    // static_dir 자체가 없어도(로컬 cargo run에서 client/dist를 안 만든
    // 경우와 동일한 상황) 서버가 안 죽고 기존 API는 정상 동작해야 한다.
    let db_path = temp_path("db-missing").with_extension("sqlite3");
    let nonexistent_static_dir = temp_path("does-not-exist");

    let server = spawn_server_with_static_dir(&db_path, &nonexistent_static_dir);
    let base = format!("http://127.0.0.1:{}", server.port);

    let root_status = reqwest::get(format!("{base}/")).await.unwrap().status();
    assert_eq!(root_status.as_u16(), 404);

    let health_status = reqwest::get(format!("{base}/health")).await.unwrap().status();
    assert!(health_status.is_success());

    let _ = std::fs::remove_file(&db_path);
}
