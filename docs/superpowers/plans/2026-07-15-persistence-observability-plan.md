# 로봇팔 컨베이어 게임 — Plan 3: 영속화 + REST API + 관측가능성 + 하드닝 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **워크트리 없이 `main`에서 직접 작업한다** — 이 저장소는 소유자 혼자 쓰는 프로젝트라 워크트리/피처 브랜치 분리를 생략한다.

**Goal:** Plan 2가 끝난 뒤 KANBAN.md에 명시적으로 남겨둔 세 가지 하드닝 갭(재접속 실배선, Lagged 브로드캐스트 처리, 틱 루프 패닉 방어)을 먼저 닫고, 그 위에 SQLite 영속화·REST API·tracing 구조화 로깅·Prometheus `/metrics` 엔드포인트를 추가한다.

**Architecture:** 하드닝 3개 태스크는 기존 Plan 2 코드(`ws.rs`, `main.rs`)를 손보는 것이라 새 의존성이 없다. 영속화/REST/관측가능성은 각각 독립 모듈(`persistence.rs`, `config.rs`, `metrics.rs`)로 분리하고, `sim_core`(Plan 1)는 이번에도 전혀 건드리지 않는다. SQLite 쓰기는 동기(rusqlite)라 `tokio::task::spawn_blocking`으로 감싸 비동기 틱 루프를 막지 않는다. REST 핸들러는 기존 WS 핸들러가 쓰던 `State`+`Extension` 혼합 패턴을 그대로 따른다.

**Tech Stack:** `rusqlite`(bundled feature, 시스템 sqlite3 불필요), `tracing`+`tracing-subscriber`, `prometheus`. 기존 axum/tokio/serde/uuid 의존성은 그대로.

**축 관련 주의사항:** 이전 두 Plan과 마찬가지로 컴파일러 피드백 없이 작성됐다 — 실제 크레이트 API 시그니처가 미묘하게 다르면(특히 `prometheus`/`rusqlite`의 매크로/트레잇 바운드), 동작 사양(각 스텝의 테스트가 무엇을 검증해야 하는지)을 기준으로 조정한다.

**설계 문서 참조:** `docs/robot-arm-conveyor-game-design.md`의 "영속화 & API", "관찰가능성" 절. `docs/KANBAN.md`의 "Plan 2 마무리 후 명시적으로 남기는 갭" 절.

---

## 파일 구조

| 파일 | 책임 |
|---|---|
| `server/Cargo.toml` | `rusqlite`, `tracing`, `tracing-subscriber`, `prometheus` 추가. |
| `server/src/ws.rs` | (수정) 세션 재접속 실배선, Lagged 브로드캐스트 리싱크. |
| `server/src/protocol.rs` | (수정) `ClientCommand::Resume`, `ServerMessage::Snapshot.session_id`/`ServerMessage::ResumeAck`, `uuid` serde 지원. |
| `server/src/main.rs` | (수정) `safe_tick` 패닉 방어, tracing 초기화, DB/설정/메트릭 상태 배선 및 라우트 추가. |
| `server/src/persistence.rs` | SQLite 스키마 + 통계 행 삽입/조회. |
| `server/src/config.rs` | 런타임 설정(`AppConfig`) + GET/POST `/api/config`. |
| `server/src/metrics.rs` | Prometheus 레지스트리 + `/metrics` 핸들러. |
| `server/tests/ws_integration.rs` | (수정) Lagged 리싱크 통합테스트 추가. |
| `server/tests/rest_integration.rs` | REST(`/api/config`, `/api/stats/history`, `/metrics`) 통합테스트. |

---

### Task 1: 세션 재접속 실배선

`session.rs`(Plan 2 Task 9)의 순수 로직을 실제 WS 핸들러에 연결한다. 매 연결마다 새 세션을 발급해 최초 스냅샷에 실어 보내고, 클라이언트가 `Resume{session_id}`를 보내면 유예시간 내인지 확인해 응답한다.

**Files:**
- Modify: `server/Cargo.toml` (uuid에 `serde` 피처 추가)
- Modify: `server/src/protocol.rs`
- Modify: `server/src/ws.rs`
- Modify: `server/src/main.rs`

- [ ] **Step 1: `uuid`에 serde 피처 추가**

`server/Cargo.toml`의 `uuid` 줄을 교체:
```toml
uuid = { version = "1", features = ["v4", "serde"] }
```

- [ ] **Step 2: 프로토콜에 세션 메시지 추가**

`server/src/protocol.rs`의 `ClientCommand` enum에 변형 추가:
```rust
pub enum ClientCommand {
    SelectRobot { robot_id: u32 },
    ReleaseRobot,
    ToggleConveyor,
    SetRobotCount { count: usize },
    TriggerArmAction { robot_id: u32, task: WireTask },
    Resume { session_id: uuid::Uuid },
}
```

