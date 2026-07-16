# 로봇 내구도/고장/복구 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 로봇이 작업 중 마모를 누적해 확률적으로 고장나고, 오퍼레이터가 `RepairRobot` 커맨드로 고정 시간 동안 복구시키는 메커니즘을 추가한다 — 인프라 레벨 장애 격리(`safe_tick`/`tick_panics_total`)와 같은 "감지→조치→관측" 서사를 시뮬레이션 도메인에도 적용.

**Architecture:** `sim_core`(네트워크 의존성 없음)에 `RobotStatus`(Operational/Failed/Repairing) 필드와 `(robot_id, tick_count)` 시드 결정적 해시 기반 고장 판정을 추가하고, 서버 레이어(`game_state`/`protocol`/`ws`)에서 커맨드+와이어 배선을, 마지막으로 관측가능성 레이어(`metrics`/`persistence`/`main`)에서 메트릭+SQLite 이력+REST 엔드포인트를 배선한다.

**Tech Stack:** Rust, `rayon`(기존 병렬 틱), `serde`/`serde_json`(와이어), `rusqlite`(영속화), `prometheus`(메트릭), `proptest`(결정성 회귀).

**참고 문서:** `docs/superpowers/specs/2026-07-16-robot-durability-failure-design.md` (승인된 설계, 완결성 리뷰로 갭 수정 완료 — 이 계획은 그 스펙을 그대로 구현한다).

**작업 방식:** 이 저장소는 솔로 프로젝트라 워크트리 격리 없이 `main`에서 바로 작업한다(이 프로젝트의 확립된 관례).

---

### Task 1: `sim_core` — `RobotStatus`, 마모, 결정적 고장 판정, 이동 정지

**Files:**
- Modify: `server/src/sim.rs`

- [ ] **Step 1: `RobotStatus` enum과 `Robot` 필드 추가, 상수 추가**

`server/src/sim.rs` 상단부(기존 `const REPATH_INTERVAL`/`LEG_CYCLE_SPEED` 바로 아래)에 상수 추가:

```rust
const WEAR_LIMIT_TICKS: u64 = 2000; // 100초 분량의 작업(20Hz 기준) — 튜닝 대상
const MAX_FAILURE_PROB: f64 = 0.05; // 완전 마모 상태에서의 틱당 최대 고장 확률 — 튜닝 대상
pub const REPAIR_TICKS: u32 = 100; // 20Hz 기준 5초 — 튜닝 대상. game_state.rs가 RepairRobot 처리 시 이 값을 참조하므로 pub.
```

`Task` enum 바로 아래에 `RobotStatus` enum 추가:

```rust
/// 로봇의 동작 가능 여부. `task`(무슨 작업을 하려는 참인지)와는 별개다 —
/// `task`는 팔 동작만 나타내고 이동은 항상 자동이라, 고장으로 이동까지
/// 멈추려면 별도 필드가 필요하다. `Repairing` 중에도 `task`는 그대로
/// 보존되므로 복구가 끝나면 하던 일을 잊지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotStatus {
    Operational,
    Failed,
    Repairing { remaining_ticks: u32 },
}
```

`Robot` 구조체에 필드 추가(기존 필드 순서 유지, 끝에 추가):

```rust
#[derive(Debug, Clone)]
pub struct Robot {
    pub id: u32,
    pub pos: CellId,
    pub goal: CellId,
    pub path: Vec<CellId>,
    pub ticks_until_repath: u32,
    pub pose: BodyPose,
    pub leg_cycle_progress: f32,
    pub task: Task,
    pub worn_ticks: u64,
    pub status: RobotStatus,
}
```

`Robot::new`에 초기값 추가:

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
            worn_ticks: 0,
            status: RobotStatus::Operational,
        }
    }

    /// 0.0(방금 교체됨) ~ 1.0(완전 마모)의 마모 비율. 고장 확률 계산과
    /// 프로토콜의 `durability_remaining` 노출 양쪽이 이 함수 하나만
    /// 쓴다 — 계산식을 두 곳에 복사해두면 `WEAR_LIMIT_TICKS`를 나중에
    /// 튜닝할 때 한쪽만 고치고 잊어버리는 드리프트가 생기기 쉽다.
    pub fn wear_ratio(&self) -> f32 {
        (self.worn_ticks as f32 / WEAR_LIMIT_TICKS as f32).min(1.0)
    }
}
```

- [ ] **Step 2: 결정적 "난수"와 상태 전이 함수 추가**

`Robot`의 `impl` 블록 뒤, `SimState` 정의 앞에 추가:

```rust
/// (robot_id, tick_count)를 섞어 대략 [0.0, 1.0] 구간의 결정적 의사난수를
/// 낸다(u64 -> f64 변환의 부동소수점 반올림으로 극히 드물게 정확히 1.0이
/// 나올 수 있음 — `failure_prob`가 최대 `MAX_FAILURE_PROB`=0.05를 넘지
/// 않으므로 그 경우도 그냥 "고장 아님"으로 정확히 처리되어 문제없다).
/// splitmix64 파이널라이저를 재사용 — 암호학적 강도는 필요 없고, 입력이
/// 조금만 달라져도 출력이 크게 달라지는 성질(avalanche)만 있으면 된다.
/// 상태를 가진 RNG를 안 쓰는 이유: `tick()`이 `rayon`으로 로봇을 병렬
/// 갱신하며 각 로봇은 스냅샷만 읽는 무공유 모델이라, 상태 있는 RNG를
/// 넣으면 그 불변식이 깨진다(이 함수는 순수 함수라 안전).
fn deterministic_roll(robot_id: u32, tick_count: u64) -> f64 {
    let mut x = (robot_id as u64).wrapping_mul(0x9E3779B97F4A7C15) ^ tick_count.wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^= x >> 31;
    (x as f64) / (u64::MAX as f64)
}

