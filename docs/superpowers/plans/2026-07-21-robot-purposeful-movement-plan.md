# 목적있는 이동 (작업 사이클) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 컨베이어가 켜져 있으면 로봇이 자동으로 픽업 지점→배치 지점을 오가며 화물을 나르게 하고, 그 완료 여부를 실제 생산량 통계에 연결한다. 컨베이어가 꺼지면 기존 순찰로 되돌아간다.

**Architecture:** `sim_core::tick()`에 `conveyor_running: bool` 파라미터를 추가해 `plan_robot`이 순찰과 작업 사이클 중 어느 쪽을 따를지 결정적으로 판단하게 한다(사이드이펙트 없는 순수 함수 원칙 유지). 생산량 집계는 기존 `detect_status_transitions`와 같은 "이전/이후 스냅샷 비교 순수 함수" 패턴으로 처리한다. 와이어 프로토콜에 `carrying: bool` 필드 하나만 추가해 클라이언트가 운반 아이콘을 그리게 한다.

**Tech Stack:** Rust(`sim_core`/`server`), TypeScript(`client/`).

**참고 설계 문서:** [`docs/superpowers/specs/2026-07-21-robot-purposeful-movement-design.md`](../specs/2026-07-21-robot-purposeful-movement-design.md) — 이 문서는 완결성 리뷰에서 발견된 갭(작업 지점 충돌 버그, `TriggerArmAction` 악용 경로)을 이미 반영한 최종본이다.

---

## 사전 확인 사항 (구현자가 알아야 할 기존 코드 — 이 계획서 작성 시점 기준 정확한 라인)

- `server/src/sim.rs`: `Robot` 구조체(84-95행), `Task`/`RobotStatus` enum, `update_status`(143-167행, 마모/고장/복구 — 안 건드림), `patrol_points`/`next_patrol_goal`(240-255행 — **안 건드림**, 컨베이어 꺼짐일 때만 계속 쓰임), `plan_robot`(257-303행 — 이 태스크가 재작성하는 핵심), `safe_plan_robot`(307-309행), `tick`(187-234행), `deterministic_roll`(132-138행 — 재사용).
- `server/src/main.rs`: `safe_tick`(92-100행, `sim_core::sim::tick`를 패닉으로부터 격리하는 바이너리 크레이트 전용 래퍼 — `sim.rs`가 아니라 여기 있음), `detect_status_transitions`(109-136행 — 이 태스크가 추가할 `detect_completed_placements`의 모델), `spawn_tick_loop`의 생산량 집계 블록(177-181행 — 이 태스크가 교체하는 부분).
- `server/src/game_state.rs`: `Conveyor { running: bool }`(4-18행), `GameState.conveyor` 필드 — `tick()` 호출부는 `game_state.rs`가 아니라 `main.rs::spawn_tick_loop`에 있다(`guard: MutexGuard<GameState>`를 통해 `guard.sim`/`guard.conveyor.running`에 직접 접근).
- `server/src/protocol.rs`: `RobotView`(121-132행), `impl From<&Robot> for RobotView`(169-184행).
- `server/src/delta.rs`: 테스트 헬퍼 `robot_view()`(39-52행) — `RobotView` 리터럴을 만든다.
- 기존 `tick(&state)`/`safe_tick(&guard.sim)` 호출부는 정확히 3곳: `server/src/sim.rs`의 `#[cfg(test)] mod tests`(약 20회, 전부 인자가 `state` 아니면 `after_one`), `server/tests/tick_properties.rs`(proptest 5개, 일부는 2회씩 호출), `server/src/main.rs`(프로덕션 1회 + 테스트 1회).
- 클라이언트: `client/src/net/protocol.ts`의 `RobotView` 인터페이스(26-37행), `client/src/state/interpolation.ts`의 `InterpolatedRobot extends RobotView`(스프레드로 필드를 그대로 넘기므로 새 필드 추가 시 여기는 변경 불필요), `client/src/render/canvas.ts`의 `drawRobot`(로봇 외형 리디자인 플랜에서 이미 재작성됨 — 이 태스크는 그 함수 끝에 화물 아이콘만 추가).

---

## Task 1: sim_core 작업 사이클 + `conveyor_running` 시그니처 전파

**이 태스크가 끝나면 `cargo test --manifest-path server/Cargo.toml`(전체) 그린 상태여야 한다** — `tick()`/`safe_tick()`을 호출하는 모든 곳(라이브러리 테스트, `tests/tick_properties.rs`, `main.rs`)을 한 태스크 안에서 함께 고친다. 그렇지 않으면 이 태스크가 끝난 시점에 워크스페이스가 컴파일 자체가 안 되는 상태로 남는다.

**Files:**
- Modify: `server/src/sim.rs`
- Modify: `server/tests/tick_properties.rs`
- Modify: `server/src/main.rs` (`safe_tick` 시그니처 + 그 호출부만 — 생산량 집계 교체는 Task 2)

- [ ] **Step 1: `server/src/sim.rs` 상단에 새 상수 추가**

기존 상수 블록(`pub const REPAIR_TICKS: u32 = 100;` 바로 아래)에 추가:

```rust
pub const PICK_TICKS: u32 = 20; // 20Hz 기준 약 1초 — 튜닝 대상
pub const PLACE_TICKS: u32 = 20; // 20Hz 기준 약 1초 — 튜닝 대상
pub const UNIT_PER_CYCLE: f32 = 1.0; // 배치 1회 완료당 생산량 — main.rs가 참조
const PICKUP_SEED: u64 = 0;
const PLACE_SEED: u64 = 1;
```