`ServerMessage`의 `Snapshot` 변형에 필드 추가, 새 변형 하나 추가:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum ServerMessage {
    Snapshot { v: u8, tick: u64, session_id: uuid::Uuid, conveyor: ConveyorView, robots: Vec<RobotView> },
    Delta { v: u8, tick: u64, conveyor: Option<ConveyorView>, changed_robots: Vec<RobotView>, removed_robot_ids: Vec<u32> },
    ResumeAck { v: u8, session_id: uuid::Uuid, resumed: bool },
}
```

`to_snapshot`의 시그니처를 세션 ID를 받도록 바꾼다:
```rust
pub fn to_snapshot(state: &GameState, session_id: uuid::Uuid) -> ServerMessage {
    ServerMessage::Snapshot {
        v: PROTOCOL_VERSION,
        tick: state.sim.tick_count,
        session_id,
        conveyor: state.conveyor.into(),
        robots: state.sim.robots.iter().map(RobotView::from).collect(),
    }
}
```

`protocol.rs`의 기존 테스트 3개를 각각 이렇게 고친다:

- `server_message_round_trips_through_json`: `ServerMessage::Snapshot { ... }` 구조체 리터럴에 `session_id: uuid::Uuid::nil(),` 필드를 추가한다 (`v`/`tick`/`conveyor`/`robots`는 그대로 유지):
  ```rust
  let msg = ServerMessage::Snapshot {
      v: 1,
      tick: 42,
      session_id: uuid::Uuid::nil(),
      conveyor: ConveyorView { running: true },
      robots: vec![],
  };
  ```
- `to_snapshot_reflects_current_game_state`: `to_snapshot(&state)` 호출을 `to_snapshot(&state, uuid::Uuid::nil())`로 바꾸고, 그 결과를 매치하는 패턴 `ServerMessage::Snapshot { v, tick, conveyor, robots }`(필드 4개 나열)을 `ServerMessage::Snapshot { v, tick, conveyor, robots, .. }`(끝에 `..` 추가, 5번째 필드인 `session_id`는 이 테스트가 검증하는 대상이 아니므로 무시)로 바꾼다.
- `client_command_deserializes_from_tagged_json`/`client_command_round_trips_through_json`: `ClientCommand`에 새 변형(`Resume`)이 추가됐을 뿐 기존 변형은 안 바뀌었으므로 손댈 필요 없다.

- [ ] **Step 3: `main.rs`의 `to_snapshot` 호출부 수정**

`spawn_tick_loop`의 두 `to_snapshot(&guard)` 호출을 `to_snapshot(&guard, uuid::Uuid::nil())`로 바꾼다 — 틱 루프가 내부적으로 스냅샷을 들고 있는 건 델타 계산용 스크래치일 뿐, 실제로 그 값 자체가 그대로 전선을 타고 나가는 일은 없으므로(항상 `compute_delta`를 거쳐 `Delta`로 나간다) 세션 ID는 의미가 없어 nil을 쓴다.

- [ ] **Step 4: `ws.rs`에 세션 핸들 타입 + 발급/확인 로직 추가**

`server/src/ws.rs`에서 `SharedState`/`Broadcaster` 정의 아래에 추가:
```rust
pub type SessionHandle = Arc<Mutex<crate::session::SessionRegistry>>;
```

`ws_route`에 세 번째 익스트랙터 추가:
```rust
pub async fn ws_route(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    axum::extract::Extension(broadcaster): axum::extract::Extension<Broadcaster>,
    axum::extract::Extension(sessions): axum::extract::Extension<SessionHandle>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, broadcaster, sessions))
}
```

`handle_socket`을 아래로 교체 — 연결마다 새 세션을 발급해 스냅샷에 싣고, `Resume` 커맨드를 `apply_command`로 넘기기 전에 가로채 처리한다:
```rust
async fn handle_socket(mut socket: WebSocket, state: SharedState, broadcaster: Broadcaster, sessions: SessionHandle) {
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
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("client lagged behind by {n} messages; resyncing with a full snapshot");
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
```

(이 Step에서 Lagged 처리도 함께 들어간다 — Task 2에서 다시 다루지 않고 여기서 끝낸다. `apply_command`/`send_message` 함수 본문은 그대로 둔다.)

- [ ] **Step 5: `main.rs`에 세션 레지스트리 상태 배선**

`server/src/main.rs`의 `main()` 함수에서 `broadcaster` 채널을 만든 직후에 추가:
```rust
    let sessions: ws::SessionHandle = Arc::new(Mutex::new(session::SessionRegistry::new()));
```

`build_app`의 시그니처와 본문에 세션 핸들을 추가로 받아 레이어에 얹는다:
```rust
pub fn build_app(state: SharedState, broadcaster: Broadcaster, sessions: ws::SessionHandle) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_route))
        .with_state(state)
        .layer(axum::extract::Extension(broadcaster))
        .layer(axum::extract::Extension(sessions))
}
```

`main()`에서 `build_app(state, broadcaster)` 호출부를 `build_app(state, broadcaster, sessions)`로 바꾼다.

- [ ] **Step 6: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 전부 PASS (protocol.rs의 기존 테스트 2개를 수정했을 뿐 새 테스트는 추가하지 않았다 — 실배선 자체의 검증은 Task 10의 통합테스트에서 한다). 정확한 개수는 관찰해서 보고한다.

- [ ] **Step 7: Commit**

```bash
git add server/Cargo.toml server/src/protocol.rs server/src/ws.rs server/src/main.rs
git commit -m "feat: wire session registry into ws handler with reconnect resume and lag resync"
```

---

### Task 2: 틱 루프 패닉 방어 (`safe_tick`)

`sim_core::sim::tick`이 (아직 알려지지 않은 어떤 이유로든) 패닉해도 서버 틱 루프 전체가 죽지 않도록 감싼다. Plan 1의 `safe_call`과 같은 이유로, `sim_core` 자체에 결함 주입 지점을 다시 여는 대신 이 서버 레벨 래퍼가 실제로 격리 역할을 한다는 것만 검증한다.

**Files:**
- Modify: `server/src/main.rs`

- [ ] **Step 1: `safe_tick` 함수 + 테스트 모듈 추가**

`server/src/main.rs`의 `spawn_tick_loop` 함수 위에 추가:
```rust
/// `sim_core::sim::tick`을 패닉으로부터 격리한다. 패닉이 나면 이번 틱은
/// 건너뛰고(시뮬레이션 상태는 직전 틱 그대로 유지) 서버 프로세스와 다른
/// 연결은 영향받지 않는다.
fn safe_tick(sim: &SimState) -> Option<SimState> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick(sim))) {
        Ok(next) => Some(next),
        Err(_) => {
            eprintln!("tick() panicked; skipping this tick, simulation state unchanged");
            None
        }
    }
}
```

`spawn_tick_loop` 안의 `guard.sim = tick(&guard.sim);` 줄을 아래로 교체:
```rust
                if let Some(next_sim) = safe_tick(&guard.sim) {
                    guard.sim = next_sim;
                }