/// 로봇의 마모/고장/복구 상태를 한 틱만큼 전진시킨다. 순수 함수(로봇을
/// 값으로 받아 값으로 반환) — `plan_robot`이 다른 순수 갱신 단계들과
/// 나란히 호출한다.
fn update_status(mut robot: Robot, tick_count: u64) -> Robot {
    match robot.status {
        RobotStatus::Operational => {
            if matches!(robot.task, Task::Picking | Task::Placing) {
                robot.worn_ticks += 1;
            }
            let failure_prob = robot.wear_ratio() as f64 * MAX_FAILURE_PROB;
            if deterministic_roll(robot.id, tick_count) < failure_prob {
                robot.status = RobotStatus::Failed;
            }
        }
        RobotStatus::Failed => {
            // RepairRobot 커맨드(game_state.rs)가 트리거하기 전까지는 가만히 있는다.
        }
        RobotStatus::Repairing { remaining_ticks } => {
            robot.status = if remaining_ticks <= 1 {
                robot.worn_ticks = 0;
                RobotStatus::Operational
            } else {
                RobotStatus::Repairing { remaining_ticks: remaining_ticks - 1 }
            };
        }
    }
    robot
}
```

- [ ] **Step 3: `plan_robot`/`safe_plan_robot`/`tick`에 배선 — 이동 정지**

`plan_robot` 함수를 교체(맨 앞에 `update_status` 호출 + 조기 리턴 추가, 시그니처에 `tick_count: u64` 추가):

```rust
fn plan_robot(grid: &Grid, robot: &Robot, occupied: &HashSet<CellId>, tick_count: u64) -> Robot {
    let mut next = update_status(robot.clone(), tick_count);

    if next.status != RobotStatus::Operational {
        // Failed/Repairing 로봇은 이동도, 재계획도 하지 않고 제자리에
        // 얼어붙는다. 다른 로봇들의 A*는 `occupied`(아래 tick() 참고)가
        // 매 틱 전체 로봇 위치로 다시 계산되므로, 이 로봇은 자동으로
        // 장애물 취급된다 — 그리드 쪽에 새 코드가 필요 없다.
        return next;
    }

    if next.pos == next.goal {
        return next;
    }

    if next.path.is_empty() || next.ticks_until_repath == 0 {
        let mut blocked = occupied.clone();
        blocked.remove(&next.pos);
        next.path = find_path(grid, next.pos, next.goal, &blocked).unwrap_or_default();
        next.ticks_until_repath = REPATH_INTERVAL;
    } else {
        next.ticks_until_repath -= 1;
    }

    if let Some(&next_cell) = next.path.first() {
        if !occupied.contains(&next_cell) {
            next.pos = next_cell;
            next.path.remove(0);
        }
    }

    next
}
```

`safe_plan_robot` 시그니처에 `tick_count: u64` 추가:

```rust
fn safe_plan_robot(grid: &Grid, robot: &Robot, occupied: &HashSet<CellId>, tick_count: u64) -> Robot {
    safe_call(robot, || plan_robot(grid, robot, occupied, tick_count))
}
```

`tick()`의 호출부 수정(`.map(...)` 클로저에 `state.tick_count` 추가):

```rust
    let planned: Vec<Robot> = state
        .robots
        .par_iter()
        .map(|robot| safe_plan_robot(&state.grid, robot, &occupied, state.tick_count))
        .collect();
```

- [ ] **Step 4: 단위테스트 추가**

`#[cfg(test)] mod tests` 블록 안, 기존 `new_robot_starts_idle` 테스트 뒤에 추가:

```rust
    #[test]
    fn new_robot_starts_operational_with_no_wear() {
        let robot = Robot::new(1, (0, 0), (0, 0));
        assert_eq!(robot.status, RobotStatus::Operational);
        assert_eq!(robot.worn_ticks, 0);
    }

    #[test]
    fn worn_ticks_accumulates_only_while_working() {
        let idle = update_status(Robot::new(1, (0, 0), (0, 0)), 0);
        assert_eq!(idle.worn_ticks, 0, "Idle robots should not wear");

        let mut working = Robot::new(2, (0, 0), (0, 0));
        working.task = Task::Picking;
        let working = update_status(working, 0);
        assert_eq!(working.worn_ticks, 1, "a working robot should wear by exactly one tick");
    }

    #[test]
    fn repairing_robot_does_not_accumulate_wear() {
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        robot.task = Task::Picking;
        robot.status = RobotStatus::Repairing { remaining_ticks: 5 };
        robot.worn_ticks = 10;

        let next = update_status(robot, 0);

        assert_eq!(next.worn_ticks, 10, "wear must not accumulate while repairing");
    }

    #[test]
    fn deterministic_roll_is_pure_and_repeatable() {
        let a = deterministic_roll(7, 1000);
        let b = deterministic_roll(7, 1000);
        assert_eq!(a, b);
    }

    #[test]
    fn deterministic_roll_stays_within_unit_interval() {
        for tick in 0..1000u64 {
            let roll = deterministic_roll(3, tick);
            assert!((0.0..=1.0).contains(&roll), "roll {roll} out of range at tick {tick}");
        }
    }

    #[test]
    fn deterministic_roll_is_roughly_uniformly_distributed() {
        let sum: f64 = (0..10_000u64).map(|tick| deterministic_roll(42, tick)).sum();
        let mean = sum / 10_000.0;
        assert!((0.4..0.6).contains(&mean), "mean {mean} is far from the expected ~0.5 for a uniform distribution");
    }

    #[test]
    fn fully_worn_robot_fails_at_roughly_max_failure_prob_rate() {
        // worn_ticks를 한계치로 박아두면 wear_ratio()==1.0,
        // failure_prob==MAX_FAILURE_PROB(0.05)로 고정된다 — 여러
        // tick_count에 대해 update_status를 반복 호출해 실제로 그 비율
        // 근처로 고장이 발생하는지 통계적으로 확인한다(정확히 5%일
        // 필요는 없고 자릿수만 맞으면 됨 — 결정적 해시라 매번 같은 결과).
        let mut failures = 0u32;
        let samples = 20_000u64;
        for tick in 0..samples {
            let mut robot = Robot::new(1, (0, 0), (0, 0));
            robot.task = Task::Picking;
            robot.worn_ticks = WEAR_LIMIT_TICKS;
            let next = update_status(robot, tick);
            if next.status == RobotStatus::Failed {
                failures += 1;
            }
        }
        let rate = failures as f64 / samples as f64;
        assert!((0.03..0.07).contains(&rate), "expected a failure rate near 0.05, got {rate}");
    }

    #[test]
    fn failed_robot_does_not_move_even_toward_an_unreached_goal() {
        let mut state = simple_state(5, 1);
        let mut robot = Robot::new(1, (0, 0), (3, 0));
        robot.status = RobotStatus::Failed;
        state.robots.push(robot);

        let next = tick(&state);

        assert_eq!(next.robots[0].pos, (0, 0), "a Failed robot must not move");
    }

    #[test]
    fn failed_robot_blocks_the_cell_for_other_robots() {
        let mut blocker = Robot::new(1, (1, 0), (2, 0)); // would move toward (2,0) if operational
        blocker.status = RobotStatus::Failed;
        let mover = Robot::new(2, (0, 0), (2, 0)); // only path to its goal runs through (1,0)
        let mut state = simple_state(3, 1);
        state.robots.push(blocker);
        state.robots.push(mover);

        let next = tick(&state);

        let blocker_after = next.robots.iter().find(|r| r.id == 1).unwrap();
        let mover_after = next.robots.iter().find(|r| r.id == 2).unwrap();
        assert_eq!(blocker_after.pos, (1, 0), "a Failed robot must never move");
        assert_eq!(mover_after.pos, (0, 0), "the mover cannot advance into the Failed robot's cell, its only path forward");
    }

    #[test]
    fn repairing_robot_counts_down_and_returns_to_operational() {
        let mut state = simple_state(3, 1);
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        robot.status = RobotStatus::Repairing { remaining_ticks: 2 };
        robot.worn_ticks = 500;
        state.robots.push(robot);

        let after_one = tick(&state);
        assert_eq!(after_one.robots[0].status, RobotStatus::Repairing { remaining_ticks: 1 });

        let after_two = tick(&after_one);
        assert_eq!(after_two.robots[0].status, RobotStatus::Operational);
        assert_eq!(after_two.robots[0].worn_ticks, 0, "worn_ticks should reset to 0 once repair completes");
    }
```