- [ ] **Step 2: `Robot` 구조체에 필드 추가**

`pub struct Robot { ... }`(84-95행)에 다음 두 필드를 `pub facing: Direction,` 다음 줄에 추가:

```rust
    pub carrying: bool,
    pub work_ticks_remaining: u32,
```

`impl Robot { pub fn new(...) { Robot { ... } } }`(97-111행)의 리터럴에 `facing: Direction::East,` 다음 줄에 추가:

```rust
            carrying: false,
            work_ticks_remaining: 0,
```

- [ ] **Step 3: `work_points` 함수 추가**

`next_patrol_goal` 함수(250-255행) 바로 다음에 추가:

```rust
/// 작업 사이클의 픽업/배치 지점 — `patrol_points`와 달리 그리드 전체
/// (`w*h`칸)에 걸쳐 해시로 분산시킨다. `patrol_points`는 x/y를 각각
/// `id*7 mod w`/`id*3 mod h`로 독립 계산해서 정확히 w(보통 10)대마다
/// 좌표 쌍이 완전히 겹치는데(주기가 w에 불과), 순찰에서는 스쳐 지나가는
/// 통과점이라 무해했지만(로봇이 거기 머무르지 않음) 작업 사이클은 이
/// 지점에서 `PICK_TICKS`/`PLACE_TICKS` 동안 정지하므로 겹치면 다른
/// 로봇이 훨씬 오래 못 들어갈 수 있다(설계문서 §4). `deterministic_roll`
/// (마모/고장 판정에 쓰는 것과 같은 순수 해시 함수)을 재사용해 그리드
/// 전체 칸 하나를 인덱스로 뽑으면 주기가 w*h로 늘어나 충돌 확률이
/// 크게 줄어든다(완전히 없어지지는 않음 — 잔여 한계는 설계문서 §4 참고,
/// 의도적으로 받아들인 한계라 재시도 로직은 만들지 않는다).
fn work_points(id: u32, grid: &Grid) -> (CellId, CellId) {
    let w = grid.width.max(1);
    let h = grid.height.max(1);
    let cell_count = (w * h).max(1);
    let pickup_idx = (deterministic_roll(id, PICKUP_SEED) * cell_count as f64) as i32 % cell_count;
    let mut place_idx = (deterministic_roll(id, PLACE_SEED) * cell_count as f64) as i32 % cell_count;
    if place_idx == pickup_idx {
        place_idx = (place_idx + 1) % cell_count;
    }
    let pickup = (pickup_idx % w, pickup_idx / w);
    let place = (place_idx % w, place_idx / w);
    (pickup, place)
}
```

- [ ] **Step 4: 경로추종 로직을 `advance_along_path` 헬퍼로 추출**

`plan_robot` 함수(257-303행) 바로 다음에, 그 함수가 하던 "경로 찾기+한 칸 이동" 부분을 그대로 옮긴 새 헬퍼를 추가(순찰과 작업 사이클이 공유):

```rust
/// 목표(`next.goal`)를 향해 경로를 찾고 한 칸 이동을 시도한다 — 순찰과
/// 작업 사이클 모두 "그리드 위에서 목표까지 걸어간다"는 점은 같고
/// 목표를 무엇으로 삼을지만 다르므로, 이 로직은 공유한다.
fn advance_along_path(grid: &Grid, mut next: Robot, occupied: &HashSet<CellId>, tick_count: u64) -> Robot {
    if next.path.is_empty() || next.ticks_until_repath == 0 {
        let mut blocked = occupied.clone();
        blocked.remove(&next.pos);
        next.path = find_path(grid, next.pos, next.goal, &blocked).unwrap_or_default();
        next.ticks_until_repath = REPATH_INTERVAL;
    } else {
        next.ticks_until_repath -= 1;
    }

    // `u64::is_multiple_of`(clippy가 권하는 표현)는 Dockerfile이 고정한
    // `rust:1.85-bookworm`에서 아직 unstable이라 Docker 빌드가 깨진다 —
    // 이식성을 위해 `%`로 되돌린다.
    #[allow(clippy::manual_is_multiple_of)]
    if tick_count % PATROL_MOVE_INTERVAL_TICKS == 0 {
        if let Some(&next_cell) = next.path.first() {
            if !occupied.contains(&next_cell) {
                next.pos = next_cell;
                next.path.remove(0);
            }
        }
    }

    next
}
```

- [ ] **Step 5: `plan_robot`을 순찰/작업 사이클 분기로 재작성**

`plan_robot` 함수(257-303행) 전체를 아래로 교체(시그니처에 `conveyor_running: bool` 추가):

