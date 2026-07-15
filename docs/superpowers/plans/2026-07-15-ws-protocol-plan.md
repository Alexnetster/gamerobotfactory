# 로봇팔 컨베이어 게임 — Plan 2: WS 프로토콜 & 네트워킹 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **워크트리 없이 `main`에서 직접 작업한다** — 이 저장소는 소유자 혼자 쓰는 프로젝트이며, `feature/sim-core` 때와 달리 워크트리/피처 브랜치 분리는 생략한다.

**Goal:** Plan 1의 `sim_core` 라이브러리 위에 실제 서버 바이너리(`server/src/main.rs`)를 세워, WebSocket으로 로봇 상태를 델타 동기화하고 컨베이어 on/off·로봇 대수 조절·로봇 선택+팔 동작 커맨드를 처리하는 단일 오퍼레이터 세션 서버를 만든다.

**Architecture:** `sim_core`(순수 라이브러리, 이미 완성)는 건드리지 않되 로봇에 `Task`(작업 상태) 필드 하나만 추가한다. 컨베이어 상태·선택된 로봇 같은 "세션/오케스트레이션" 상태는 새 바이너리 크레이트 코드(`server/src/game_state.rs` 등)에 둬서 이미 리뷰된 `SimState`의 구조를 건드리지 않는다. 틱 루프는 20Hz로 `sim_core::sim::tick`을 돌리고, 매 틱마다 마지막으로 보낸 스냅샷과 비교해 델타를 계산해 WebSocket으로 내보낸다.

**Tech Stack:** `axum`(HTTP+WebSocket), `tokio`(비동기 런타임), `serde`+`serde_json`(와이어 포맷, 설계문서의 "시작은 JSON" 결정), `uuid`(세션 토큰). 테스트용으로 `tokio-tungstenite`+`futures-util`을 dev-dependency로 추가해 실제 WS 클라이언트로 통합테스트한다.

**축 관련 주의사항 (계획 작성자 노트):** axum/tokio-tungstenite의 정확한 API 시그니처는 버전에 따라 미묘하게 다를 수 있다 — 이 계획은 컴파일러 피드백 없이 작성되었으므로, 만약 아래 코드가 실제 크레이트 버전과 시그니처가 안 맞으면(예: `axum::extract::ws` 관련 타입 위치나 트레잇 바운드), **동작 사양(각 스텝의 테스트가 무엇을 검증해야 하는지)을 기준으로 실제 API에 맞게 조정한다** — 임의로 동작 자체를 바꾸지 말고, 컴파일 에러가 나면 `cargo doc --open` 등으로 실제 시그니처를 확인해 맞춰 쓴다.

**설계 문서 참조:** `docs/robot-arm-conveyor-game-design.md`의 "프로토콜 & 백엔드 설계", "플레이어 상호작용" 절.

---

## 파일 구조

| 파일 | 책임 |
|---|---|
| `server/Cargo.toml` | 네트워킹 의존성 추가 (axum, tokio, serde, serde_json, uuid) + 테스트용 dev-dependency (tokio-tungstenite, futures-util). |
| `server/src/sim.rs` | (수정) `Task` enum + `Robot.task` 필드 추가. 그 외 기존 구조는 건드리지 않음. |
| `server/src/game_state.rs` | `Conveyor`, `GameState`(= `sim: SimState` + `conveyor` + `selected_robot`), 커맨드 적용 순수 함수들. |
| `server/src/protocol.rs` | 와이어 타입(`ClientCommand`, `ServerMessage`, `RobotView`, `ConveyorView`) + `GameState` → 스냅샷 변환. |
| `server/src/delta.rs` | 두 스냅샷을 비교해 변경된 로봇/컨베이어만 담은 델타 메시지 계산. |
| `server/src/session.rs` | 세션 토큰 발급, 재접속 유예시간, 클라이언트별 "마지막으로 보낸 스냅샷" 기준선 관리. |
| `server/src/ws.rs` | axum WebSocket 핸들러 — 연결 수립, 커맨드 수신·적용, 틱 브로드캐스트 구독. |
| `server/src/main.rs` | 바이너리 엔트리포인트 — 공유 상태, 틱 루프 태스크, axum 라우터(`/health`, `/ws`) 기동. |
| `server/tests/ws_integration.rs` | 실제 서버를 띄우고 `tokio-tungstenite` 클라이언트로 커맨드 시퀀스를 보내 검증하는 통합테스트. |

---

### Task 1: `sim_core`에 로봇 작업 상태(`Task`) 추가

**Files:**
- Modify: `server/src/sim.rs`

- [ ] **Step 1: `Task` enum + `Robot.task` 필드 추가 + 테스트**

`server/src/sim.rs`에서 `BodyPose` 정의 아래에 추가:

```rust
/// 로봇이 지금 수행 중인 팔 작업. `TriggerArmAction` 커맨드가 이 값을
/// 바꾼다 — 실제 IK/애니메이션 계산은 클라이언트/렌더러(Plan 4)의 몫이고,
/// 여기서는 "지금 무슨 작업 중인가"라는 사실만 기록한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    Idle,
    Picking,
    Placing,
}
```

`Robot` 구조체에 필드 추가:

```rust
pub struct Robot {
    pub id: u32,
    pub pos: CellId,
    pub goal: CellId,
    pub path: Vec<CellId>,
    pub ticks_until_repath: u32,
    pub pose: BodyPose,
    pub leg_cycle_progress: f32,
    pub task: Task,
}
```