- [ ] **Step 5: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml --lib`
Expected: sim_core lib 테스트 전부 PASS (기존 35개 + 신규 10개 = 45개). 기존 테스트(`robot_moves_one_step_toward_goal_each_tick` 등)는 `tick()`의 공개 시그니처가 안 바뀌었으므로 그대로 통과해야 한다.

- [ ] **Step 6: Commit**

```bash
git add server/src/sim.rs
git commit -m "feat: add robot durability, deterministic failure rolls, and movement freeze on failure"
```

---

### Task 2: `game_state.rs` — 커맨드 검증(고장 로봇 작업 거부, 복구 커맨드)

**Files:**
- Modify: `server/src/game_state.rs`

- [ ] **Step 1: import 및 `CommandError` variant 추가**

`use sim_core::sim::{Robot, SimState, Task};`를 다음으로 교체:

```rust
use sim_core::sim::{Robot, RobotStatus, SimState, Task, REPAIR_TICKS};
```

`CommandError` enum 교체:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    RobotNotFound(u32),
    RobotNotOperational(u32),
    RobotNotFailed(u32),
}
```

- [ ] **Step 2: `trigger_arm_action` 가드 + `repair_robot` 추가**

`trigger_arm_action`을 교체:

```rust
    pub fn trigger_arm_action(&mut self, robot_id: u32, task: Task) -> Result<(), CommandError> {
        let robot = self
            .sim
            .robots
            .iter_mut()
            .find(|r| r.id == robot_id)
            .ok_or(CommandError::RobotNotFound(robot_id))?;
        if robot.status != RobotStatus::Operational {
            return Err(CommandError::RobotNotOperational(robot_id));
        }
        robot.task = task;
        Ok(())
    }

    /// 고장난(`Failed`) 로봇을 복구 시작 상태로 전이시킨다. `REPAIR_TICKS`
    /// 동안 `Repairing` 상태를 거친 뒤 `sim_core::sim::tick()`(Task 1에서
    /// 추가한 `update_status`)이 자동으로 `Operational`로 되돌리고
    /// `worn_ticks`를 리셋한다 — 이 함수는 그 카운트다운을 시작만 한다.
    pub fn repair_robot(&mut self, robot_id: u32) -> Result<(), CommandError> {
        let robot = self
            .sim
            .robots
            .iter_mut()
            .find(|r| r.id == robot_id)
            .ok_or(CommandError::RobotNotFound(robot_id))?;
        if robot.status != RobotStatus::Failed {
            return Err(CommandError::RobotNotFailed(robot_id));
        }
        robot.status = RobotStatus::Repairing { remaining_ticks: REPAIR_TICKS };
        Ok(())
    }
```

(`GameState` 구조체와 `impl` 블록의 나머지 메서드 (`select_robot`, `release_robot`, `set_robot_count` 등)는 그대로 둔다 — 스펙에서 확정한 대로 `SelectRobot`은 상태와 무관하게 계속 허용되고, `set_robot_count`는 상태 인지 없이 기존 최고-ID 제거 로직을 그대로 쓴다.)

- [ ] **Step 3: 단위테스트 추가**

`#[cfg(test)] mod tests` 블록 안, 기존 `trigger_arm_action_rejects_unknown_robot` 테스트 뒤에 추가:

```rust
    #[test]
    fn trigger_arm_action_rejects_non_operational_robot() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = state.sim.robots[0].id;
        state.sim.robots[0].status = RobotStatus::Failed;

        let err = state.trigger_arm_action(id, Task::Picking);

        assert_eq!(err, Err(CommandError::RobotNotOperational(id)));
    }

    #[test]
    fn repair_robot_transitions_a_failed_robot_to_repairing() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = state.sim.robots[0].id;
        state.sim.robots[0].status = RobotStatus::Failed;

        state.repair_robot(id).unwrap();

        assert_eq!(state.sim.robots[0].status, RobotStatus::Repairing { remaining_ticks: REPAIR_TICKS });
    }

    #[test]
    fn repair_robot_rejects_a_non_failed_robot() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = state.sim.robots[0].id;

        let err = state.repair_robot(id);

        assert_eq!(err, Err(CommandError::RobotNotFailed(id)));
    }

    #[test]
    fn repair_robot_rejects_unknown_robot() {
        let mut state = empty_state();
        let err = state.repair_robot(999);
        assert_eq!(err, Err(CommandError::RobotNotFound(999)));
    }

    #[test]
    fn select_robot_works_on_a_failed_robot() {
        // 스펙의 명시적 결정: 고장난 로봇도 선택은 계속 허용해야 오퍼레이터가
        // 상태를 보고 RepairRobot 대상으로 지정할 수 있다.
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = state.sim.robots[0].id;
        state.sim.robots[0].status = RobotStatus::Failed;

        state.select_robot(id).unwrap();

        assert_eq!(state.selected_robot, Some(id));
    }

    #[test]
    fn set_robot_count_shrink_removes_highest_id_even_if_it_is_repairing() {
        // 스펙의 명시적 v1 결정: 상태 인지 제거 우선순위는 두지 않는다 —
        // 복구 중인 로봇도 ID가 가장 크면 그대로 제거 대상이다.
        let mut state = empty_state();
        state.set_robot_count(2);
        let highest_id = state.sim.robots.iter().map(|r| r.id).max().unwrap();
        state
            .sim
            .robots
            .iter_mut()
            .find(|r| r.id == highest_id)
            .unwrap()
            .status = RobotStatus::Repairing { remaining_ticks: 10 };

        state.set_robot_count(1);

        assert!(
            !state.sim.robots.iter().any(|r| r.id == highest_id),
            "a Repairing robot is not special-cased during shrink"
        );
    }
```