```

파일 맨 아래에 테스트 모듈 추가 (아직 `main.rs`에 `#[cfg(test)]` 모듈이 없다면 새로 만든다):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_tick_passes_through_normal_ticks_unchanged() {
        let sim = SimState { grid: Arc::new(Grid::new(3, 3)), robots: Vec::new(), tick_count: 5 };
        let result = safe_tick(&sim);
        assert!(result.is_some());
        assert_eq!(result.unwrap().tick_count, 6);
    }

    #[test]
    fn catch_unwind_recovers_from_a_panic_the_same_way_safe_tick_does() {
        // sim_core::sim::tick() 자체를 결정적으로 패닉시키려면 이미 완성되어
        // 포트폴리오 리뷰까지 거친 Plan 1 라이브러리에 결함 주입 지점을 다시
        // 여는 작업이 필요한데, 이는 이 태스크 범위 밖이다. 대신 safe_tick이
        // 실제로 쓰는 것과 동일한 catch_unwind 복구 경로를 직접 검증한다 —
        // Plan 1의 safe_call 테스트, Plan 2의 goal-exception 재검증과 같은
        // 패턴(패턴 자체를 검증 + 정상 경로 테스트로 실제 배선까지 커버).
        let result: std::thread::Result<i32> = std::panic::catch_unwind(|| panic!("simulated tick fault"));
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 이전 스위트 + 신규 2개.

- [ ] **Step 3: Commit**

```bash
git add server/src/main.rs
git commit -m "feat: isolate tick loop from panics with safe_tick"
```

---

### Task 3: SQLite 영속화 (`persistence.rs`)

**Files:**
- Modify: `server/Cargo.toml`
- Create: `server/src/persistence.rs`
- Modify: `server/src/main.rs`

- [ ] **Step 1: 의존성 추가**

`server/Cargo.toml`의 `[dependencies]`에 추가:
```toml
rusqlite = { version = "0.31", features = ["bundled"] }
```

- [ ] **Step 2: 구현 + 테스트 작성**

`server/src/persistence.rs`:
```rust
use rusqlite::{params, Connection, Result};
use serde::Serialize;

pub fn open_db(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)?;
    init_schema(&conn)?;
    Ok(conn)
}

pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS stats_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tick INTEGER NOT NULL,
            robot_count INTEGER NOT NULL,
            conveyor_running INTEGER NOT NULL,
            total_production REAL NOT NULL
        )",
        [],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct StatsRow {
    pub tick: u64,
    pub robot_count: usize,
    pub conveyor_running: bool,
    pub total_production: f32,
}

pub fn insert_stats(conn: &Connection, row: &StatsRow) -> Result<()> {
    conn.execute(
        "INSERT INTO stats_history (tick, robot_count, conveyor_running, total_production) VALUES (?1, ?2, ?3, ?4)",
        params![
            row.tick as i64,
            row.robot_count as i64,
            row.conveyor_running as i64,
            row.total_production as f64
        ],
    )?;
    Ok(())
}

pub fn recent_stats(conn: &Connection, limit: usize) -> Result<Vec<StatsRow>> {
    let mut stmt = conn.prepare(
        "SELECT tick, robot_count, conveyor_running, total_production FROM stats_history ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(StatsRow {
            tick: row.get::<_, i64>(0)? as u64,
            robot_count: row.get::<_, i64>(1)? as usize,
            conveyor_running: row.get::<_, i64>(2)? != 0,
            total_production: row.get::<_, f64>(3)? as f32,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn insert_and_read_back_a_stats_row() {
        let conn = test_db();
        let row = StatsRow { tick: 42, robot_count: 3, conveyor_running: true, total_production: 1.5 };
        insert_stats(&conn, &row).unwrap();

        let rows = recent_stats(&conn, 10).unwrap();
        assert_eq!(rows, vec![row]);
    }

    #[test]
    fn recent_stats_returns_newest_first_and_respects_limit() {
        let conn = test_db();
        for tick in 0..5u64 {
            insert_stats(
                &conn,
                &StatsRow { tick, robot_count: 1, conveyor_running: true, total_production: 0.0 },
            )
            .unwrap();
        }

        let rows = recent_stats(&conn, 2).unwrap();
        assert_eq!(rows.iter().map(|r| r.tick).collect::<Vec<_>>(), vec![4, 3]);
    }

    #[test]
    fn recent_stats_on_empty_db_returns_empty_vec() {
        let conn = test_db();
        let rows = recent_stats(&conn, 10).unwrap();
        assert!(rows.is_empty());
    }
}
```

`server/src/main.rs`에 추가:
```rust
mod persistence;
```

- [ ] **Step 3: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 이전 스위트 + `persistence` 신규 3개.

- [ ] **Step 4: Commit**

```bash
git add server/Cargo.toml server/src/main.rs server/src/persistence.rs
git commit -m "feat: add SQLite persistence for production stats history"
```

---

### Task 4: 런타임 설정 (`config.rs`) + `GET`/`POST /api/config`

**Files:**
- Create: `server/src/config.rs`
- Modify: `server/src/main.rs`

- [ ] **Step 1: 구현 + 테스트 작성**

`server/src/config.rs`:
```rust
use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

/// REST로 조회/변경 가능한 런타임 설정. WS(실시간 게임 상태)와 REST(설정)의
/// 책임을 분리한다는 설계문서의 결정을 실제로 구현하는 지점.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    pub persist_every_n_ticks: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig { persist_every_n_ticks: 20 }
    }
}

pub type ConfigHandle = Arc<Mutex<AppConfig>>;

pub async fn get_config(Extension(config): Extension<ConfigHandle>) -> impl IntoResponse {
    let current = *config.lock().await;
    Json(current)
}

pub async fn post_config(
    Extension(config): Extension<ConfigHandle>,
    Json(update): Json<AppConfig>,
) -> impl IntoResponse {
    if update.persist_every_n_ticks == 0 {
        return (StatusCode::BAD_REQUEST, "persist_every_n_ticks must be at least 1").into_response();
    }
    let mut current = config.lock().await;
    *current = update;
    Json(update).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_a_sane_persist_interval() {
        assert_eq!(AppConfig::default().persist_every_n_ticks, 20);
    }
}
```

`server/src/main.rs`에 추가:
```rust
mod config;
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 이전 스위트 + `config` 신규 1개. (`get_config`/`post_config`는 axum 핸들러라 유닛테스트로 직접 부르기보다 Task 10의 REST 통합테스트에서 실제 HTTP로 검증한다.)

- [ ] **Step 3: Commit**

```bash
git add server/src/main.rs server/src/config.rs
git commit -m "feat: add runtime AppConfig with GET/POST /api/config"
```

---

### Task 5: Prometheus `/metrics` 엔드포인트 (`metrics.rs`)

**Files:**
- Modify: `server/Cargo.toml`
- Create: `server/src/metrics.rs`
- Modify: `server/src/main.rs`

- [ ] **Step 1: 의존성 추가**

`server/Cargo.toml`의 `[dependencies]`에 추가:
```toml
prometheus = "0.13"
```

- [ ] **Step 2: 구현 + 테스트 작성**

`server/src/metrics.rs`:
```rust
use axum::extract::Extension;
use axum::response::IntoResponse;
use prometheus::{
    register_int_counter_with_registry, register_int_gauge_with_registry, Encoder, IntCounter, IntGauge, Registry,
    TextEncoder,
};
use std::sync::Arc;

pub struct Metrics {
    registry: Registry,
    pub ticks_total: IntCounter,
    pub connected_clients: IntGauge,
    pub robot_count: IntGauge,
    /// `safe_tick`(Task 2)이 패닉을 잡아낸 횟수. 이게 없으면 틱 루프가
    /// 매번 패닉해서 시뮬레이션이 조용히 멈춰도(서버 프로세스 자체는
    /// 살아있으니 `/health`는 여전히 "ok"를 반환) 밖에서 알아챌 방법이
    /// 없다 — Task 2 코드 리뷰에서 지적된 "조용한 멈춤" 관측 공백을
    /// 메우는 지표.
    pub tick_panics_total: IntCounter,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();
        let ticks_total = register_int_counter_with_registry!(
            "gamerobotfactory_ticks_total",
            "Total simulation ticks processed",
            registry
        )
        .expect("metric registration is infallible for a fresh registry");
        let connected_clients = register_int_gauge_with_registry!(
            "gamerobotfactory_connected_clients",
            "Currently connected WebSocket clients",
            registry
        )
        .expect("metric registration is infallible for a fresh registry");
        let robot_count = register_int_gauge_with_registry!(
            "gamerobotfactory_robot_count",
            "Current number of robots in the simulation",
            registry
        )
        .expect("metric registration is infallible for a fresh registry");
        let tick_panics_total = register_int_counter_with_registry!(
            "gamerobotfactory_tick_panics_total",
            "Total number of ticks where sim_core::sim::tick panicked and was skipped",
            registry
        )
        .expect("metric registration is infallible for a fresh registry");

        Metrics { registry, ticks_total, connected_clients, robot_count, tick_panics_total }
    }

    pub fn encode(&self) -> (String, Vec<u8>) {
        let encoder = TextEncoder::new();
        let families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder
            .encode(&families, &mut buffer)
            .expect("encoding a freshly-gathered metric family set does not fail");
        (encoder.format_type().to_string(), buffer)
    }
}