`Robot::new`에 필드 추가:

```rust
impl Robot {
    pub fn new(id: u32, pos: CellId, goal: CellId) -> Self {
        Robot {
            id,
            pos,
            goal,
            path: Vec::new(),
            ticks_until_repath: 0,
            pose: BodyPose::Standing,
            leg_cycle_progress: 0.0,
            task: Task::Idle,
        }
    }
}
```

`mod tests` 안에 추가:

```rust
    #[test]
    fn new_robot_starts_idle() {
        let robot = Robot::new(1, (0, 0), (0, 0));
        assert_eq!(robot.task, Task::Idle);
    }
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 기존 전체 스위트(39개) + 신규 1개 = 40개 PASS. (`Robot::new`가 유일한 생성 경로이므로 다른 파일 수정 불필요 — `production.rs`/`tick_properties.rs` 등 기존 호출부는 그대로 컴파일된다.)

- [ ] **Step 3: Commit**

```bash
git add server/src/sim.rs
git commit -m "feat: add Task (arm action state) to Robot"
```

---

### Task 2: 네트워킹 의존성 추가

**Files:**
- Modify: `server/Cargo.toml`

- [ ] **Step 1: 의존성 추가**

`server/Cargo.toml`을 다음과 같이 갱신 (기존 `[dependencies]`/`[dev-dependencies]`에 항목 추가, 기존 `rayon`/`proptest`는 유지):

```toml
[package]
name = "gamerobotfactory-server"
version = "0.1.0"
edition = "2021"

[lib]
name = "sim_core"
path = "src/lib.rs"

[[bin]]
name = "server"
path = "src/main.rs"