- [ ] **Step 4: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml --bin server game_state`
Expected: `game_state` 모듈 테스트 전부 PASS (기존 9개 + 신규 6개 = 15개).

- [ ] **Step 5: Commit**

```bash
git add server/src/game_state.rs
git commit -m "feat: reject arm actions on non-operational robots, add RepairRobot command handler"
```

---

### Task 3: `protocol.rs` + `delta.rs` — 와이어 프로토콜(`WireStatus`, 내구도 노출, `RepairRobot`)

**Files:**
- Modify: `server/src/protocol.rs`
- Modify: `server/src/delta.rs` (기존 테스트 헬퍼가 `RobotView`의 신규 필드 때문에 컴파일이 깨지므로 같이 고친다)

- [ ] **Step 1: import 업데이트, `WireStatus` 추가**

`use sim_core::sim::{BodyPose, Robot, Task};`를 다음으로 교체:

```rust
use sim_core::sim::{BodyPose, Robot, RobotStatus, Task};
```

`WireTask`/`WirePose` 정의 뒤, `WireCellId` 앞에 `WireStatus` 추가:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum WireStatus {
    Operational,
    Failed,
    Repairing { remaining_ticks: u32 },
}

impl From<RobotStatus> for WireStatus {
    fn from(s: RobotStatus) -> WireStatus {
        match s {
            RobotStatus::Operational => WireStatus::Operational,
            RobotStatus::Failed => WireStatus::Failed,
            RobotStatus::Repairing { remaining_ticks } => WireStatus::Repairing { remaining_ticks },
        }
    }
}
```

- [ ] **Step 2: `RobotView`에 필드 추가 + 5% 반올림 헬퍼**

`RobotView` 구조체와 `From<&Robot>` impl을 교체:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RobotView {
    pub id: u32,
    pub pos: WireCellId,
    pub pose: WirePose,
    pub leg_cycle_progress: f32,
    pub task: WireTask,
    pub status: WireStatus,
    pub durability_remaining: f32,
}

impl From<&Robot> for RobotView {
    fn from(r: &Robot) -> RobotView {
        RobotView {
            id: r.id,
            pos: r.pos.into(),
            pose: r.pose.into(),
            leg_cycle_progress: r.leg_cycle_progress,
            task: r.task.into(),
            status: r.status.into(),
            durability_remaining: quantize_durability(r.wear_ratio()),
        }
    }
}

/// 델타 압축과의 상호작용(설계문서 참고) — 원값을 그대로 실으면 작업
/// 중인 로봇은 매 틱 델타에 실려서 "안 바뀌면 안 보낸다"는 대역폭 절약이
/// 무력화된다. 5% 단위로 반올림해서 값이 바뀌는 빈도를 낮춘다(약 100틱에
/// 한 번). `Repairing`의 `remaining_ticks`는 반대로 일부러 반올림하지
/// 않는다 — 진행률 표시로서의 가치가 크고, 최대 100틱(5초)으로 짧아서
/// 대역폭 비용이 무시할 만하다(설계문서 참고).
fn quantize_durability(wear_ratio: f32) -> f32 {
    let durability = 1.0 - wear_ratio;
    (durability * 20.0).round() / 20.0
}
```

- [ ] **Step 3: `ClientCommand`에 `RepairRobot` 추가**

`ClientCommand` enum의 `TriggerArmAction` variant 바로 뒤에 추가:

```rust
    RepairRobot { robot_id: u32 },
```

- [ ] **Step 4: `delta.rs`의 테스트 헬퍼 수정**

`server/src/delta.rs`의 `#[cfg(test)] mod tests` 블록에서 `use crate::protocol::{WireCellId, WireTask};`를 다음으로 교체:

```rust
    use crate::protocol::{WireCellId, WireStatus, WireTask};
```

`robot_view` 헬퍼 함수를 교체:

```rust
    fn robot_view(id: u32, x: i32) -> RobotView {
        RobotView {
            id,
            pos: WireCellId { x, y: 0 },
            pose: BodyPose::Standing.into(),
            leg_cycle_progress: 0.0,
            task: WireTask::Idle,
            status: WireStatus::Operational,
            durability_remaining: 1.0,
        }
    }
```

(이 파일의 나머지 테스트는 그대로 둔다 — `compute_delta`는 `RobotView` 전체를 `PartialEq`로 비교하므로 별도 로직 수정 없이 새 필드도 자동으로 델타 비교에 포함된다.)

- [ ] **Step 5: `protocol.rs`에 단위테스트 추가**

`#[cfg(test)] mod tests` 블록 안, 기존 `client_command_round_trips_through_json` 테스트 뒤에 추가:

```rust
    #[test]
    fn repair_robot_command_round_trips_through_json() {
        let cmd = ClientCommand::RepairRobot { robot_id: 5 };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: ClientCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(cmd, back);
    }

    #[test]
    fn robot_view_reports_operational_status_by_default() {
        use sim_core::sim::Robot;
        let robot = Robot::new(1, (0, 0), (0, 0));
        let view = RobotView::from(&robot);
        assert_eq!(view.status, WireStatus::Operational);
        assert_eq!(view.durability_remaining, 1.0);
    }

    #[test]
    fn robot_view_quantizes_durability_to_the_nearest_five_percent() {
        use sim_core::sim::Robot;
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        // wear_ratio = 1100/2000 = 0.55 -> raw durability_remaining 0.45,
        // 이미 5%의 배수라 반올림 여부와 무관하게 정확히 0.45가 나와야 한다.
        robot.worn_ticks = 1100;

        let view = RobotView::from(&robot);

        assert_eq!(view.durability_remaining, 0.45);
    }

    #[test]
    fn robot_view_reports_repairing_status_with_remaining_ticks() {
        use sim_core::sim::{Robot, RobotStatus};
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        robot.status = RobotStatus::Repairing { remaining_ticks: 42 };

        let view = RobotView::from(&robot);

        assert_eq!(view.status, WireStatus::Repairing { remaining_ticks: 42 });
    }
```