```rust
fn plan_robot(grid: &Grid, robot: &Robot, occupied: &HashSet<CellId>, tick_count: u64, conveyor_running: bool) -> Robot {
    let mut next = update_status(robot.clone(), tick_count);

    if next.status != RobotStatus::Operational {
        // Failed/Repairing 로봇은 이동도, 재계획도 하지 않고 제자리에
        // 얼어붙는다(기존 동작 그대로 — carrying/work_ticks_remaining도
        // task와 똑같이 여기서 안 건드려서 복구 후 하던 작업을 이어간다).
        return next;
    }

    if !conveyor_running {
        // 컨베이어가 꺼지면 진행 중이던 작업은 즉시 리셋한다 — 어중간한
        // 상태로 남지 않도록 하는 결정적 규칙(설계문서 §5).
        if next.task != Task::Idle || next.carrying || next.work_ticks_remaining > 0 {
            next.task = Task::Idle;
            next.carrying = false;
            next.work_ticks_remaining = 0;
        }
        if next.pos == next.goal {
            next.goal = next_patrol_goal(&next, grid);
        }
        return advance_along_path(grid, next, occupied, tick_count);
    }

    // 컨베이어 켜짐: 작업 사이클. `task`는 여기서 절대 입력으로 읽지
    // 않는다 — 오직 (carrying, work_ticks_remaining, pos)에서만
    // 파생시켜 매 틱 다시 써서, `TriggerArmAction`으로 수동 설정된
    // task가 카운트다운 없이 즉시 완료 처리되는 악용 경로를 원천
    // 차단한다(설계문서 §5-§6).
    if next.work_ticks_remaining > 0 {
        next.task = if next.carrying { Task::Placing } else { Task::Picking };
        next.work_ticks_remaining -= 1;
        if next.work_ticks_remaining == 0 {
            next.carrying = !next.carrying;
            next.task = Task::Idle;
        }
        return next; // 작업 중에는 이동하지 않는다
    }

    let (pickup, place) = work_points(next.id, grid);
    let target = if next.carrying { place } else { pickup };

    if next.pos != target {
        if next.goal != target {
            // 목표가 바뀌었다(예: 방금 화물을 집어 배치 지점으로 전환) —
            // 남은 경로/재계획 타이머를 지워야 낡은 경로를 계속 따라가는
            // 버그가 안 생긴다(sim.rs 기존 순찰 테스트가 이미 이 요구사항을
            // 증명함).
            next.goal = target;
            next.path.clear();
            next.ticks_until_repath = 0;
        }
        next.task = Task::Idle;
        return advance_along_path(grid, next, occupied, tick_count);
    }

    next.goal = target;
    next.task = if next.carrying { Task::Placing } else { Task::Picking };
    next.work_ticks_remaining = if next.carrying { PLACE_TICKS } else { PICK_TICKS };
    next
}
```

- [ ] **Step 6: `safe_plan_robot`/`tick` 시그니처에 `conveyor_running` 추가**

`safe_plan_robot`(307-309행)을 교체:

```rust
fn safe_plan_robot(grid: &Grid, robot: &Robot, occupied: &HashSet<CellId>, tick_count: u64, conveyor_running: bool) -> Robot {
    safe_call(robot, || plan_robot(grid, robot, occupied, tick_count, conveyor_running))
}
```

`tick` 함수(187행 시그니처, 193행 호출)를 교체:

```rust
pub fn tick(state: &SimState, conveyor_running: bool) -> SimState {
```

```rust
        .map(|robot| safe_plan_robot(&state.grid, robot, &occupied, state.tick_count, conveyor_running))
```

(`tick` 함수 나머지 본문은 그대로 — intents/resolve_intents/leg_cycle_progress/facing 갱신 로직은 `conveyor_running`과 무관하다.)

- [ ] **Step 7: 기존 `mod tests` 안의 모든 `tick(&...)` 호출에 `false` 추가**

`server/src/sim.rs`의 `#[cfg(test)] mod tests` 블록(359-717행) 안에서, `tick(&state)`로 호출하는 모든 곳(약 17회)을 `tick(&state, false)`로, `tick(&after_one)`(611행 한 곳)을 `tick(&after_one, false)`로 바꾼다. `false`를 쓰는 이유: 이 기존 테스트들은 전부 순찰/충돌/패닉격리/상태전이 동작을 검증하고, 작업 사이클(컨베이어 켜짐)과는 무관하기 때문이다.

중첩 호출 2곳(435-436행, `tick_is_deterministic_across_repeated_runs`)은 안쪽과 바깥쪽 둘 다 고쳐야 한다:

```rust
        let positions_a: Vec<CellId> = tick(&tick(&state, false), false).robots.iter().map(|r| r.pos).collect();
        let positions_b: Vec<CellId> = tick(&tick(&state, false), false).robots.iter().map(|r| r.pos).collect();
```

이 스텝이 끝나면 `cargo build --manifest-path server/Cargo.toml --lib` (라이브러리 타깃만)이 컴파일돼야 한다(Step 9까지는 `tests/tick_properties.rs`와 `main.rs`가 아직 옛 시그니처를 쓰고 있어 전체 `cargo test`는 실패하는 게 정상 — Step 8, 9에서 마저 고친다).

- [ ] **Step 8: 새 작업 사이클 테스트 추가**

`mod tests` 블록 끝(717행, `next_patrol_goal_alternates_between_the_two_patrol_points` 테스트 다음)에 추가:

```rust
    #[test]
    fn work_points_are_always_distinct_for_a_reasonably_sized_grid() {
        let grid = Grid::new(10, 10);
        for id in 0..50u32 {
            let (a, b) = work_points(id, &grid);
            assert_ne!(a, b, "work points must differ for id {id}");
        }
    }

    #[test]
    fn full_work_cycle_moves_to_pickup_picks_up_carries_and_places() {
        let grid = Arc::new(Grid::new(10, 10));
        let (pickup, place) = work_points(7, &grid);
        let mut state = SimState { grid: grid.clone(), robots: vec![Robot::new(7, pickup, pickup)], tick_count: 0 };

        // 이미 픽업 지점에 서 있는 상태로 시작 — 이동 없이 곧바로
        // Picking을 시작하는지부터 확인한다.
        state = tick(&state, true);
        assert_eq!(state.robots[0].task, Task::Picking);
        assert!(!state.robots[0].carrying);

        for _ in 0..PICK_TICKS {
            state = tick(&state, true);
        }
        assert!(state.robots[0].carrying, "PICK_TICKS번 틱이 지나면 화물을 들고 있어야 한다");
        assert_eq!(state.robots[0].task, Task::Idle);

        let mut arrived = false;
        for _ in 0..500 {
            state = tick(&state, true);
            if state.robots[0].pos == place {
                arrived = true;
                break;
            }
        }
        assert!(arrived, "carrying 로봇은 결국 배치 지점에 도착해야 한다");

        state = tick(&state, true);
        assert_eq!(state.robots[0].task, Task::Placing);

        for _ in 0..PLACE_TICKS {
            state = tick(&state, true);
        }
        assert!(!state.robots[0].carrying, "PLACE_TICKS번 틱 후에는 화물을 내려놓아야 한다");
        assert_eq!(state.robots[0].task, Task::Idle);
    }

    #[test]
    fn turning_conveyor_off_mid_work_resets_task_and_carrying_immediately() {
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        robot.task = Task::Picking;
        robot.work_ticks_remaining = 5;
        robot.carrying = true;
        let grid = Grid::new(5, 5);
        let occupied: HashSet<CellId> = HashSet::new();

        let next = plan_robot(&grid, &robot, &occupied, 0, false);

        assert_eq!(next.task, Task::Idle);
        assert!(!next.carrying);
        assert_eq!(next.work_ticks_remaining, 0);
    }

    #[test]
    fn manual_trigger_arm_action_cannot_skip_the_work_cycle_wait() {
        // TriggerArmAction(game_state.rs) 커맨드는 robot.task만 직접
        // 쓴다 — work_ticks_remaining은 안 건드린다. 작업 사이클 로직이
        // task를 절대 입력으로 읽지 않고 (carrying, work_ticks_remaining,
        // pos)에서만 파생시키므로, 이렇게 수동으로 끼워넣은 task는 다음
        // 틱에 무조건 덮어써지고 carrying이 즉시 반전되면 안 된다
        // (설계문서 §5-§6 — 이 테스트가 막는 악용 경로).
        let grid = Grid::new(10, 10);
        let (pickup, _place) = work_points(1, &grid);
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        robot.task = Task::Picking; // 오퍼레이터가 수동으로 끼워넣은 값
        // work_ticks_remaining은 기본값 0 그대로

        let occupied: HashSet<CellId> = HashSet::new();
        let next = plan_robot(&grid, &robot, &occupied, 0, true);

        assert!(!next.carrying, "manually-set Picking task must not instantly complete without the auto cycle's own countdown");
        if next.pos != pickup {
            assert_eq!(next.task, Task::Idle, "auto cycle should overwrite the manual task while still transiting to the pickup point");
        }
    }
```

- [ ] **Step 9: `server/tests/tick_properties.rs`에 `conveyor_running` proptest 입력 추가**

파일 상단 import에 `use proptest::prelude::*;` 다음 줄쯤에 이미 있는 것 재사용(추가 import 불필요). 5개 프로퍼티 테스트 전부에 `conveyor_running: bool` 인자를 추가하고, `tick(&state)` 호출을 `tick(&state, conveyor_running)`으로 바꾼다:

```rust
    #[test]
    fn tick_never_produces_collisions(state in arbitrary_sim_state(), conveyor_running: bool) {
        let next = tick(&state, conveyor_running);

        let mut seen = HashSet::new();
        for robot in &next.robots {
            prop_assert!(seen.insert(robot.pos), "duplicate position after tick: {:?}", robot.pos);
        }
    }

    #[test]
    fn tick_is_deterministic(state in arbitrary_sim_state(), conveyor_running: bool) {
        let positions_a: Vec<CellId> = tick(&state, conveyor_running).robots.iter().map(|r| r.pos).collect();
        let positions_b: Vec<CellId> = tick(&state, conveyor_running).robots.iter().map(|r| r.pos).collect();
        prop_assert_eq!(positions_a, positions_b);
    }

    #[test]
    fn tick_never_produces_collisions_with_frozen_robots(state in arbitrary_sim_state_with_some_frozen_robots(), conveyor_running: bool) {
        let next = tick(&state, conveyor_running);

        let mut seen = HashSet::new();
        for robot in &next.robots {
            prop_assert!(seen.insert(robot.pos), "duplicate position after tick: {:?}", robot.pos);
        }
    }

    #[test]
    fn tick_is_deterministic_with_frozen_robots(state in arbitrary_sim_state_with_some_frozen_robots(), conveyor_running: bool) {
        let a: Vec<(CellId, RobotStatus)> = tick(&state, conveyor_running).robots.iter().map(|r| (r.pos, r.status)).collect();
        let b: Vec<(CellId, RobotStatus)> = tick(&state, conveyor_running).robots.iter().map(|r| (r.pos, r.status)).collect();
        prop_assert_eq!(a, b);
    }

    #[test]
    fn frozen_robots_never_move(state in arbitrary_sim_state_with_some_frozen_robots(), conveyor_running: bool) {
        let frozen_positions: std::collections::HashMap<u32, CellId> = state
            .robots
            .iter()
            .filter(|r| {
                matches!(r.status, RobotStatus::Failed)
                    || matches!(r.status, RobotStatus::Repairing { remaining_ticks } if remaining_ticks > 1)
            })
            .map(|r| (r.id, r.pos))
            .collect();

        let next = tick(&state, conveyor_running);

        for robot in &next.robots {
            if let Some(&original_pos) = frozen_positions.get(&robot.id) {
                prop_assert_eq!(robot.pos, original_pos, "a non-Operational robot must not move");
            }
        }
    }
```