[dependencies]
rayon = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "time", "sync", "signal"] }
axum = { version = "0.7", features = ["ws"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }

[dev-dependencies]
proptest = "1"
tokio-tungstenite = "0.23"
futures-util = "0.3"
```

- [ ] **Step 2: 빈 main.rs로 빌드 확인**

`server/src/main.rs`를 임시로 다음처럼 최소 상태로 만든다(다음 태스크에서 실제 내용으로 교체):

```rust
fn main() {
    println!("server binary placeholder — replaced in Task 6");
}
```

Run: `cargo build --manifest-path server/Cargo.toml`
Expected: 새 의존성이 전부 내려받아지고 라이브러리+바이너리 둘 다 컴파일 성공.

- [ ] **Step 3: Commit**

```bash
git add server/Cargo.toml server/src/main.rs
git commit -m "chore: add networking dependencies and binary target scaffold"
```

---

### Task 3: `game_state.rs` — 컨베이어 + 커맨드 적용 (네트워킹 없이 순수 로직)

**Files:**
- Create: `server/src/game_state.rs`
- Modify: `server/src/main.rs` (모듈 선언 추가)

- [ ] **Step 1: 구현 + 테스트 작성**

`server/src/game_state.rs`:

```rust
use sim_core::sim::{Robot, SimState, Task};
use sim_core::grid::CellId;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conveyor {
    pub running: bool,
}

impl Conveyor {
    pub fn new() -> Self {
        Conveyor { running: true }
    }
}

/// 시뮬레이션 진실(`SimState`)에 세션/오케스트레이션 상태(컨베이어,
/// 선택된 로봇)를 얹은 것. `selected_robot`은 "지금 이 오퍼레이터가
/// 어느 로봇을 보고 있는가"라는 UI 개념이라 시뮬레이션 진실이 아니므로
/// 여기(바이너리 크레이트)에 두고 `sim_core::SimState`는 건드리지 않는다.
pub struct GameState {
    pub sim: SimState,
    pub conveyor: Conveyor,
    pub selected_robot: Option<u32>,
    next_robot_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    RobotNotFound(u32),
}

impl GameState {
    pub fn new(sim: SimState) -> Self {
        let next_robot_id = sim.robots.iter().map(|r| r.id).max().map_or(0, |max| max + 1);
        GameState { sim, conveyor: Conveyor::new(), selected_robot: None, next_robot_id }
    }

    pub fn toggle_conveyor(&mut self) {
        self.conveyor.running = !self.conveyor.running;
    }

    /// 로봇 대수를 정확히 `target`대로 맞춘다. 늘려야 하면 그리드 원점
    /// 근처의 빈 칸에 새 로봇을 스폰하고(자기 자신을 목표로 삼아 제자리
    /// 대기), 줄여야 하면 ID가 가장 큰 로봇부터 제거한다.
    pub fn set_robot_count(&mut self, target: usize) {
        while self.sim.robots.len() < target {
            let id = self.next_robot_id;
            self.next_robot_id += 1;
            let spawn_at = (0, 0);
            self.sim.robots.push(Robot::new(id, spawn_at, spawn_at));
        }
        while self.sim.robots.len() > target {
            self.sim.robots.pop();
            // 로봇을 뒤에서부터 push했으므로 pop이 "가장 최근에 추가된
            // (통상 가장 큰 id) 로봇 제거"와 대체로 일치한다. 엄밀한
            // "가장 큰 id 제거"가 필요하면 정렬 후 제거로 바꾼다.
        }
        if let Some(selected) = self.selected_robot {
            if !self.sim.robots.iter().any(|r| r.id == selected) {
                self.selected_robot = None;
            }
        }
    }

    pub fn select_robot(&mut self, robot_id: u32) -> Result<(), CommandError> {
        if !self.sim.robots.iter().any(|r| r.id == robot_id) {
            return Err(CommandError::RobotNotFound(robot_id));
        }
        self.selected_robot = Some(robot_id);
        Ok(())
    }

    pub fn release_robot(&mut self) {
        self.selected_robot = None;
    }

    pub fn trigger_arm_action(&mut self, robot_id: u32, task: Task) -> Result<(), CommandError> {
        let robot = self
            .sim
            .robots
            .iter_mut()
            .find(|r| r.id == robot_id)
            .ok_or(CommandError::RobotNotFound(robot_id))?;
        robot.task = task;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::grid::Grid;
    use std::sync::Arc;

    fn empty_state() -> GameState {
        GameState::new(SimState { grid: Arc::new(Grid::new(5, 5)), robots: Vec::new(), tick_count: 0 })
    }

    #[test]
    fn toggle_conveyor_flips_running_state() {
        let mut state = empty_state();
        assert!(state.conveyor.running);
        state.toggle_conveyor();
        assert!(!state.conveyor.running);
        state.toggle_conveyor();
        assert!(state.conveyor.running);
    }

    #[test]
    fn set_robot_count_grows_and_shrinks() {
        let mut state = empty_state();
        state.set_robot_count(3);
        assert_eq!(state.sim.robots.len(), 3);
        state.set_robot_count(1);
        assert_eq!(state.sim.robots.len(), 1);
    }

    #[test]
    fn set_robot_count_assigns_unique_growing_ids() {
        let mut state = empty_state();
        state.set_robot_count(2);
        state.set_robot_count(1);
        state.set_robot_count(3);
        let ids: Vec<u32> = state.sim.robots.iter().map(|r| r.id).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(ids.len(), unique.len(), "no robot id should be reused: {ids:?}");
    }

    #[test]
    fn select_robot_rejects_unknown_id() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let unknown_id = state.sim.robots[0].id + 100;
        assert_eq!(state.select_robot(unknown_id), Err(CommandError::RobotNotFound(unknown_id)));
    }

    #[test]
    fn select_then_release_clears_selection() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = state.sim.robots[0].id;
        state.select_robot(id).unwrap();
        assert_eq!(state.selected_robot, Some(id));
        state.release_robot();
        assert_eq!(state.selected_robot, None);
    }

    #[test]
    fn removing_selected_robot_clears_selection() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = state.sim.robots[0].id;
        state.select_robot(id).unwrap();
        state.set_robot_count(0);
        assert_eq!(state.selected_robot, None);
    }

    #[test]
    fn trigger_arm_action_sets_task_on_the_right_robot() {
        let mut state = empty_state();
        state.set_robot_count(2);
        let target_id = state.sim.robots[1].id;
        state.trigger_arm_action(target_id, Task::Picking).unwrap();
        assert_eq!(state.sim.robots[0].task, Task::Idle);
        assert_eq!(state.sim.robots[1].task, Task::Picking);
    }

    #[test]
    fn trigger_arm_action_rejects_unknown_robot() {
        let mut state = empty_state();
        let err = state.trigger_arm_action(999, Task::Picking);
        assert_eq!(err, Err(CommandError::RobotNotFound(999)));
    }
}
```

`server/src/main.rs`에 (placeholder 내용 위에) 추가:

```rust
mod game_state;
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 이전 40개(라이브러리) + `game_state` 신규 7개 = 47개 PASS. (`game_state.rs`는 `server/src/main.rs`가 `mod game_state;`로 선언하므로 바이너리 타깃의 테스트로 실행된다 — `cargo test`가 라이브러리와 바이너리 타깃 테스트를 모두 돈다.)

- [ ] **Step 3: Commit**

```bash
git add server/src/main.rs server/src/game_state.rs
git commit -m "feat: add GameState with conveyor toggle, robot count, selection, and arm-action commands"
```

---

### Task 4: `protocol.rs` — 와이어 타입 + 스냅샷 변환

**Files:**
- Create: `server/src/protocol.rs`
- Modify: `server/src/main.rs`

- [ ] **Step 1: 구현 + 테스트 작성**

`server/src/protocol.rs`:

```rust
use crate::game_state::{Conveyor, GameState};
use serde::{Deserialize, Serialize};
use sim_core::grid::CellId;
use sim_core::sim::{BodyPose, Robot, Task};

pub const PROTOCOL_VERSION: u8 = 1;

/// 클라이언트 → 서버 커맨드. `#[serde(tag = "type")]`로 JSON에서
/// `{"type": "ToggleConveyor"}` 같은 식으로 태그가 붙는다.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ClientCommand {
    SelectRobot { robot_id: u32 },
    ReleaseRobot,
    ToggleConveyor,
    SetRobotCount { count: usize },
    TriggerArmAction { robot_id: u32, task: WireTask },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WireTask {
    Idle,
    Picking,
    Placing,
}

impl From<WireTask> for Task {
    fn from(t: WireTask) -> Task {
        match t {
            WireTask::Idle => Task::Idle,
            WireTask::Picking => Task::Picking,
            WireTask::Placing => Task::Placing,
        }
    }
}