- [ ] **Step 6: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml --bin server`
Expected: `protocol`/`delta` 모듈 포함 서버 바이너리 유닛테스트 전부 PASS.

- [ ] **Step 7: Commit**

```bash
git add server/src/protocol.rs server/src/delta.rs
git commit -m "feat: expose robot status/durability on the wire, add RepairRobot command"
```

---

### Task 4: `ws.rs` — `RepairRobot` 커맨드 배선 + WS 통합테스트

**Files:**
- Modify: `server/src/ws.rs`
- Modify: `server/tests/ws_integration.rs`

- [ ] **Step 1: `apply_command`에 `RepairRobot` arm 추가**

`apply_command` 함수의 `TriggerArmAction` arm 바로 뒤에 추가:

```rust
        RepairRobot { robot_id } => {
            if let Err(err) = state.repair_robot(robot_id) {
                tracing::warn!(?err, "RepairRobot rejected");
            }
        }
```

- [ ] **Step 2: 통합테스트 추가**

`server/tests/ws_integration.rs`에 추가(파일 끝, 기존 마지막 테스트 뒤):

```rust
#[tokio::test]
async fn repair_robot_on_a_healthy_robot_is_rejected_without_crashing_the_connection() {
    // 실제로 로봇을 고장내서 성공 경로까지 테스트하지는 않는다 — 자연
    // 마모(2000틱=100초)+확률적 지연을 기다리는 건 느리고 취약한 테스트가
    // 된다(설계문서/Task 8의 교훈). 여기서는 거부 경로가 연결을 죽이지
    // 않는지만 실제 서버로 확인하고, 성공 경로는 game_state.rs의 결정적
    // 단위테스트가 이미 검증한다.
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
```

- [ ] **Step 3: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml --test ws_integration`
Expected: 기존 4개 + 신규 1개 = 5개 PASS.

- [ ] **Step 4: Commit**

```bash
git add server/src/ws.rs server/tests/ws_integration.rs
git commit -m "feat: wire RepairRobot command through apply_command"
```

---

### Task 5: `metrics.rs` — `robot_failures_total`/`robots_repairing`

**Files:**
- Modify: `server/src/metrics.rs`

- [ ] **Step 1: 필드 추가 + 등록**

`Metrics` 구조체에 필드 추가(`tick_duration_seconds` 뒤):

```rust
    /// 로봇이 Operational -> Failed로 전이할 때마다 증가 — 로봇 도메인
    /// 장애가 인프라 장애(tick_panics_total)와 같은 방식으로 관측
    /// 가능해지도록 하는 지표.
    pub robot_failures_total: IntCounter,
    /// 매 틱, 현재 Repairing 상태인 로봇 수로 갱신되는 게이지.
    pub robots_repairing: IntGauge,
```

`Metrics::new()` 안, `tick_duration_seconds` 등록 코드 뒤에 추가:

```rust
        let robot_failures_total = register_int_counter_with_registry!(
            "gamerobotfactory_robot_failures_total",
            "Total number of robot Operational -> Failed transitions",
            registry
        )
        .expect("registration only fails on a duplicate/invalid metric name; these 7 names are distinct and validly formed");
        let robots_repairing = register_int_gauge_with_registry!(
            "gamerobotfactory_robots_repairing",
            "Current number of robots in the Repairing state",
            registry
        )
        .expect("registration only fails on a duplicate/invalid metric name; these 7 names are distinct and validly formed");
```

기존 5개 `.expect(...)` 호출의 메시지도 "these 5 names"에서 "these 7 names"로 전부 갱신한다(파일 안 모든 등록 호출이 같은 문구를 써야 함).

`Metrics { ... }` 구조체 리터럴에 필드 추가:

```rust
        Metrics {
            registry,
            ticks_total,
            connected_clients,
            robot_count,
            tick_panics_total,
            tick_duration_seconds,
            robot_failures_total,
            robots_repairing,
        }
```

- [ ] **Step 2: 단위테스트 추가/갱신**

`fresh_metrics_encode_without_error_and_include_registered_names` 테스트에 두 줄 추가:

```rust
        assert!(text.contains("gamerobotfactory_robot_failures_total"));
        assert!(text.contains("gamerobotfactory_robots_repairing"));
```

새 테스트를 `observing_a_tick_duration_is_reflected_in_the_encoded_output` 뒤에 추가:

```rust
    #[test]
    fn robot_failure_metrics_are_registered_and_reflect_updates() {
        let metrics = Metrics::new();
        metrics.robot_failures_total.inc();
        metrics.robots_repairing.set(2);
        let (_, body) = metrics.encode();
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("gamerobotfactory_robot_failures_total 1"));
        assert!(text.contains("gamerobotfactory_robots_repairing 2"));
    }
```

- [ ] **Step 3: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml --bin server metrics`
Expected: 기존 3개 + 신규 1개 = 4개 PASS.

- [ ] **Step 4: Commit**

```bash
git add server/src/metrics.rs
git commit -m "feat: add robot_failures_total and robots_repairing metrics"
```

---

### Task 6: `persistence.rs` — `robot_failure_events` 테이블

**Files:**
- Modify: `server/src/persistence.rs`

- [ ] **Step 1: 스키마 추가**

`init_schema` 함수 안, 기존 `stats_history` 테이블 생성 뒤에 추가:

```rust
    conn.execute(
        "CREATE TABLE IF NOT EXISTS robot_failure_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tick INTEGER NOT NULL,
            robot_id INTEGER NOT NULL,
            event_type TEXT NOT NULL
        )",
        [],
    )?;
```

- [ ] **Step 2: `FailureEvent` + insert/read 함수 추가**

`StatsRow`/`insert_stats`/`recent_stats` 정의 뒤에 추가:

```rust
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FailureEvent {
    pub tick: u64,
    pub robot_id: u32,
    pub event_type: String,
}

pub fn insert_failure_event(conn: &Connection, tick: u64, robot_id: u32, event_type: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO robot_failure_events (tick, robot_id, event_type) VALUES (?1, ?2, ?3)",
        params![tick as i64, robot_id as i64, event_type],
    )?;
    Ok(())
}