pub type MetricsHandle = Arc<Metrics>;

pub async fn metrics_route(Extension(metrics): Extension<MetricsHandle>) -> impl IntoResponse {
    let (content_type, body) = metrics.encode();
    ([(axum::http::header::CONTENT_TYPE, content_type)], body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_metrics_encode_without_error_and_include_registered_names() {
        let metrics = Metrics::new();
        let (content_type, body) = metrics.encode();
        assert!(content_type.starts_with("text/plain"));
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("gamerobotfactory_ticks_total"));
        assert!(text.contains("gamerobotfactory_connected_clients"));
        assert!(text.contains("gamerobotfactory_robot_count"));
        assert!(text.contains("gamerobotfactory_tick_panics_total"));
    }

    #[test]
    fn incrementing_a_counter_is_reflected_in_the_encoded_output() {
        let metrics = Metrics::new();
        metrics.ticks_total.inc();
        metrics.ticks_total.inc();
        let (_, body) = metrics.encode();
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("gamerobotfactory_ticks_total 2"));
    }
}
```

`server/src/main.rs`에 추가:
```rust
mod metrics;
```

- [ ] **Step 3: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 이전 스위트 + `metrics` 신규 2개.

- [ ] **Step 4: Commit**

```bash
git add server/Cargo.toml server/src/main.rs server/src/metrics.rs
git commit -m "feat: add Prometheus metrics registry and encoder"
```

---

### Task 6: tracing 구조화 로깅

**Files:**
- Modify: `server/Cargo.toml`
- Modify: `server/src/main.rs`
- Modify: `server/src/ws.rs`

- [ ] **Step 1: 의존성 추가**

`server/Cargo.toml`의 `[dependencies]`에 추가:
```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