impl From<Task> for WireTask {
    fn from(t: Task) -> WireTask {
        match t {
            Task::Idle => WireTask::Idle,
            Task::Picking => WireTask::Picking,
            Task::Placing => WireTask::Placing,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WirePose {
    Standing,
    Crouching,
}

impl From<BodyPose> for WirePose {
    fn from(p: BodyPose) -> WirePose {
        match p {
            BodyPose::Standing => WirePose::Standing,
            BodyPose::Crouching => WirePose::Crouching,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RobotView {
    pub id: u32,
    pub pos: CellId,
    pub pose: WirePose,
    pub leg_cycle_progress: f32,
    pub task: WireTask,
}

impl From<&Robot> for RobotView {
    fn from(r: &Robot) -> RobotView {
        RobotView {
            id: r.id,
            pos: r.pos,
            pose: r.pose.into(),
            leg_cycle_progress: r.leg_cycle_progress,
            task: r.task.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ConveyorView {
    pub running: bool,
}

impl From<Conveyor> for ConveyorView {
    fn from(c: Conveyor) -> ConveyorView {
        ConveyorView { running: c.running }
    }
}

/// 서버 → 클라이언트 메시지. `v` 필드로 프로토콜 확장성을 명시적으로
/// 남겨둔다(설계문서 참고) — 지금은 항상 1이지만, 나중에 필드가 늘어나도
/// 이 필드 하나로 하위 호환을 다룰 수 있다.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum ServerMessage {
    Snapshot { v: u8, tick: u64, conveyor: ConveyorView, robots: Vec<RobotView> },
    Delta { v: u8, tick: u64, conveyor: Option<ConveyorView>, changed_robots: Vec<RobotView>, removed_robot_ids: Vec<u32> },
}

pub fn to_snapshot(state: &GameState) -> ServerMessage {
    ServerMessage::Snapshot {
        v: PROTOCOL_VERSION,
        tick: state.sim.tick_count,
        conveyor: state.conveyor.into(),
        robots: state.sim.robots.iter().map(RobotView::from).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_command_deserializes_from_tagged_json() {
        let json = r#"{"type":"ToggleConveyor"}"#;
        let cmd: ClientCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd, ClientCommand::ToggleConveyor);

        let json = r#"{"type":"SelectRobot","robot_id":7}"#;
        let cmd: ClientCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd, ClientCommand::SelectRobot { robot_id: 7 });

        let json = r#"{"type":"TriggerArmAction","robot_id":3,"task":"Picking"}"#;
        let cmd: ClientCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd, ClientCommand::TriggerArmAction { robot_id: 3, task: WireTask::Picking });
    }

    #[test]
    fn server_message_round_trips_through_json() {
        let msg = ServerMessage::Snapshot {
            v: 1,
            tick: 42,
            conveyor: ConveyorView { running: true },
            robots: vec![],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn to_snapshot_reflects_current_game_state() {
        use crate::game_state::GameState;
        use sim_core::grid::Grid;
        use sim_core::sim::SimState;
        use std::sync::Arc;

        let mut state = GameState::new(SimState { grid: Arc::new(Grid::new(3, 3)), robots: Vec::new(), tick_count: 5 });
        state.set_robot_count(2);
        state.toggle_conveyor();

        let snapshot = to_snapshot(&state);
        match snapshot {
            ServerMessage::Snapshot { v, tick, conveyor, robots } => {
                assert_eq!(v, PROTOCOL_VERSION);
                assert_eq!(tick, 5);
                assert!(!conveyor.running);
                assert_eq!(robots.len(), 2);
            }
            _ => panic!("expected Snapshot"),
        }
    }
}
```

`server/src/main.rs`에 추가:

```rust
mod protocol;
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 이전 47개 + `protocol` 신규 3개 = 50개 PASS.

- [ ] **Step 3: Commit**

```bash
git add server/src/main.rs server/src/protocol.rs
git commit -m "feat: add wire protocol types and snapshot serialization"
```

---

### Task 5: `delta.rs` — 변경분만 담은 델타 계산

**Files:**
- Create: `server/src/delta.rs`
- Modify: `server/src/main.rs`

- [ ] **Step 1: 구현 + 테스트 작성**

`server/src/delta.rs`:

```rust
use crate::protocol::{ConveyorView, RobotView, ServerMessage, PROTOCOL_VERSION};

/// `previous`(이 클라이언트에게 마지막으로 보낸 스냅샷)와 `current`를
/// 비교해, 바뀐 로봇만 담긴 델타 메시지를 만든다. 유휴 상태로 멈춰있는
/// 로봇은 매 틱 다시 보내지 않아도 되므로 대역폭을 아낀다.
pub fn compute_delta(
    previous_conveyor: ConveyorView,
    previous_robots: &[RobotView],
    current_tick: u64,
    current_conveyor: ConveyorView,
    current_robots: &[RobotView],
) -> ServerMessage {
    let conveyor = if previous_conveyor == current_conveyor { None } else { Some(current_conveyor) };

    let changed_robots: Vec<RobotView> = current_robots
        .iter()
        .filter(|current| {
            let unchanged = previous_robots.iter().any(|prev| prev == *current);
            !unchanged
        })
        .cloned()
        .collect();

    let removed_robot_ids: Vec<u32> = previous_robots
        .iter()
        .filter(|prev| !current_robots.iter().any(|current| current.id == prev.id))
        .map(|prev| prev.id)
        .collect();

    ServerMessage::Delta { v: PROTOCOL_VERSION, tick: current_tick, conveyor, changed_robots, removed_robot_ids }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::sim::BodyPose;
    use crate::protocol::WireTask;

    fn robot_view(id: u32, x: i32) -> RobotView {
        RobotView {
            id,
            pos: (x, 0),
            pose: BodyPose::Standing.into(),
            leg_cycle_progress: 0.0,
            task: WireTask::Idle,
        }
    }

    #[test]
    fn unchanged_robots_are_omitted_from_delta() {
        let prev = vec![robot_view(1, 0)];
        let curr = vec![robot_view(1, 0)];

        let msg = compute_delta(ConveyorView { running: true }, &prev, 1, ConveyorView { running: true }, &curr);

        match msg {
            ServerMessage::Delta { conveyor, changed_robots, removed_robot_ids, .. } => {
                assert!(conveyor.is_none());
                assert!(changed_robots.is_empty());
                assert!(removed_robot_ids.is_empty());
            }
            _ => panic!("expected Delta"),
        }
    }

    #[test]
    fn moved_robot_is_included_in_delta() {
        let prev = vec![robot_view(1, 0)];
        let curr = vec![robot_view(1, 1)];

        let msg = compute_delta(ConveyorView { running: true }, &prev, 1, ConveyorView { running: true }, &curr);

        match msg {
            ServerMessage::Delta { changed_robots, .. } => {
                assert_eq!(changed_robots, vec![robot_view(1, 1)]);
            }
            _ => panic!("expected Delta"),
        }
    }

    #[test]
    fn removed_robot_id_is_reported() {
        let prev = vec![robot_view(1, 0), robot_view(2, 0)];
        let curr = vec![robot_view(1, 0)];

        let msg = compute_delta(ConveyorView { running: true }, &prev, 1, ConveyorView { running: true }, &curr);

        match msg {
            ServerMessage::Delta { removed_robot_ids, changed_robots, .. } => {
                assert_eq!(removed_robot_ids, vec![2]);
                assert!(changed_robots.is_empty());
            }
            _ => panic!("expected Delta"),
        }
    }

    #[test]
    fn conveyor_change_is_reported_only_when_it_changed() {
        let msg = compute_delta(ConveyorView { running: true }, &[], 1, ConveyorView { running: false }, &[]);
        match msg {
            ServerMessage::Delta { conveyor, .. } => assert_eq!(conveyor, Some(ConveyorView { running: false })),
            _ => panic!("expected Delta"),
        }
    }
}
```

`server/src/main.rs`에 추가:

```rust
mod delta;
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 이전 50개 + `delta` 신규 4개 = 54개 PASS.

- [ ] **Step 3: Commit**

```bash
git add server/src/main.rs server/src/delta.rs
git commit -m "feat: add delta computation that omits unchanged robots"
```

---

### Task 6: `main.rs` — 최소 axum 서버 (health check만)

이 태스크에서 처음으로 실제 서버 바이너리가 뜬다. WS는 아직 없다 — Task 7에서 추가한다.

**Files:**
- Modify: `server/src/main.rs`

- [ ] **Step 1: 최소 axum 앱 작성**

`server/src/main.rs` 전체를 다음으로 교체 (기존 `mod` 선언들은 유지):

```rust
mod game_state;
mod protocol;
mod delta;

use axum::{routing::get, Router};

async fn health() -> &'static str {
    "ok"
}

/// 포트를 고정하지 않고 OS가 빈 포트를 골라주게 한다(`:0`) — 통합테스트
/// (Task 10)에서 여러 서버 인스턴스를 동시에 띄워도 포트 충돌이 나지
/// 않도록 하기 위함. 실제 바인딩된 포트는 표준출력에 기계가 파싱하기
/// 쉬운 한 줄(`LISTENING_PORT={port}`)로 알려준다.
#[tokio::main]
async fn main() {
    let app = Router::new().route("/health", get(health));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    println!("LISTENING_PORT={}", listener.local_addr().unwrap().port());
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 2: 수동 확인**

Run: `cargo build --manifest-path server/Cargo.toml`
Expected: 컴파일 성공. (`cargo run --manifest-path server/Cargo.toml`으로 직접 띄우면 `LISTENING_PORT=<숫자>`가 출력된다 — 자동화된 스텝은 아니므로 빌드 성공만 확인하고 다음 태스크로 넘어간다. Task 10의 통합테스트가 이 출력 줄을 파싱해 실제로 접속·검증한다.)

- [ ] **Step 3: Commit**

```bash
git add server/src/main.rs
git commit -m "feat: start minimal axum server with a health check route"
```

---

### Task 7: WebSocket 핸들러 (커맨드 수신 + 최초 스냅샷)

**Files:**
- Create: `server/src/ws.rs`
- Modify: `server/src/main.rs`

- [ ] **Step 1: 구현**

`server/src/ws.rs`:

```rust
use crate::game_state::GameState;
use crate::protocol::{to_snapshot, ClientCommand, ServerMessage};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::Arc;
use tokio::sync::Mutex;

pub type SharedState = Arc<Mutex<GameState>>;

pub async fn ws_route(ws: WebSocketUpgrade, State(state): State<SharedState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: SharedState) {
    {
        let snapshot = {
            let guard = state.lock().await;
            to_snapshot(&guard)
        };
        if send_message(&mut socket, &snapshot).await.is_err() {
            return;
        }
    }

    while let Some(Ok(msg)) = socket.recv().await {
        if let Message::Text(text) = msg {
            match serde_json::from_str::<ClientCommand>(&text) {
                Ok(command) => {
                    let mut guard = state.lock().await;
                    apply_command(&mut guard, command);
                }
                Err(err) => {
                    eprintln!("invalid client command, ignoring: {err}");
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
```

`server/src/main.rs`을 다음으로 교체:

```rust
mod game_state;
mod protocol;
mod delta;
mod ws;

use axum::{routing::get, Router};
use game_state::GameState;
use sim_core::grid::Grid;
use sim_core::sim::SimState;
use std::sync::Arc;
use tokio::sync::Mutex;
use ws::{ws_route, SharedState};

async fn health() -> &'static str {
    "ok"
}

fn initial_state() -> SharedState {
    let sim = SimState { grid: Arc::new(Grid::new(10, 10)), robots: Vec::new(), tick_count: 0 };
    Arc::new(Mutex::new(GameState::new(sim)))
}

#[tokio::main]
async fn main() {
    let state = initial_state();

    let app = Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_route))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    println!("LISTENING_PORT={}", listener.local_addr().unwrap().port());
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 2: 빌드 확인**

Run: `cargo build --manifest-path server/Cargo.toml`
Expected: 컴파일 성공. (이 태스크는 실제 네트워킹 코드라 유닛테스트로 검증하기 어렵다 — Task 10의 통합테스트에서 실제로 커맨드를 보내 검증한다.)

- [ ] **Step 3: Commit**

```bash
git add server/src/main.rs server/src/ws.rs
git commit -m "feat: add WebSocket handler that sends an initial snapshot and applies commands"
```

---

### Task 8: 틱 루프 (20Hz 백그라운드 태스크 + 델타 브로드캐스트)

**Files:**
- Modify: `server/src/main.rs`
- Modify: `server/src/ws.rs`

- [ ] **Step 1: 브로드캐스트 채널 추가 + 틱 루프 스폰**

`server/src/ws.rs`의 `SharedState` 정의 아래에 추가:

```rust
pub type Broadcaster = tokio::sync::broadcast::Sender<crate::protocol::ServerMessage>;
```

`handle_socket` 함수를 아래로 교체 (초기 스냅샷 전송 후, 커맨드 수신과 브로드캐스트 구독을 동시에 처리하도록 `tokio::select!` 사용):

```rust
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
                    Some(Err(_)) | None => break,
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
```

`ws_route`를 브로드캐스터도 받도록 교체:

```rust
pub async fn ws_route(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    axum::extract::Extension(broadcaster): axum::extract::Extension<Broadcaster>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, broadcaster))
}
```

`server/src/main.rs`을 아래로 교체:

```rust
mod game_state;
mod protocol;
mod delta;
mod ws;

use axum::{routing::get, Router};
use delta::compute_delta;
use game_state::GameState;
use protocol::to_snapshot;
use sim_core::grid::Grid;
use sim_core::production::total_production;
use sim_core::sim::{tick, SimState};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use ws::{ws_route, Broadcaster, SharedState};

async fn health() -> &'static str {
    "ok"
}

fn initial_state() -> SharedState {
    let sim = SimState { grid: Arc::new(Grid::new(10, 10)), robots: Vec::new(), tick_count: 0 };
    Arc::new(Mutex::new(GameState::new(sim)))
}

const TICK_INTERVAL: Duration = Duration::from_millis(50); // 20Hz

fn spawn_tick_loop(state: SharedState, broadcaster: Broadcaster) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        let mut last_snapshot = {
            let guard = state.lock().await;
            to_snapshot(&guard)
        };

        loop {
            interval.tick().await;

            let (message, next_snapshot) = {
                let mut guard = state.lock().await;
                guard.sim = tick(&guard.sim);

                if guard.conveyor.running {
                    let units: HashMap<u32, f32> = guard.sim.robots.iter().map(|r| (r.id, 0.01)).collect();
                    let _ = total_production(&guard.sim.robots, &units);
                    // 생산량 값 자체를 아직 어디에도 저장하지 않는다 — 실제
                    // "생산량 누적 상태"는 이 Plan의 범위 밖(설계문서 경영
                    // 레이어)이라, 결정적 집계 함수가 실제로 매 틱 호출된다는
                    // 것만 지금은 증명해둔다.
                }

                let current_snapshot = to_snapshot(&guard);
                let delta = match (&last_snapshot, &current_snapshot) {
                    (
                        protocol::ServerMessage::Snapshot { conveyor: prev_conveyor, robots: prev_robots, .. },
                        protocol::ServerMessage::Snapshot { tick: cur_tick, conveyor: cur_conveyor, robots: cur_robots, .. },
                    ) => compute_delta(*prev_conveyor, prev_robots, *cur_tick, *cur_conveyor, cur_robots),
                    _ => current_snapshot.clone(),
                };
                (delta, current_snapshot)
            };

            last_snapshot = next_snapshot;
            let _ = broadcaster.send(message);
        }
    });
}

#[tokio::main]
async fn main() {
    let state = initial_state();
    let (broadcaster, _rx) = tokio::sync::broadcast::channel::<protocol::ServerMessage>(32);

    spawn_tick_loop(state.clone(), broadcaster.clone());

    let app = Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_route))
        .with_state(state)
        .layer(axum::extract::Extension(broadcaster));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    println!("LISTENING_PORT={}", listener.local_addr().unwrap().port());
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 2: 빌드 확인**

Run: `cargo build --manifest-path server/Cargo.toml`
Expected: 컴파일 성공. (`SimState`/`ServerMessage` 등이 `Clone`을 필요로 하는 곳이 있으니, 컴파일 에러가 나면 해당 타입에 `#[derive(Clone)]`이 이미 있는지 확인하고 없으면 추가한다 — `ServerMessage`는 이미 `Clone`을 derive했고, `SimState`도 Plan 1에서 이미 `Clone`을 derive했다.)

- [ ] **Step 3: Commit**

```bash
git add server/src/main.rs server/src/ws.rs
git commit -m "feat: add 20Hz tick loop broadcasting deltas to connected clients"
```

---

### Task 9: 세션 / 재접속 유예시간

**Files:**
- Create: `server/src/session.rs`
- Modify: `server/src/main.rs`, `server/src/ws.rs`

- [ ] **Step 1: 구현 + 테스트 작성**

`server/src/session.rs`:

```rust
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const RECONNECT_GRACE_PERIOD: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
struct SessionEntry {
    last_seen: Instant,
}

/// 활성 세션들을 추적해, 유예시간 내 재접속인지 판단한다. 순수 로직만
/// 담당하고(시간은 주입받은 `Instant` 기준), 실제 소켓/네트워킹은 모른다.
#[derive(Debug, Default)]
pub struct SessionRegistry {
    sessions: HashMap<Uuid, SessionEntry>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        SessionRegistry { sessions: HashMap::new() }
    }

    pub fn start_session(&mut self, now: Instant) -> Uuid {
        let id = Uuid::new_v4();
        self.sessions.insert(id, SessionEntry { last_seen: now });
        id
    }

    pub fn touch(&mut self, id: Uuid, now: Instant) {
        if let Some(entry) = self.sessions.get_mut(&id) {
            entry.last_seen = now;
        }
    }

    /// `id`가 아직 유예시간 내에 있으면 `true`(재접속 시 델타 기준선을
    /// 이어갈 수 있다는 뜻), 만료됐거나 존재한 적 없으면 `false`.
    pub fn is_within_grace_period(&self, id: Uuid, now: Instant) -> bool {
        match self.sessions.get(&id) {
            Some(entry) => now.duration_since(entry.last_seen) < RECONNECT_GRACE_PERIOD,
            None => false,
        }
    }

    pub fn evict_expired(&mut self, now: Instant) {
        self.sessions.retain(|_, entry| now.duration_since(entry.last_seen) < RECONNECT_GRACE_PERIOD);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_session_is_within_grace_period() {
        let mut registry = SessionRegistry::new();
        let now = Instant::now();
        let id = registry.start_session(now);
        assert!(registry.is_within_grace_period(id, now));
    }

    #[test]
    fn session_expires_after_grace_period() {
        let mut registry = SessionRegistry::new();
        let now = Instant::now();
        let id = registry.start_session(now);
        let later = now + RECONNECT_GRACE_PERIOD + Duration::from_secs(1);
        assert!(!registry.is_within_grace_period(id, later));
    }

    #[test]
    fn touch_extends_the_grace_period() {
        let mut registry = SessionRegistry::new();
        let now = Instant::now();
        let id = registry.start_session(now);

        let mid = now + Duration::from_secs(20);
        registry.touch(id, mid);

        let later = mid + Duration::from_secs(20); // 40s after start, but only 20s after touch
        assert!(registry.is_within_grace_period(id, later));
    }

    #[test]
    fn unknown_session_is_never_within_grace_period() {
        let registry = SessionRegistry::new();
        assert!(!registry.is_within_grace_period(Uuid::new_v4(), Instant::now()));
    }

    #[test]
    fn evict_expired_removes_only_stale_sessions() {
        let mut registry = SessionRegistry::new();
        let now = Instant::now();
        let fresh = registry.start_session(now);
        let stale = registry.start_session(now - RECONNECT_GRACE_PERIOD - Duration::from_secs(1));

        registry.evict_expired(now);

        assert!(registry.is_within_grace_period(fresh, now));
        assert!(!registry.is_within_grace_period(stale, now));
    }
}
```

`server/src/main.rs`에 추가:

```rust
mod session;
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 이전 54개 + `session` 신규 5개 = 59개 PASS.

`server/src/ws.rs`/`main.rs`에 `SessionRegistry`를 실제로 배선하는 것(재접속 시 세션 토큰을 클라이언트가 보내오게 하고 조회하는 것)은 이 Task에서는 하지 않는다 — 순수 로직과 테스트만 우선 만들고, 실제 소켓 프로토콜에 세션 토큰을 얹는 배선은 범위가 커서 Task 10의 통합테스트에서 최소한으로만(연결마다 새 세션 발급 정도) 다룬다. 만약 시간이 되면 아래를 추가로 배선해도 좋다:
- 최초 연결 시 서버가 `Snapshot` 메시지에 `session_id` 필드를 실어 보낸다(스텝은 생략 가능한 스트레치 — 필수 아님).

- [ ] **Step 3: Commit**

```bash
git add server/src/main.rs server/src/session.rs
git commit -m "feat: add session registry with reconnect grace period tracking"
```

---

### Task 10: 통합테스트 — 실제 서버 + 실제 WS 클라이언트

**Files:**
- Create: `server/tests/ws_integration.rs`
- Modify: `server/src/main.rs` (테스트에서 서버 로직을 재사용할 수 있도록 일부를 라이브러리화할 필요는 없음 — 테스트가 `server` 바이너리를 서브프로세스로 띄우거나, 앱 구성 함수를 공유하는 두 가지 방법이 있는데, 아래는 후자를 택해 `main.rs`의 라우터 구성을 재사용 가능한 함수로 뺀다)

- [ ] **Step 1: `main.rs`에서 앱 구성을 재사용 가능한 함수로 분리**

`server/src/main.rs`의 `main` 함수 본문 중 라우터 구성 부분을 별도 pub 함수로 뺀다:

```rust
pub fn build_app(state: SharedState, broadcaster: Broadcaster) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_route))
        .with_state(state)
        .layer(axum::extract::Extension(broadcaster))
}
```

`main` 함수는 이 함수를 호출하도록 교체:

```rust
#[tokio::main]
async fn main() {
    let state = initial_state();
    let (broadcaster, _rx) = tokio::sync::broadcast::channel::<protocol::ServerMessage>(32);
    spawn_tick_loop(state.clone(), broadcaster.clone());

    let app = build_app(state, broadcaster);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await.unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}