pub fn recent_failure_events(conn: &Connection, limit: usize) -> Result<Vec<FailureEvent>> {
    let mut stmt = conn.prepare(
        "SELECT tick, robot_id, event_type FROM robot_failure_events ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(FailureEvent {
            tick: row.get::<_, i64>(0)? as u64,
            robot_id: row.get::<_, i64>(1)? as u32,
            event_type: row.get(2)?,
        })
    })?;
    rows.collect()
}
```

- [ ] **Step 3: 단위테스트 추가**

`#[cfg(test)] mod tests` 블록 안, 기존 마지막 테스트 뒤에 추가:

```rust
    #[test]
    fn insert_and_read_back_a_failure_event() {
        let conn = test_db();
        insert_failure_event(&conn, 42, 3, "failed").unwrap();

        let rows = recent_failure_events(&conn, 10).unwrap();

        assert_eq!(rows, vec![FailureEvent { tick: 42, robot_id: 3, event_type: "failed".to_string() }]);
    }

    #[test]
    fn recent_failure_events_returns_newest_first_and_respects_limit() {
        let conn = test_db();
        for tick in 0..5u64 {
            insert_failure_event(&conn, tick, 1, "failed").unwrap();
        }

        let rows = recent_failure_events(&conn, 2).unwrap();

        assert_eq!(rows.iter().map(|r| r.tick).collect::<Vec<_>>(), vec![4, 3]);
    }

    #[test]
    fn recent_failure_events_on_empty_db_returns_empty_vec() {
        let conn = test_db();
        let rows = recent_failure_events(&conn, 10).unwrap();
        assert!(rows.is_empty());
    }
```

- [ ] **Step 4: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml --bin server persistence`
Expected: 기존 3개 + 신규 3개 = 6개 PASS.

- [ ] **Step 5: Commit**

```bash
git add server/src/persistence.rs
git commit -m "feat: add robot_failure_events table and read/write functions"
```

---

### Task 7: `main.rs` — 전이 감지, 메트릭/영속화 배선, REST 엔드포인트

**Files:**
- Modify: `server/src/main.rs`
- Modify: `server/tests/rest_integration.rs`

- [ ] **Step 1: 순수 전이 감지 함수 추가**

`safe_tick` 함수 뒤, `spawn_tick_loop` 앞에 추가:

```rust
/// 이전 틱과 이번 틱의 로봇별 상태를 ID 기준으로 비교해, 로그해야 할 전이
/// (Operational -> Failed, Repairing -> Operational)를 찾아낸다. 순수
/// 함수로 분리해둔 이유: 실제 tick()/rayon/결정적 확률 굴림 없이도 이
/// 로직 자체를 빠르고 결정적으로 단위테스트할 수 있게 하기 위함 — 실제
/// 확률적 사건이 벽시계 안에서 일어나길 기다리는 통합테스트는 느리고
/// 취약해지기 쉽다(Task 8 Lagged 리싱크 사건과 같은 이유). `ws.rs`의
/// `decide_broadcast_update`와 같은 패턴.
fn detect_status_transitions(
    previous_robots: &[protocol::RobotView],
    current_robots: &[protocol::RobotView],
    tick: u64,
) -> Vec<persistence::FailureEvent> {
    let mut events = Vec::new();
    for current in current_robots {
        let Some(previous) = previous_robots.iter().find(|p| p.id == current.id) else { continue };
        match (previous.status, current.status) {
            (protocol::WireStatus::Operational, protocol::WireStatus::Failed) => {
                events.push(persistence::FailureEvent {
                    tick,
                    robot_id: current.id,
                    event_type: "failed".to_string(),
                });
            }
            (protocol::WireStatus::Repairing { .. }, protocol::WireStatus::Operational) => {
                events.push(persistence::FailureEvent {
                    tick,
                    robot_id: current.id,
                    event_type: "repaired".to_string(),
                });
            }
            _ => {}
        }
    }
    events
}
```

- [ ] **Step 2: `spawn_tick_loop`에 배선**

`spawn_tick_loop` 함수 전체를 교체:

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

            let tick_processing_start = std::time::Instant::now();
            let (message, next_snapshot, should_persist, stats_row, failure_events) = {
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
                metrics.robots_repairing.set(
                    guard
                        .sim
                        .robots
                        .iter()
                        .filter(|r| matches!(r.status, sim_core::sim::RobotStatus::Repairing { .. }))
                        .count() as i64,
                );

                let current_snapshot = to_snapshot(&guard, uuid::Uuid::nil());
                let (delta, failure_events) = match (&last_snapshot, &current_snapshot) {
                    (
                        protocol::ServerMessage::Snapshot { conveyor: prev_conveyor, robots: prev_robots, .. },
                        protocol::ServerMessage::Snapshot { tick: cur_tick, conveyor: cur_conveyor, robots: cur_robots, .. },
                    ) => {
                        let delta = compute_delta(*prev_conveyor, prev_robots, *cur_tick, *cur_conveyor, cur_robots);
                        let events = detect_status_transitions(prev_robots, cur_robots, *cur_tick);
                        (delta, events)
                    }
                    _ => (current_snapshot.clone(), Vec::new()),
                };

                for event in &failure_events {
                    if event.event_type == "failed" {
                        metrics.robot_failures_total.inc();
                    }
                }

                let should_persist = guard.sim.tick_count % persist_every_n_ticks == 0;
                let stats_row = persistence::StatsRow {
                    tick: guard.sim.tick_count,
                    robot_count: guard.sim.robots.len(),
                    conveyor_running: guard.conveyor.running,
                    total_production: total_production_value,
                };

                (delta, current_snapshot, should_persist, stats_row, failure_events)
            };
            metrics.tick_duration_seconds.observe(tick_processing_start.elapsed().as_secs_f64());

            last_snapshot = next_snapshot;
            let _ = broadcaster.send(message);

            if should_persist {
                let db = Arc::clone(&db);
                tokio::task::spawn_blocking(move || {
                    let conn = match db.lock() {
                        Ok(conn) => conn,
                        Err(err) => {
                            tracing::error!(%err, "db mutex poisoned; skipping this persist attempt");
                            return;
                        }
                    };
                    if let Err(err) = persistence::insert_stats(&conn, &stats_row) {
                        tracing::error!(%err, "failed to persist stats row");
                    }
                });
            }

            if !failure_events.is_empty() {
                let db = Arc::clone(&db);
                tokio::task::spawn_blocking(move || {
                    let conn = match db.lock() {
                        Ok(conn) => conn,
                        Err(err) => {
                            tracing::error!(%err, "db mutex poisoned; skipping failure event persist");
                            return;
                        }
                    };
                    for event in &failure_events {
                        if let Err(err) =
                            persistence::insert_failure_event(&conn, event.tick, event.robot_id, &event.event_type)
                        {
                            tracing::error!(%err, "failed to persist robot failure event");
                        }
                    }
                });
            }
        }
    });
}
```