(각 함수 시그니처의 `state in arbitrary_sim_state()` 다음에 `, conveyor_running: bool`을 추가하는 것뿐 — proptest는 별도 전략 함수 없이 `bool` 타입에 대해 자동으로 `any::<bool>()` 전략을 쓴다.)

- [ ] **Step 10: `server/src/main.rs`의 `safe_tick` 시그니처와 호출부 갱신**

`safe_tick` 함수(92-100행)를 교체:

```rust
fn safe_tick(sim: &SimState, conveyor_running: bool) -> Option<SimState> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick(sim, conveyor_running))) {
        Ok(next) => Some(next),
        Err(_) => {
            tracing::error!("tick() panicked; skipping this tick, simulation state unchanged");
            None
        }
    }
}
```

프로덕션 호출부(172행)를 교체:

```rust
                match safe_tick(&guard.sim, guard.conveyor.running) {
```

`main.rs`의 `safe_tick` 테스트(357-366행 부근, `safe_tick_passes_through_normal_ticks_unchanged`)의 호출부(359행)를 교체:

```rust
        let result = safe_tick(&sim, false);
```

- [ ] **Step 11: 전체 서버 테스트 확인**

Run:
```bash
cargo test --manifest-path server/Cargo.toml
cargo clippy --manifest-path server/Cargo.toml --all-targets
```
Expected: 전부 PASS. (`sim.rs:285`의 `unknown lint` 경고는 이 플랜과 무관한 기존 이슈이므로 무시 — 로봇 외형 리디자인 플랜 Task 4에서 이미 확인·기록됨.)

- [ ] **Step 12: 뮤테이션 테스트 — `manual_trigger_arm_action_cannot_skip_the_work_cycle_wait`이 실제로 악용 경로를 잡는지 확인**

`plan_robot`의 컨베이어 켜짐 분기 맨 앞에 임시로 다음을 추가(악용 경로를 일부러 되살림):

```rust
    if conveyor_running && next.task == Task::Picking && next.work_ticks_remaining == 0 {
        next.carrying = true; // 악용 경로 재현용 임시 코드 — 되돌릴 것
    }
```

`cargo test --manifest-path server/Cargo.toml manual_trigger_arm_action`을 실행해 이 테스트가 실제로 실패하는지 확인한 뒤, 임시 코드를 제거하고 다시 통과하는지 확인한다.

- [ ] **Step 13: 커밋**

```bash
git add server/src/sim.rs server/tests/tick_properties.rs server/src/main.rs
git commit -m "feat: add conveyor-gated pick/carry/place work cycle to sim_core"
```

---

## Task 2: 생산량 집계를 실제 배치 완료에 연결

**Files:**
- Modify: `server/src/main.rs`
- Test: `server/tests/rest_integration.rs`

- [ ] **Step 1: `detect_completed_placements` 순수 함수 추가**

`server/src/main.rs`의 `detect_status_transitions` 함수(109-136행) 바로 다음에 추가:

```rust
/// 이전 틱과 이번 틱의 로봇별 `carrying` 값을 ID 기준으로 비교해, 방금
/// 배치를 완료한(carrying: true -> false) 로봇 ID를 찾아낸다.
/// `detect_status_transitions`와 같은 이유(실제 tick()/작업 사이클
/// 타이밍 없이도 결정적으로 단위테스트하기 위함)로 순수 함수로 분리했다.
fn detect_completed_placements(previous_robots: &[protocol::RobotView], current_robots: &[protocol::RobotView]) -> Vec<u32> {
    let mut completed = Vec::new();
    for current in current_robots {
        let Some(previous) = previous_robots.iter().find(|p| p.id == current.id) else { continue };
        if previous.carrying && !current.carrying {
            completed.push(current.id);
        }
    }
    completed
}
```

- [ ] **Step 2: 생산량 집계를 배치 완료 이벤트 기반으로 교체**

`spawn_tick_loop` 함수 안(158-227행)의 아래 블록을 통째로 교체한다. 기존(177-181행, 181행 뒤로 다른 코드 이어짐):

```rust
                let mut total_production_value = 0.0_f32;
                if guard.conveyor.running {
                    let units: HashMap<u32, f32> = guard.sim.robots.iter().map(|r| (r.id, 0.01)).collect();
                    total_production_value = total_production(&guard.sim.robots, &units);
                }

                metrics.ticks_total.inc();
```

새 코드(`total_production_value` 관련 블록 삭제, `metrics.ticks_total.inc();`부터는 그대로 유지):

```rust
                metrics.ticks_total.inc();
```

그리고 그 아래 있는 `let (delta, failure_events) = match (&last_snapshot, &current_snapshot) { ... }` 블록(195-205행)을 교체해서 `total_production_value`도 같은 매치 안에서 계산하게 한다:

```rust
                let current_snapshot = to_snapshot(&guard, uuid::Uuid::nil());
                let (delta, failure_events, total_production_value) = match (&last_snapshot, &current_snapshot) {
                    (
                        protocol::ServerMessage::Snapshot { conveyor: prev_conveyor, robots: prev_robots, .. },
                        protocol::ServerMessage::Snapshot { tick: cur_tick, conveyor: cur_conveyor, robots: cur_robots, .. },
                    ) => {
                        let delta = compute_delta(*prev_conveyor, prev_robots, *cur_tick, *cur_conveyor, cur_robots);
                        let events = detect_status_transitions(prev_robots, cur_robots, *cur_tick);
                        let completed = detect_completed_placements(prev_robots, cur_robots);
                        let units: HashMap<u32, f32> = completed.iter().map(|&id| (id, sim_core::sim::UNIT_PER_CYCLE)).collect();
                        let production = total_production(&guard.sim.robots, &units);
                        (delta, events, production)
                    }
                    _ => (current_snapshot.clone(), Vec::new(), 0.0_f32),
                };
```