- [ ] **Step 2: `main()`에서 구독자 초기화**

`server/src/main.rs`의 `#[tokio::main] async fn main()` 맨 앞에 추가:
```rust
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
```

- [ ] **Step 3: 핵심 `eprintln!`을 `tracing`으로 교체**

`server/src/ws.rs`에서 다음 세 곳을 구조화 로깅으로 바꾼다(문자열 형태만 바뀌고 동작은 동일):
```rust
// "invalid client command, ignoring: {err}" 자리
Err(err) => tracing::warn!(error = %err, "invalid client command, ignoring"),
```
```rust
// "websocket error, closing connection: {err}" 자리
Some(Err(err)) => {
    tracing::warn!(error = %err, "websocket error, closing connection");
    break;
}
```
```rust
// "client lagged behind by {n} messages; resyncing with a full snapshot" 자리
Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
    tracing::warn!(missed_messages = n, "client lagged behind; resyncing with a full snapshot");
    ...
}
```

`server/src/main.rs`의 `safe_tick`에서:
```rust
Err(_) => {
    tracing::error!("tick() panicked; skipping this tick, simulation state unchanged");
    None
}
```

`apply_command`의 두 `eprintln!("... rejected: {err:?}")`도 각각 `tracing::warn!(?err, "SelectRobot rejected")` / `tracing::warn!(?err, "TriggerArmAction rejected")`로 바꾼다.

- [ ] **Step 4: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 이전 스위트 그대로 통과(로깅 문구만 바뀌었으므로 개수 변화 없음).

- [ ] **Step 5: 수동 확인**

서버를 `RUST_LOG=info cargo run --manifest-path server/Cargo.toml`로 띄워 구조화된 로그 줄(타임스탬프, 레벨, 필드)이 실제로 찍히는지 확인한다.

- [ ] **Step 6: Commit**

```bash
git add server/Cargo.toml server/src/main.rs server/src/ws.rs
git commit -m "feat: replace ad-hoc eprintln logging with structured tracing"
```

---

### Task 7: 영속화 + 설정 + 메트릭을 `main.rs`에 배선

지금까지 만든 모듈들을 실제로 서버 상태/틱 루프/라우터에 연결한다.

**Files:**
- Modify: `server/src/main.rs`
- Modify: `server/src/ws.rs`

- [ ] **Step 1: 상태 초기화 + 라우트 추가**

`server/src/main.rs`를 아래 요소들을 반영해 갱신한다 (기존 `mod` 선언, `health`, `initial_state`, `TICK_INTERVAL`, `safe_tick`은 그대로 유지):