- [ ] **Step 3: REST 엔드포인트 + `build_app` 등록**

`stats_history` 핸들러 함수 뒤에 추가:

```rust
async fn robot_failures(Extension(db): Extension<DbHandle>) -> impl IntoResponse {
    let rows = tokio::task::spawn_blocking(move || {
        let conn = db.lock().unwrap();
        persistence::recent_failure_events(&conn, 50)
    })
    .await;

    match rows {
        Ok(Ok(rows)) => axum::Json(rows).into_response(),
        Ok(Err(err)) => {
            tracing::error!(%err, "failed to read robot failure history");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
        }
        Err(join_err) => {
            tracing::error!(%join_err, "robot failure history query task panicked");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "internal error reading robot failure history".to_string())
                .into_response()
        }
    }
}
```

`build_app`에 라우트 추가(`/api/stats/history` 라우트 바로 뒤):

```rust
        .route("/api/robots/failures", get(robot_failures))
```

- [ ] **Step 4: `main.rs`에 단위테스트 추가**

`#[cfg(test)] mod tests` 블록 안, 기존 마지막 테스트 뒤에 추가:

```rust
    fn sample_robot_view(id: u32, status: protocol::WireStatus) -> protocol::RobotView {
        protocol::RobotView {
            id,
            pos: protocol::WireCellId { x: 0, y: 0 },
            pose: protocol::WirePose::Standing,
            leg_cycle_progress: 0.0,
            task: protocol::WireTask::Idle,
            status,
            durability_remaining: 1.0,
        }
    }

    #[test]
    fn detects_a_new_failure() {
        let previous = vec![sample_robot_view(1, protocol::WireStatus::Operational)];
        let current = vec![sample_robot_view(1, protocol::WireStatus::Failed)];

        let events = detect_status_transitions(&previous, &current, 42);

        assert_eq!(events, vec![persistence::FailureEvent { tick: 42, robot_id: 1, event_type: "failed".to_string() }]);
    }

    #[test]
    fn detects_a_completed_repair() {
        let previous = vec![sample_robot_view(1, protocol::WireStatus::Repairing { remaining_ticks: 1 })];
        let current = vec![sample_robot_view(1, protocol::WireStatus::Operational)];

        let events = detect_status_transitions(&previous, &current, 7);

        assert_eq!(events, vec![persistence::FailureEvent { tick: 7, robot_id: 1, event_type: "repaired".to_string() }]);
    }

    #[test]
    fn no_event_for_an_unrelated_change() {
        let previous = vec![sample_robot_view(1, protocol::WireStatus::Operational)];
        let current = vec![sample_robot_view(1, protocol::WireStatus::Operational)];

        let events = detect_status_transitions(&previous, &current, 1);

        assert!(events.is_empty());
    }

    #[test]
    fn no_event_for_a_removed_robot() {
        let previous = vec![sample_robot_view(1, protocol::WireStatus::Failed)];
        let current: Vec<protocol::RobotView> = vec![];

        let events = detect_status_transitions(&previous, &current, 1);

        assert!(events.is_empty(), "a removed robot should not generate a spurious event");
    }

    #[test]
    fn no_event_for_a_brand_new_robot() {
        let previous: Vec<protocol::RobotView> = vec![];
        let current = vec![sample_robot_view(5, protocol::WireStatus::Operational)];

        let events = detect_status_transitions(&previous, &current, 1);

        assert!(events.is_empty(), "a robot with no previous entry should not be treated as a transition");
    }
```

- [ ] **Step 5: `rest_integration.rs`에 통합테스트 추가**

`server/tests/rest_integration.rs` 파일 끝에 추가:

```rust
#[tokio::test]
async fn robot_failures_endpoint_returns_an_empty_list_when_nothing_has_failed() {
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
async fn metrics_endpoint_exposes_robot_failure_gauges_at_their_baseline() {
    let db_path = temp_db_path("robot-failure-metrics");
    let server = spawn_server_with_isolated_db(&db_path);
    let base = format!("http://127.0.0.1:{}", server.port);
    let client = reqwest::Client::new();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let response = client.get(format!("{base}/metrics")).send().await.expect("GET /metrics failed");
    let body = response.text().await.expect("failed to read metrics body");

    // 실제로 고장이 발생하는 걸 기다리는 건(자연 마모로 2000틱=100초 +
    // 확률적 지연) 이 테스트를 느리고 취약하게 만든다 — 대신 두 지표가
    // 노출되고 있고, 짧은 실행 동안 고장이 없었다는 정상적인 기저값(0)을
    // 보이는지만 확인한다. 값이 실제로 바뀌는 로직(detect_status_transitions)
    // 자체는 main.rs의 결정적 단위테스트가 이미 검증한다.
    assert!(body.contains("gamerobotfactory_robot_failures_total 0"));
    assert!(body.contains("gamerobotfactory_robots_repairing 0"));

    let _ = std::fs::remove_file(&db_path);
}
```

- [ ] **Step 6: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 전체 스위트 PASS(정확한 총계는 Task 9에서 확정 — 이 시점엔 기존 85개 + 이 태스크에서 추가한 단위/통합테스트가 전부 더해진 개수).

- [ ] **Step 7: Commit**

```bash
git add server/src/main.rs server/tests/rest_integration.rs
git commit -m "feat: wire robot failure/repair transitions into metrics, persistence, and a REST endpoint"
```

---

### Task 8: `tick_properties.rs` — 고장/복구 로봇이 섞인 상태에 대한 결정성 proptest

**Files:**
- Modify: `server/tests/tick_properties.rs`

- [ ] **Step 1: import 및 생성기 추가**

`use sim_core::sim::{tick, Robot, SimState};`를 다음으로 교체:

```rust
use sim_core::sim::{tick, Robot, RobotStatus, SimState};
```