(`current_snapshot`을 만드는 줄이 이미 저 위치에 있었으므로, 겹치지 않도록 기존 `let current_snapshot = to_snapshot(&guard, uuid::Uuid::nil());` 한 줄과 그 다음 `match` 블록을 통째로 위 코드로 대체한다.)

이 변경으로 "컨베이어가 켜져 있으면 로봇 수만큼 생산량이 오른다"에서 "실제로 배치를 완료한 로봇 수만큼 생산량이 오른다"로 바뀐다.

- [ ] **Step 3: 타입체크 겸 컴파일 확인**

Run: `cargo build --manifest-path server/Cargo.toml`
Expected: 에러 없음. (`HashMap` import는 이미 `main.rs` 상단에 있음 — 안 지웠는지만 확인.)

- [ ] **Step 4: REST 통합테스트 추가 — 실제로 생산량이 오르는지 확인**

`server/tests/rest_integration.rs`의 `stats_history_reflects_persisted_rows_after_running` 테스트(103-130행) 다음에 추가:

```rust
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
    // PLACE_TICKS(20)를 감안해 넉넉히 6초 기다린다(그리드 10x10 최대
    // 이동거리 기준 최악의 경우에도 20Hz에서 여유 있게 끝난다).
    // 실제 로봇을 스폰하려면 WS로 SetRobotCount를 보내야 하므로 여기서는
    // REST가 아니라 WS 클라이언트를 잠깐 쓴다.
    let (ws_stream, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}/ws", server.port))
        .await
        .expect("failed to connect WS");
    use futures_util::{SinkExt, StreamExt};
    let (mut write, _read) = ws_stream.split();
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({ "type": "SetRobotCount", "count": 3 }).to_string().into(),
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

    let _ = std::fs::remove_file(&db_path);
}
```

`tokio-tungstenite`/`futures-util`은 이미 `server/Cargo.toml`에 있다(`server/tests/ws_integration.rs`가 이미 같은 크레이트로 WS 클라이언트를 만드는 데 쓰고 있다 — `use futures_util::{SinkExt, StreamExt};`/`tokio_tungstenite::connect_async` 패턴을 그대로 따라간 것). 추가 설치 불필요.

- [ ] **Step 5: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml production_only_increases`
Expected: PASS (6초 정도 소요 — 실시간 대기가 있는 통합테스트라 원래 느림, 기존 `stats_history_reflects_persisted_rows_after_running`도 같은 이유로 0.5초 대기함).

- [ ] **Step 6: 뮤테이션 테스트**

`detect_completed_placements`의 `if previous.carrying && !current.carrying` 조건을 임시로 `if true`로 바꿔서(모든 로봇을 매 틱 "방금 완료"로 오인하게 만듦) `cargo test --manifest-path server/Cargo.toml`을 돌려보고, 위 REST 테스트나 기존 `detect_status_transitions`류 테스트 중 하나가 실제로 이상 동작(생산량이 비정상적으로 빨리 치솟거나 관련 단위테스트 실패)을 보이는지 확인한다. 확인 후 원래 조건으로 되돌리고 다시 통과 확인.

- [ ] **Step 7: 커밋**

```bash
git add server/src/main.rs server/tests/rest_integration.rs
git commit -m "feat: tie production stats to actual placement completion instead of flat conveyor-on rate"
```

---

## Task 3: 와이어 프로토콜에 `carrying` 필드 추가

**Files:**
- Modify: `server/src/protocol.rs`
- Modify: `server/src/delta.rs`
- Modify: `server/src/main.rs` (테스트 헬퍼만)
- Test: `server/tests/ws_integration.rs`

- [ ] **Step 1: `RobotView`에 필드 추가**

`server/src/protocol.rs`의 `RobotView` 구조체(121-132행)에 `pub arm_pose: WireArmPose,` 다음 줄에 추가:

```rust
    pub carrying: bool,
```

`impl From<&Robot> for RobotView`(169-184행)의 리터럴에 `arm_pose: arm_pose_for(r),` 다음 줄에 추가:

```rust
            carrying: r.carrying,
```

- [ ] **Step 2: 테스트 헬퍼 2곳에 필드 추가**

`server/src/delta.rs`의 `robot_view()` 헬퍼(39-52행) 리터럴에 `arm_pose: WireArmPose { shoulder_angle: 0.0, elbow_angle: 0.0 },` 다음 줄에 추가:

```rust
            carrying: false,
```

`server/src/main.rs`의 `sample_robot_view()` 헬퍼(376-389행) 리터럴에도 같은 위치에 추가:

```rust
            carrying: false,