```rust
use config::{get_config, post_config, AppConfig, ConfigHandle};
use metrics::{metrics_route, Metrics, MetricsHandle};
use std::sync::Mutex as StdMutex;

type DbHandle = Arc<StdMutex<rusqlite::Connection>>;

async fn stats_history(Extension(db): Extension<DbHandle>) -> impl IntoResponse {
    let rows = tokio::task::spawn_blocking(move || {
        let conn = db.lock().unwrap();
        persistence::recent_stats(&conn, 50)
    })
    .await
    .expect("spawn_blocking task panicked");

    match rows {
        Ok(rows) => axum::Json(rows).into_response(),
        Err(err) => {
            tracing::error!(%err, "failed to read stats history");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
        }
    }
}
```

`spawn_tick_loop`의 시그니처에 `db: DbHandle, config: ConfigHandle, metrics: MetricsHandle`를 추가하고, 루프 안에서 매 틱마다 `metrics.ticks_total.inc()` / `metrics.robot_count.set(guard.sim.robots.len() as i64)`를 갱신하고, `guard.sim.tick_count % persist_every_n_ticks == 0`일 때만 `spawn_blocking`으로 통계 행을 적재하도록 고친다:

```rust
fn spawn_tick_loop(
    state: SharedState,
    broadcaster: Broadcaster,
    db: DbHandle,
    config: ConfigHandle,
    metrics: MetricsHandle,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        let mut last_snapshot = {
            let guard = state.lock().await;
            to_snapshot(&guard, uuid::Uuid::nil())
        };

        loop {
            interval.tick().await;

            let persist_every_n_ticks = { *config.lock().await }.persist_every_n_ticks;

            let (message, next_snapshot, should_persist, stats_row) = {
                let mut guard = state.lock().await;
                match safe_tick(&guard.sim) {
                    Some(next_sim) => guard.sim = next_sim,
                    None => metrics.tick_panics_total.inc(),
                }

                let mut total_production_value = 0.0_f32;
                if guard.conveyor.running {
                    let units: HashMap<u32, f32> = guard.sim.robots.iter().map(|r| (r.id, 0.01)).collect();
                    total_production_value = total_production(&guard.sim.robots, &units);
                }

                metrics.ticks_total.inc();
                metrics.robot_count.set(guard.sim.robots.len() as i64);

                let current_snapshot = to_snapshot(&guard, uuid::Uuid::nil());
                let delta = match (&last_snapshot, &current_snapshot) {
                    (
                        protocol::ServerMessage::Snapshot { conveyor: prev_conveyor, robots: prev_robots, .. },
                        protocol::ServerMessage::Snapshot { tick: cur_tick, conveyor: cur_conveyor, robots: cur_robots, .. },
                    ) => compute_delta(*prev_conveyor, prev_robots, *cur_tick, *cur_conveyor, cur_robots),
                    _ => current_snapshot.clone(),
                };

                let should_persist = guard.sim.tick_count % persist_every_n_ticks == 0;
                let stats_row = persistence::StatsRow {
                    tick: guard.sim.tick_count,
                    robot_count: guard.sim.robots.len(),
                    conveyor_running: guard.conveyor.running,
                    total_production: total_production_value,
                };

                (delta, current_snapshot, should_persist, stats_row)
            };

            last_snapshot = next_snapshot;
            let _ = broadcaster.send(message);

            if should_persist {
                let db = Arc::clone(&db);
                tokio::task::spawn_blocking(move || {
                    let conn = db.lock().unwrap();
                    if let Err(err) = persistence::insert_stats(&conn, &stats_row) {
                        tracing::error!(%err, "failed to persist stats row");
                    }
                });
            }
        }
    });
}
```

(`total_production`은 이미 `sim_core::production`에서 import돼 있다 — 이제 실제로 반환값을 `total_production_value`에 저장해서 DB에 적재한다. 이전까지는 `let _ = total_production(...)`로 버렸었다.)

`build_app`에 db/config/metrics 레이어와 새 라우트를 추가:
```rust
pub fn build_app(
    state: SharedState,
    broadcaster: Broadcaster,
    sessions: ws::SessionHandle,
    db: DbHandle,
    config: ConfigHandle,
    metrics: MetricsHandle,
) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_route))
        .route("/api/stats/history", get(stats_history))
        .route("/api/config", get(get_config).post(post_config))
        .route("/metrics", get(metrics_route))
        .with_state(state)
        .layer(axum::extract::Extension(broadcaster))
        .layer(axum::extract::Extension(sessions))
        .layer(axum::extract::Extension(db))
        .layer(axum::extract::Extension(config))
        .layer(axum::extract::Extension(metrics))
}
```

`main()`을 아래로 교체 — DB 경로를 환경 변수로 받아 통합테스트가 서로 다른 파일을 쓸 수 있게 한다:
```rust
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let state = initial_state();
    let (broadcaster, _rx) = tokio::sync::broadcast::channel::<protocol::ServerMessage>(32);
    let sessions: ws::SessionHandle = Arc::new(Mutex::new(session::SessionRegistry::new()));

    let db_path = std::env::var("GAMEROBOTFACTORY_DB_PATH").unwrap_or_else(|_| "gamerobotfactory.sqlite3".to_string());
    let db: DbHandle = Arc::new(StdMutex::new(
        persistence::open_db(&db_path).expect("failed to open sqlite db"),
    ));
    let config: ConfigHandle = Arc::new(Mutex::new(AppConfig::default()));
    let metrics: MetricsHandle = Arc::new(Metrics::new());

    spawn_tick_loop(state.clone(), broadcaster.clone(), db.clone(), config.clone(), metrics.clone());

    let app = build_app(state, broadcaster, sessions, db, config, metrics);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind 127.0.0.1:0");
    println!("LISTENING_PORT={}", listener.local_addr().unwrap().port());
    axum::serve(listener, app).await.expect("server exited with an error");
}
```