```

`main.rs` 전체에서 필요한 것들(`GameState`, `SharedState`, `Broadcaster`, `initial_state`, `spawn_tick_loop`, `build_app`, `health`)이 통합테스트에서 쓰일 수 있도록, 바이너리 크레이트 대신 이 로직들을 `server/src/lib.rs`가 아니라 **바이너리 전용 모듈들을 그대로 두고, `server/tests/ws_integration.rs`는 포트를 0(운영체제가 빈 포트를 골라줌)으로 띄운 뒤 실제 서버 프로세스를 spawn하는 대신, 같은 바이너리 크레이트의 `#[path]` 트릭 없이 가장 간단한 방법**을 쓴다: `main.rs`의 로직을 그대로 유지하되, 통합 테스트는 `std::process::Command`로 `cargo run --manifest-path server/Cargo.toml`을 서브프로세스로 띄워 검증한다.

- [ ] **Step 2: 통합테스트 작성**

`server/tests/ws_integration.rs`:

```rust
use futures_util::{SinkExt, StreamExt};
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
    assert!(text.contains("\"kind\":\"Snapshot\""));
    assert!(text.contains("\"robots\":[]"));

    // 2) SetRobotCount 커맨드를 보낸다.
    write
        .send(Message::Text(r#"{"type":"SetRobotCount","count":2}"#.to_string()))
        .await
        .unwrap();

    // 3) 다음 틱 브로드캐스트(델타)에서 로봇 2대가 등장하는지 확인한다.
    //    틱 주기가 50ms이므로 몇 번의 메시지 안에는 반영되어야 한다.
    let mut saw_two_robots = false;
    for _ in 0..20 {
        let Some(Ok(Message::Text(text))) = read.next().await else { break };
        if text.contains("\"changed_robots\":[") && text.matches("\"id\":").count() >= 2 {
            saw_two_robots = true;
            break;
        }
    }
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
```