기존 `arbitrary_sim_state` 함수 뒤에 추가:

```rust
/// 일부 로봇을 `Failed`/`Repairing`(제자리에 얼어붙은 장애물)로 시딩한다
/// — 이 기능이 도입한 "영구적으로 안 움직이는 로봇" 시나리오에서도
/// 충돌 없음/결정성이 유지되는지 검증하기 위함.
fn frozen_statuses() -> impl Strategy<Value = Vec<RobotStatus>> {
    proptest::collection::vec(
        prop_oneof![
            Just(RobotStatus::Operational),
            Just(RobotStatus::Failed),
            (1u32..=50).prop_map(|remaining_ticks| RobotStatus::Repairing { remaining_ticks }),
        ],
        ROBOT_COUNT,
    )
}

fn arbitrary_sim_state_with_some_frozen_robots() -> impl Strategy<Value = SimState> {
    (distinct_starts(), goals(), frozen_statuses()).prop_map(|(starts, goals, statuses)| {
        let robots: Vec<Robot> = starts
            .into_iter()
            .zip(goals)
            .zip(statuses)
            .enumerate()
            .map(|(i, ((pos, goal), status))| {
                let mut robot = Robot::new(i as u32, pos, goal);
                robot.status = status;
                robot
            })
            .collect();
        SimState { grid: Arc::new(Grid::new(SIZE, SIZE)), robots, tick_count: 0 }
    })
}
```

- [ ] **Step 2: proptest 추가**

기존 `proptest! { ... }` 블록의 마지막 테스트(`tick_is_deterministic`) 뒤에 추가(같은 `proptest!` 블록 안):

```rust
    /// Failed/Repairing 로봇이 섞여 있어도 충돌 방지 불변식이 유지된다.
    #[test]
    fn tick_never_produces_collisions_with_frozen_robots(state in arbitrary_sim_state_with_some_frozen_robots()) {
        let next = tick(&state);

        let mut seen = HashSet::new();
        for robot in &next.robots {
            prop_assert!(seen.insert(robot.pos), "duplicate position after tick: {:?}", robot.pos);
        }
    }

    /// Failed/Repairing 로봇이 섞여 있어도(마모/고장 로직이 결정적 해시를
    /// 쓰므로) tick()은 여전히 순수 함수여야 한다.
    #[test]
    fn tick_is_deterministic_with_frozen_robots(state in arbitrary_sim_state_with_some_frozen_robots()) {
        let a: Vec<(CellId, RobotStatus)> = tick(&state).robots.iter().map(|r| (r.pos, r.status)).collect();
        let b: Vec<(CellId, RobotStatus)> = tick(&state).robots.iter().map(|r| (r.pos, r.status)).collect();
        prop_assert_eq!(a, b);
    }

    /// Failed/Repairing으로 시딩된 로봇은 한 틱이 지나도 원래 칸에 그대로
    /// 있어야 한다 — Task 1의 예시 기반 단위테스트를 임의의 그리드/로봇
    /// 배치로 넓게 재확인한다.
    #[test]
    fn frozen_robots_never_move(state in arbitrary_sim_state_with_some_frozen_robots()) {
        let frozen_positions: std::collections::HashMap<u32, CellId> = state
            .robots
            .iter()
            .filter(|r| !matches!(r.status, RobotStatus::Operational))
            .map(|r| (r.id, r.pos))
            .collect();

        let next = tick(&state);

        for robot in &next.robots {
            if let Some(&original_pos) = frozen_positions.get(&robot.id) {
                prop_assert_eq!(robot.pos, original_pos, "a non-Operational robot must not move");
            }
        }
    }
```

- [ ] **Step 3: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml --test tick_properties`
Expected: 기존 2개 + 신규 3개 = 5개 PASS.

- [ ] **Step 4: Commit**

```bash
git add server/tests/tick_properties.rs
git commit -m "test: verify tick determinism and collision-freedom with frozen (Failed/Repairing) robots"
```

---

### Task 9: 전체 검증 + 문서 갱신

**Files:**
- Modify: `docs/KANBAN.md`
- Modify: `README.md`

- [ ] **Step 1: 전체 스위트 + clippy 확인**

Run: `cargo test --manifest-path server/Cargo.toml && cargo clippy --manifest-path server/Cargo.toml --all-targets`
Expected: 전부 PASS, 경고 0개. Task 1~8에서 추가한 테스트를 전부 합친 정확한 총계를 여기서 확정하고 기록한다(대략 기존 85개 + sim.rs 10개 + game_state.rs 6개 + protocol.rs 3개 + ws_integration.rs 1개 + metrics.rs 1개 + persistence.rs 3개 + main.rs 5개 + rest_integration.rs 2개 + tick_properties.rs 3개 ≈ 119개 — Task 9 실행 시 실제 숫자로 교정).

- [ ] **Step 2: 반복 실행으로 플레이키니스 확인**

Run: `cargo test --manifest-path server/Cargo.toml` (2~3회 반복)
Expected: 매번 동일하게 PASS. 특히 `fully_worn_robot_fails_at_roughly_max_failure_prob_rate`(통계적 단언)와 `deterministic_roll_is_roughly_uniformly_distributed`는 결정적 해시 함수라 입력이 고정되어 있으면 항상 같은 결과가 나와야 한다 — 반복 실행에서 다른 값이 나오면 버그다.

- [ ] **Step 3: `docs/KANBAN.md` 갱신**

`## Backlog`의 "로봇 내구도/고장/복구" 섹션을 `## Done`으로 옮기고, 각 태스크 완료와 커밋 SHA, 최종 테스트 총계를 기록한다(지금까지 이 파일에 기록해온 것과 같은 스타일 — Plan 3의 Task별 기록 참고).

- [ ] **Step 4: `README.md` 갱신**

"지금까지 만든 것" 섹션에 이 기능을 한 항목으로 추가(예: "로봇 내구도/고장/복구 — 작업 중 마모 누적 → 결정적 확률로 고장 → `RepairRobot` 커맨드로 복구, `robot_failures_total`/`robots_repairing` 메트릭 + SQLite 이력"), "프로토콜" 섹션의 커맨드/메시지 표에 `RepairRobot`/`status`/`durability_remaining` 추가, 테스트 통과 개수 갱신.

- [ ] **Step 5: Commit**

```bash
git add docs/KANBAN.md README.md
git commit -m "docs: mark robot durability/failure/repair feature complete in KANBAN.md and README.md"
```