`mod persistence; mod config; mod metrics;` 선언이 파일 상단에 이미 있는지 확인하고(Task 3~5에서 추가했다면 있음), 없으면 추가한다.

- [ ] **Step 2: `ws.rs`에 연결 수(`connected_clients`) 배선**

`connected_clients`는 WS 연결의 수명 동안만 유효한 게이지다. `handle_socket`에는 exit 지점이 여럿 있다 — 초기 스냅샷 전송 실패 시의 `return`, `tokio::select!` 루프 안의 여러 `break`(소켓 에러, 정상 종료, 브로드캐스트 채널 종료). 이 지점마다 수동으로 `inc`/`dec`를 맞춰 넣으면 하나라도 빠뜨렸을 때 게이지가 조용히 계속 늘어나기만 하는 버그가 된다(Task 5 코드 리뷰에서 지적됨). 대신 RAII 가드로 수명을 자동으로 묶는다.

`server/src/ws.rs`에 추가:
```rust
struct ConnectionGuard<'a> {
    counter: &'a prometheus::IntGauge,
}

impl<'a> ConnectionGuard<'a> {
    fn new(counter: &'a prometheus::IntGauge) -> Self {
        counter.inc();
        ConnectionGuard { counter }
    }
}

impl<'a> Drop for ConnectionGuard<'a> {
    fn drop(&mut self) {
        self.counter.dec();
    }
}
```

`ws_route`에 네 번째 익스트랙터를 추가:
```rust
pub async fn ws_route(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    axum::extract::Extension(broadcaster): axum::extract::Extension<Broadcaster>,
    axum::extract::Extension(sessions): axum::extract::Extension<SessionHandle>,
    axum::extract::Extension(metrics): axum::extract::Extension<crate::metrics::MetricsHandle>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, broadcaster, sessions, metrics))
}
```

`handle_socket`의 시그니처에 `metrics: crate::metrics::MetricsHandle`를 추가하고, 함수 맨 앞(세션 발급 직후, 어떤 `return`/`break`보다도 먼저)에 가드를 만든다:
```rust
async fn handle_socket(
    mut socket: WebSocket,
    state: SharedState,
    broadcaster: Broadcaster,
    sessions: SessionHandle,
    metrics: crate::metrics::MetricsHandle,
) {
    let mut updates = broadcaster.subscribe();

    let own_session_id = {
        let mut registry = sessions.lock().await;
        registry.start_session(std::time::Instant::now())
    };

    let _connection_guard = ConnectionGuard::new(&metrics.connected_clients);

    // (이후 기존 초기 스냅샷 전송 블록 + tokio::select! 루프는 그대로 유지)
    ...
}
```
`_connection_guard`는 `handle_socket`이 어떤 경로로 끝나든(정상 종료, 여러 `break`, 심지어 패닉) 함수 스코프를 벗어나는 순간 `Drop`이 실행되어 자동으로 `dec()`한다 — 각 exit 지점마다 수동으로 맞출 필요가 없다. `build_app`에 이미 있는 `.layer(axum::extract::Extension(metrics))`가 `/ws` 라우트에도 적용되므로 라우터 쪽 추가 배선은 필요 없다.

- [ ] **Step 3: 빌드 확인**

Run: `cargo build --manifest-path server/Cargo.toml`
Expected: 컴파일 성공. `DbHandle`이 `std::sync::Mutex`(rusqlite 커넥션은 `Send`지만 `tokio::sync::Mutex`로 감쌀 필요는 없다 — 잠깐 잠그고 동기 작업만 하므로 `std::sync::Mutex` + `spawn_blocking`이 더 적절)를 쓰는지 확인.

- [ ] **Step 4: 수동 확인**

서버를 띄우고: `curl http://127.0.0.1:<port>/api/config` (기본값 JSON), `curl -X POST -H "Content-Type: application/json" -d '{"persist_every_n_ticks":1}' http://127.0.0.1:<port>/api/config` (갱신), 1초 정도 기다린 뒤 `curl http://127.0.0.1:<port>/api/stats/history` (행이 쌓였는지), `curl http://127.0.0.1:<port>/metrics` (Prometheus 텍스트, `gamerobotfactory_connected_clients`가 WS 클라이언트 접속/해제에 따라 오르내리는지도 실제 WS 클라이언트를 하나 붙였다 떼면서 확인)를 각각 실제로 호출해 확인한다.

- [ ] **Step 5: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 이전 스위트 전부 PASS. `main.rs`/`ws.rs`를 크게 고쳤으니 회귀가 없는지 특히 주의해서 확인한다(기존 WS 통합테스트 2개 + Lagged 테스트도 여전히 통과해야 한다).