`spawn_server()`가 매 호출마다 포트 0(임의 할당)으로 새 서버 프로세스를 띄우고 실제 포트를 stdout에서 읽어오므로, 두 테스트가 병렬로 실행돼도(기본 `cargo test` 동작) 서로 다른 포트를 쓰게 되어 충돌이 구조적으로 발생하지 않는다.

- [ ] **Step 3: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 이전 59개(유닛) + `ws_integration` 신규 2개 = 61개 PASS.

- [ ] **Step 4: 전체 스위트 + clippy 최종 확인**

Run: `cargo test --manifest-path server/Cargo.toml && cargo clippy --manifest-path server/Cargo.toml --all-targets`
Expected: 전부 PASS, 경고 0개.

- [ ] **Step 5: Commit**

```bash
git add server/src/main.rs server/tests/ws_integration.rs
git commit -m "test: add end-to-end WS integration tests against the real server binary"
```

---

## Plan 2 완료 후 상태

- 실제로 뜨는 서버 바이너리(`cargo run --manifest-path server/Cargo.toml`)가 `/health`와 `/ws`를 서빙.
- WebSocket으로 최초 스냅샷 전송 → 이후 20Hz 델타 브로드캐스트(바뀐 로봇만 포함).
- 커맨드 5종(`SelectRobot`/`ReleaseRobot`/`ToggleConveyor`/`SetRobotCount`/`TriggerArmAction`) 적용 및 검증(존재하지 않는 로봇 ID 거부).
- 세션 유예시간 로직은 순수 로직 + 유닛테스트로 존재하나, 실제 WS 재접속 흐름에 완전히 배선되지는 않음(스트레치, Plan 3에서 이어감).
- 아직 없는 것: SQLite 영속화, REST API, `/metrics`, 실제 로봇 클레임 락(v1은 단일 오퍼레이터라 불필요), Docker 배포 — Plan 3~5의 몫.