```

- [ ] **Step 3: 컴파일 확인**

Run: `cargo build --manifest-path server/Cargo.toml --all-targets`
Expected: 에러 없음. `compute_delta`(전체 구조체 `PartialEq` 비교)는 코드 변경 없이 새 필드를 자동으로 인식한다.

- [ ] **Step 4: WS 통합테스트 추가**

`server/tests/ws_integration.rs`를 열어 기존 테스트 하나(예: `RepairRobot` 관련 테스트)의 구조를 참고해서, 로봇을 1대 스폰하고 컨베이어를 켠 뒤 몇 초 기다려 `changed_robots`의 `carrying` 필드가 최소 한 번은 `true`로 바뀌는 델타를 받는지 확인하는 테스트를 추가한다. 정확한 헬퍼 함수명/커넥션 설정 방식은 그 파일의 기존 테스트를 그대로 따라 작성하되, 핵심 단언은:

```rust
    // ... 기존 파일의 커넥션 셋업 방식 재사용 ...
    // 6초 동안 델타를 계속 읽으면서 carrying: true가 한 번이라도 오는지 확인
    let mut saw_carrying = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(200), read.next()).await {
            let msg = msg.expect("WS read failed");
            if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                let parsed: serde_json::Value = serde_json::from_str(&text).expect("invalid JSON");
                if let Some(robots) = parsed["changed_robots"].as_array() {
                    if robots.iter().any(|r| r["carrying"] == serde_json::json!(true)) {
                        saw_carrying = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(saw_carrying, "expected at least one robot to report carrying:true within 6 seconds");
```

- [ ] **Step 5: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml --test ws_integration`
Expected: PASS.

- [ ] **Step 6: 커밋**

```bash
git add server/src/protocol.rs server/src/delta.rs server/src/main.rs server/tests/ws_integration.rs
git commit -m "feat: expose carrying on the wire protocol for cargo-visualization"
```

---

## Task 4: 클라이언트 — 화물 아이콘 렌더링

**Files:**
- Modify: `client/src/net/protocol.ts`
- Modify: `client/src/render/canvas.ts`
- Modify: `client/tests/unit/canvas.test.ts`
- Modify: `client/tests/unit/sidebar.test.ts`
- Modify: `client/tests/unit/interpolation.test.ts`
- Modify: `client/tests/unit/mirror.test.ts`

- [ ] **Step 1: 타입 추가**

`client/src/net/protocol.ts`의 `RobotView` 인터페이스(26-37행)에 `arm_pose: WireArmPose` 다음 줄에 추가:

```typescript
  carrying: boolean
```

- [ ] **Step 2: 타입체크로 깨지는 테스트 헬퍼 찾기**

Run: `cd client && npm run typecheck`
Expected: `carrying` 속성이 없다는 에러가 4개 파일(`canvas.test.ts`, `sidebar.test.ts`, `interpolation.test.ts`, `mirror.test.ts`)에서 난다 — 전부 `RobotView`/`InterpolatedRobot` 모양의 테스트 픽스처를 만드는 헬퍼 함수다.

- [ ] **Step 3: 각 헬퍼에 `carrying: false` 추가**

각 파일에서 `durability_remaining: 1` (또는 유사한 필드) 근처에 `RobotView` 객체 리터럴을 만드는 헬퍼 함수를 찾아 `carrying: false,`를 추가한다(정확한 헬퍼 함수명과 위치는 파일마다 다르므로 `npm run typecheck`가 알려주는 파일:줄번호를 그대로 따라간다). `client/tests/unit/canvas.test.ts`의 경우 `robotAt()` 헬퍼(5-19행)에 추가.

- [ ] **Step 4: 타입체크 재확인**

Run: `cd client && npm run typecheck`
Expected: 에러 없음.

- [ ] **Step 5: `drawRobot`에 화물 아이콘 추가**

`client/src/render/canvas.ts`의 `drawRobot` 함수(로봇 외형 리디자인 플랜에서 이미 재작성된 버전) 안에서, 팔을 그리는 `ctx.stroke()` 호출 직후, `ctx.restore()` 직전에 추가:

```typescript
  if (robot.carrying) {
    const cargoX = shoulderX + elbowDx + (wristDx - elbowDx)
    const cargoY = shoulderY + elbowDy + (wristDy - elbowDy)
    ctx.fillStyle = '#c9762f'
    ctx.strokeStyle = '#1c2024'
    ctx.lineWidth = 1.5
    ctx.fillRect(cargoX - 5, cargoY - 5, 10, 9)
    ctx.strokeRect(cargoX - 5, cargoY - 5, 10, 9)
  }
```

(`shoulderX`/`shoulderY`/`elbowDx`/`elbowDy`/`wristDx`/`wristDy`는 이미 그 함수 안에 정의돼 있는 변수를 그대로 재사용 — 손목 위치, 즉 그리퍼 끝에 화물 박스를 그린다.)

- [ ] **Step 6: 타입체크 + 단위테스트**

Run: `cd client && npm run typecheck && npm test`
Expected: 에러 없음, 전부 PASS.

- [ ] **Step 7: `carrying`이 있을 때만 그려지는지 확인하는 단위테스트 추가**

`drawRobot`은 캔버스 부수효과 함수라 직접 단위테스트가 없다(기존 관례와 동일 — 로봇 외형 리디자인 플랜의 `drawRobot` 재작성 때도 새 유닛테스트는 추가하지 않고 Playwright E2E로 대신 검증했다). 대신 `client/tests/e2e/render.spec.ts`에 다음 테스트를 추가한다(파일 상단의 기존 헬퍼 `backendPort()`/`currentRobotCount()` 재사용):

```typescript
  test('renders a cargo icon on a robot after it completes a pickup', async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 })
    await page.goto(`/?ws=ws://127.0.0.1:${backendPort()}/ws`)

    const before = await currentRobotCount(page)
    const incButton = page.locator('.sidebar button', { hasText: '+' })
    await incButton.click()
    await expect(page.locator('.robot-count')).toHaveText(String(before + 1), { timeout: 5000 })

    // 컨베이어는 서버 기본값으로 이미 켜져 있다 — 작업 사이클이 자동으로
    // 시작돼 픽업+PICK_TICKS를 거쳐 화물을 들 때까지 기다린다(최악의
    // 이동 거리 + 20틱 카운트다운 감안 6초).
    const cargoColor = { r: 0xc9, g: 0x76, b: 0x2f }
    let found = false
    for (let attempt = 0; attempt < 30 && !found; attempt++) {
      found = await page.evaluate((color) => {
        const c = document.querySelector('canvas') as HTMLCanvasElement
        const ctx = c.getContext('2d')!
        const { width, height } = c
        const data = ctx.getImageData(0, 0, width, height).data
        for (let i = 0; i < data.length; i += 4) {
          if (Math.abs(data[i] - color.r) < 10 && Math.abs(data[i + 1] - color.g) < 10 && Math.abs(data[i + 2] - color.b) < 10) {
            return true
          }
        }
        return false
      }, cargoColor)
      if (!found) await page.waitForTimeout(200)
    }
    expect(found).toBe(true)
  })
```

- [ ] **Step 8: E2E 테스트 실행**

Run: `cd client && npm run build && npx playwright test render.spec.ts`
Expected: PASS (기존 테스트 포함 전부 그린, 새 테스트는 최대 6초 정도 소요될 수 있음).

- [ ] **Step 9: 커밋**

```bash
git add client/src/net/protocol.ts client/src/render/canvas.ts client/tests/unit/canvas.test.ts client/tests/unit/sidebar.test.ts client/tests/unit/interpolation.test.ts client/tests/unit/mirror.test.ts client/tests/e2e/render.spec.ts
git commit -m "feat: render a cargo icon on robots carrying an item"
```

---

## Task 5: 전체 검증 + 문서 갱신

**Files:**
- Modify: `README.md`
- Modify: `docs/KANBAN.md`

- [ ] **Step 1: 전체 검증**

Run:
```bash
cargo test --manifest-path server/Cargo.toml
cargo clippy --manifest-path server/Cargo.toml --all-targets
cd client && npm run typecheck && npm test && npm run build && npx playwright test
```
Expected: 전부 PASS(`sim.rs:285`의 기존 unknown-lint 경고 1건 제외). 실제 숫자(서버 테스트 개수, 클라이언트 유닛/E2E 개수)는 이 실행 결과에서 그대로 가져온다 — 추측하지 않는다.

- [ ] **Step 2: README.md 갱신**

"## 지금까지 만든 것" 절의 "로봇 외형 리디자인" 항목(로봇 외형 리디자인 플랜에서 추가됨) 바로 다음에 추가:

```markdown
- **목적있는 이동(작업 사이클)**: 컨베이어가 켜져 있으면 로봇이 자동으로 픽업 지점→배치 지점을 오가며 화물을 나른다(컨베이어 꺼지면 기존 순찰로 복귀). 생산량 통계가 "컨베이어 켜짐 + 로봇 존재"라는 전제만으로 오르던 것에서, 실제로 배치를 완료한 로봇 수에 연결됐다. 픽업/배치 지점은 기존 순찰 지점(`patrol_points`)을 재사용하지 않고 그리드 전체에 해시로 분산시킨 새 지점(`work_points`)을 쓴다 — 순찰 지점을 그대로 썼다면 10칸×10칸 그리드에서 정확히 10대마다 좌표가 겹쳐 작업 사이클처럼 오래 정지하는 용도로는 위험했다는 걸 완결성 리뷰에서 발견해 바꿨다. 오퍼레이터의 수동 `TriggerArmAction`은 컨베이어가 켜진 로봇에게는 완전히 no-op이 되도록 설계했다(자동 사이클이 `task`를 절대 입력으로 읽지 않고 매 틱 다시 파생시켜 덮어쓰므로, 수동 조작으로 대기 시간 없이 화물을 즉시 든 것처럼 만드는 악용 경로가 원천 차단된다).
```

- [ ] **Step 3: docs/KANBAN.md 갱신**

"## Done" 절의 "로봇 외형 리디자인" 섹션 다음에 추가:

```markdown
### 목적있는 이동 (작업 사이클) (`docs/superpowers/specs/2026-07-21-robot-purposeful-movement-design.md`, `docs/superpowers/plans/2026-07-21-robot-purposeful-movement-plan.md`)
라이브 데모 실사용 피드백("목적도 없이 돌아다닌다") → 브레인스토밍 → 설계 → 독립 리뷰어 완결성 리뷰(작업 지점 충돌 버그, `TriggerArmAction` 악용 경로, `tick()` 시그니처 변경 회귀 위험 발견 → 전부 반영해 스펙 수정) → 5개 태스크 전부 완료.
- **Task 1** — `sim_core` 작업 사이클(픽업→운반→배치) + `tick()`/`plan_robot`/`safe_plan_robot`/`safe_tick`에 `conveyor_running` 전파 + `work_points`(순찰 지점 재사용 대신 그리드 전체 해시 분산) + `TriggerArmAction` 악용 경로 차단(task를 입력으로 안 읽고 파생만 시킴, 뮤테이션 테스트로 실증).
- **Task 2** — 생산량 집계를 "컨베이어 켜짐+로봇 존재"라는 전제에서 "실제 배치 완료"(`detect_completed_placements`)로 교체.
- **Task 3** — `RobotView.carrying` 와이어 필드 추가.
- **Task 4** — 클라이언트 화물 아이콘 렌더링.
- **Task 5** — 전체 검증 + 문서 갱신.
```

- [ ] **Step 4: 커밋**

```bash
git add README.md docs/KANBAN.md
git commit -m "docs: record purposeful-movement work cycle completion in README and KANBAN"
```