- [ ] **Step 6: Commit**

```bash
git add server/src/main.rs server/src/ws.rs
git commit -m "feat: wire persistence, config, and metrics into the server's state and routes"
```

---

### Task 8: Lagged 리싱크 통합테스트

Task 1에서 이미 구현은 끝났다(코드가 `handle_socket`에 들어있다) — 여기서는 그 동작을 실제 서버로 검증하는 통합테스트만 추가한다.

**Files:**
- Modify: `server/tests/ws_integration.rs`

- [ ] **Step 1: 테스트 추가**

`server/tests/ws_integration.rs`에 추가:
```rust
#[tokio::test]
async fn lagged_client_resyncs_instead_of_disconnecting() {
    let server = spawn_server();

    let url = format!("ws://127.0.0.1:{}/ws", server.port);
    let (ws_stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("failed to connect to ws endpoint");
    let (_write, mut read) = ws_stream.split();

    let _first = read.next().await.expect("stream ended early");

    // 브로드캐스트 채널 용량(32)을 넘기도록 충분히 오래 읽지 않는다.
    // 20Hz(50ms) 기준 32개 버퍼 ≈ 1.6초 — 3초 넘게 기다리면 확실히 넘친다.
    tokio::time::sleep(Duration::from_secs(3)).await;

    let mut still_receiving = false;
    for _ in 0..10 {
        match tokio::time::timeout(Duration::from_millis(500), read.next()).await {
            Ok(Some(Ok(_))) => {
                still_receiving = true;
                break;
            }
            _ => continue,
        }
    }
    assert!(still_receiving, "connection should survive a lag/resync event, not be dropped");
}
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml --test ws_integration`
Expected: 3개(기존 2개 + 신규 1개) PASS. 이 테스트는 3초 이상 걸리므로 느리다 — 정상이다.

- [ ] **Step 3: Commit**

```bash
git add server/tests/ws_integration.rs
git commit -m "test: verify a lagged broadcast receiver resyncs instead of being dropped"
```

---

### Task 9: 재접속(Resume) 통합테스트

**Files:**
- Modify: `server/tests/ws_integration.rs`

- [ ] **Step 1: 테스트 추가**

```rust
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
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
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
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
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
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        if json["kind"] == "ResumeAck" {
            assert_eq!(json["resumed"], false);
            saw_ack = true;
            break;
        }
    }
    assert!(saw_ack, "expected a ResumeAck message with resumed:false");
}
```

`uuid` 크레이트를 이 테스트 파일에서 쓰려면 `server/Cargo.toml`의 `[dev-dependencies]`에도 `uuid = { version = "1", features = ["v4"] }`를 추가해야 할 수 있다 — 이미 `[dependencies]`에 있어도 통합테스트(별도 크레이트 취급)에서 쓰려면 dev-dependencies에도 선언이 필요한지 확인하고, 필요하면 추가한다.

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml --test ws_integration`
Expected: 5개(기존 3개 + 신규 2개) PASS.

- [ ] **Step 3: Commit**

```bash
git add server/Cargo.toml server/tests/ws_integration.rs
git commit -m "test: verify session resume acknowledges valid and rejects unknown session ids"
```

---

### Task 10: REST + 영속화 + 메트릭 통합테스트, 전체 검증

**Files:**
- Create: `server/tests/rest_integration.rs`

- [ ] **Step 1: 테스트 작성**

`server/tests/rest_integration.rs`:
```rust
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
```

이 파일은 `reqwest`를 HTTP 클라이언트로 쓴다 — `server/Cargo.toml`의 `[dev-dependencies]`에 추가:
```toml
reqwest = { version = "0.12", features = ["json"] }
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml --test rest_integration`
Expected: 4개 PASS.

- [ ] **Step 3: 전체 스위트 + clippy 최종 확인**

Run: `cargo test --manifest-path server/Cargo.toml && cargo clippy --manifest-path server/Cargo.toml --all-targets`
Expected: 전부 PASS, 경고 0개. 이 Plan의 마지막 태스크이므로 두 번 이상 반복 실행해 타이밍에 의한 flaky 여부도 확인한다(특히 Lagged/Resume/stats 통합테스트는 실제 시간에 의존한다).

- [ ] **Step 4: Commit**

```bash
git add server/Cargo.toml server/tests/rest_integration.rs
git commit -m "test: add REST/persistence/metrics integration tests against the real server binary"
```

---

## Plan 3 완료 후 상태

- Plan 2 종료 시점에 KANBAN에 명시적으로 남겨뒀던 세 가지 하드닝 갭(재접속 실배선, Lagged 리싱크, 틱 루프 패닉 방어)이 모두 코드로 구현되고 통합테스트로 검증됨.
- SQLite에 생산 통계가 주기적으로 쌓이고 `GET /api/stats/history`로 조회 가능.
- `GET`/`POST /api/config`로 실시간(WS)과 분리된 설정 채널이 실제로 동작.
- `tracing` 구조화 로깅 + `/metrics` Prometheus 엔드포인트로 관측가능성 확보.
- 아직 없는 것: 클라이언트 렌더링(Plan 4), 데모/Docker 배포(Plan 5).
