# 클라이언트 렌더링 구현 계획 (Plan 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 서버가 계산하는 로봇팔 컨베이어 시뮬레이션을 아이소메트릭 캔버스로 렌더링하고, 우측 사이드바로 커맨드를 보내는 Vite+TS 웹 클라이언트를 만든다.

**Architecture:** 서버(`server/`)에 `facing`/`path`/`arm_pose` 노출을 먼저 추가한 뒤(Task 1-2), `client/` 아래에 net(WS+프로토콜 미러)/state(델타 병합+보간)/render(투영+z-order+캔버스)/ui(사이드바) 모듈을 순서대로 쌓고, 마지막에 실제 서버 바이너리+실제 브라우저로 검증한다.

**Tech Stack:** Rust(서버, 기존), Vite + TypeScript + Canvas 2D(클라이언트, 프레임워크 없음), vitest(단위/통합), Playwright(E2E), npm.

**설계 근거:** `docs/superpowers/specs/2026-07-17-client-rendering-design.md` (브레인스토밍에서 확정, 이 계획의 모든 결정은 그 문서를 따른다)

---

### Task 1: `sim_core` — 로봇 `facing` 필드 + 이동 방향 추적

**Files:**
- Modify: `server/src/sim.rs`

- [x] **Step 1: 실패하는 테스트 작성**

`server/src/sim.rs`의 `#[cfg(test)] mod tests` 블록 끝(`repairing_robot_counts_down_and_returns_to_operational` 테스트 뒤)에 추가:

```rust
    #[test]
    fn direction_from_move_detects_four_cardinal_directions() {
        assert_eq!(Direction::from_move((0, 0), (1, 0)), Some(Direction::East));
        assert_eq!(Direction::from_move((0, 0), (-1, 0)), Some(Direction::West));
        assert_eq!(Direction::from_move((0, 0), (0, 1)), Some(Direction::North));
        assert_eq!(Direction::from_move((0, 0), (0, -1)), Some(Direction::South));
    }

    #[test]
    fn direction_from_move_returns_none_when_positions_are_equal() {
        assert_eq!(Direction::from_move((2, 2), (2, 2)), None);
    }

    #[test]
    fn new_robot_faces_east_by_default() {
        let robot = Robot::new(1, (0, 0), (0, 0));
        assert_eq!(robot.facing, Direction::East);
    }

    #[test]
    fn facing_updates_to_match_actual_movement_direction() {
        let mut state = simple_state(5, 1);
        state.robots.push(Robot::new(1, (0, 0), (3, 0)));

        let next = tick(&state);

        assert_eq!(next.robots[0].facing, Direction::East);
    }

    #[test]
    fn facing_does_not_change_when_a_robot_loses_its_tiebreak() {
        // 로봇 2는 타이브레이크에서 져서 (2,0)에 그대로 남는다 — facing이
        // 기본값(East)에서 바뀌면 안 된다. plan_robot() 안(타이브레이크 확정
        // 전)에서 facing을 갱신하면 이 테스트가 실패한다 — "실제로 하지
        //않은 이동"으로 잘못 회전하는 버그를 정확히 잡아내기 위한 테스트.
        let mut state = simple_state(3, 1);
        state.robots.push(Robot::new(1, (0, 0), (2, 0)));
        state.robots.push(Robot::new(2, (2, 0), (0, 0)));

        let next = tick(&state);

        let r2 = next.robots.iter().find(|r| r.id == 2).unwrap();
        assert_eq!(r2.pos, (2, 0), "진 로봇은 제자리에 남아야 한다(기존 불변식)");
        assert_eq!(r2.facing, Direction::East, "실제로 이동하지 않았으니 facing도 바뀌면 안 된다");
    }

    #[test]
    fn facing_holds_last_direction_while_stationary() {
        let mut state = simple_state(5, 1);
        state.robots.push(Robot::new(1, (0, 0), (3, 0)));
        state = tick(&state); // 동쪽으로 한 칸 이동 -> facing = East
        assert_eq!(state.robots[0].facing, Direction::East);

        state.robots[0].goal = (0, 0); // 이제 서쪽으로
        state = tick(&state);
        assert_eq!(state.robots[0].facing, Direction::West);

        // 목표 지점에 도달해 멈춘 뒤에도 마지막 방향을 유지해야 한다.
        let settled = state.robots[0].pos;
        state.robots[0].goal = settled;
        let held = tick(&state);
        assert_eq!(held.robots[0].facing, Direction::West);
    }
```

- [x] **Step 2: 테스트 실패 확인**

Run: `cargo test --manifest-path server/Cargo.toml direction_from_move`
Expected: FAIL — `error[E0433]: failed to resolve: use of undeclared type Direction` (또는 `no field 'facing' on type 'Robot'`)

- [x] **Step 3: `Direction` 타입 + `facing` 필드 추가**

`server/src/sim.rs`의 `BodyPose` 정의 바로 뒤(17번째 줄 근처)에 추가:

```rust
/// 로봇이 마지막으로 실제 이동한 방향(그리드는 4방향 이동만 지원하므로
/// `Grid::neighbors`, `grid.rs:33-39` — 대각선은 없다). 렌더러(Plan 4)가
/// 몸체-로컬 팔 타겟을 월드 좌표로 회전시키는 기준으로 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    North,
    East,
    South,
    West,
}

impl Direction {
    /// 로봇이 `from`에서 `to`로 정확히 한 칸 이동했을 때의 방향.
    /// 이동이 없으면(`from == to`) `None` — 호출부가 기존 방향을 유지한다.
    pub fn from_move(from: CellId, to: CellId) -> Option<Direction> {
        match (to.0 - from.0, to.1 - from.1) {
            (1, 0) => Some(Direction::East),
            (-1, 0) => Some(Direction::West),
            (0, 1) => Some(Direction::North),
            (0, -1) => Some(Direction::South),
            _ => None,
        }
    }
}
```

`Robot` 구조체(53-64번째 줄)에 필드 추가:

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
    pub facing: Direction,
}
```

`Robot::new`(67-80번째 줄)에 필드 초기화 추가:

```rust
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
            facing: Direction::East,
        }
    }
```

`tick()`의 `new_robots` 매핑(183-197번째 줄)을 다음으로 교체 — **`plan_robot` 안이 아니라 타이브레이크가 확정된 여기서** `facing`을 갱신한다(그 이유는 위 Step 1의 `facing_does_not_change_when_a_robot_loses_its_tiebreak` 테스트 주석 참고):

```rust
    let new_robots: Vec<Robot> = state
        .robots
        .iter()
        .zip(planned)
        .zip(resolved_positions)
        .map(|((original, mut robot), final_pos)| {
            let lost_tiebreak = final_pos != robot.pos;
            robot.pos = final_pos;
            if lost_tiebreak {
                // 다른 로봇이 이번 칸을 가져갔다 — 이번 틱은 제자리에 멈추고
                // 다음 기회에 새로 재계획한다 (무의미한 즉시 재시도 방지).
                robot.path.clear();
                robot.ticks_until_repath = 0;
            }
            if robot.pos != original.pos {
                robot.leg_cycle_progress = (robot.leg_cycle_progress + LEG_CYCLE_SPEED).rem_euclid(1.0);
                if let Some(dir) = Direction::from_move(original.pos, robot.pos) {
                    robot.facing = dir;
                }
            }
            robot
        })
        .collect();
```

- [x] **Step 4: 테스트 통과 확인**

Run: `cargo test --manifest-path server/Cargo.toml --lib`
Expected: PASS (전체 스위트, 새 테스트 5개 포함)

- [x] **Step 5: 커밋**

```bash
git add server/src/sim.rs
git commit -m "feat: sim_core에 로봇 facing 필드와 이동 방향 추적 추가"
```

---

### Task 2: `protocol.rs` — path/facing/팔 각도 노출 + 자세를 task 기반으로 전환

**Files:**
- Modify: `server/src/protocol.rs`
- Modify: `server/src/sim.rs` (죽어있던 `pose` 필드 제거)
- Modify: `server/src/delta.rs` (테스트 헬퍼)

이 태스크는 세 파일을 함께 바꾼다 — `Robot.pose` 필드를 지우는 순간 `protocol.rs`(그 필드를 읽던 곳)와 `delta.rs`의 테스트 헬퍼(그 필드를 리터럴로 채우던 곳)가 동시에 안 맞게 되므로, 하나의 원자적 변경으로 묶는다.

- [ ] **Step 1: 실패하는 테스트 작성**

`server/src/protocol.rs`의 `#[cfg(test)] mod tests` 블록에 추가(기존 `robot_view_reports_repairing_status_with_remaining_ticks` 테스트 뒤):

```rust
    #[test]
    fn robot_view_reports_path_as_wire_cells() {
        use sim_core::sim::Robot;
        let mut robot = Robot::new(1, (0, 0), (5, 0));
        robot.path = vec![(1, 0), (2, 0)];

        let view = RobotView::from(&robot);

        assert_eq!(view.path, vec![WireCellId { x: 1, y: 0 }, WireCellId { x: 2, y: 0 }]);
    }

    #[test]
    fn robot_view_reports_facing() {
        use sim_core::sim::{Direction, Robot};
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        robot.facing = Direction::North;

        let view = RobotView::from(&robot);

        assert_eq!(view.facing, WireDirection::North);
    }

    #[test]
    fn robot_view_pose_is_standing_when_idle_and_crouching_while_working() {
        use sim_core::sim::{Robot, Task};
        let idle = Robot::new(1, (0, 0), (0, 0));
        assert_eq!(RobotView::from(&idle).pose, WirePose::Standing);

        let mut working = Robot::new(2, (0, 0), (0, 0));
        working.task = Task::Picking;
        assert_eq!(RobotView::from(&working).pose, WirePose::Crouching);
    }

    #[test]
    fn robot_view_arm_pose_is_idle_rest_when_task_is_idle() {
        use sim_core::sim::Robot;
        let robot = Robot::new(1, (0, 0), (0, 0));
        assert_eq!(RobotView::from(&robot).arm_pose, IDLE_ARM_POSE);
    }

    #[test]
    fn robot_view_arm_pose_is_solved_via_ik_while_working() {
        use sim_core::sim::{Robot, Task};
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        robot.task = Task::Picking;

        let view = RobotView::from(&robot);

        assert_ne!(view.arm_pose, IDLE_ARM_POSE, "작업 중이면 대기 자세가 아니라 실제 IK 해가 나와야 한다");
        assert!(view.arm_pose.shoulder_angle.is_finite());
        assert!(view.arm_pose.elbow_angle.is_finite());
    }

    #[test]
    fn robot_view_arm_pose_is_stable_when_task_and_facing_are_unchanged() {
        // task/facing이 같으면 다른 로봇(다른 id/위치)이어도 arm_pose가
        // 완전히 같아야 한다 — compute_delta의 PartialEq 비교가 이 필드를
        // 델타에서 제대로 걸러내는지(대역폭 회귀 방지)의 전제조건.
        use sim_core::sim::{Robot, Task};
        let mut a = Robot::new(1, (0, 0), (0, 0));
        a.task = Task::Picking;
        let mut b = Robot::new(2, (5, 5), (5, 5));
        b.task = Task::Picking;

        assert_eq!(RobotView::from(&a).arm_pose, RobotView::from(&b).arm_pose);
    }
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test --manifest-path server/Cargo.toml robot_view_reports_path`
Expected: FAIL — `error[E0609]: no field 'path' on type 'RobotView'` (또는 `WireDirection`/`IDLE_ARM_POSE` 미정의)

- [ ] **Step 3: `protocol.rs` 구현**

`server/src/protocol.rs` 상단 `use` 절을 교체:

```rust
use crate::game_state::{Conveyor, GameState};
use serde::{Deserialize, Serialize};
use sim_core::grid::CellId;
use sim_core::ik::solve_two_bone_ik;
use sim_core::posture::world_target_to_body_local;
use sim_core::sim::{BodyPose, Direction, Robot, RobotStatus, Task};
```

`WirePose` 정의 뒤(기존 54-67번째 줄 부근)에 `WireDirection` 추가:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WireDirection {
    North,
    East,
    South,
    West,
}

impl From<Direction> for WireDirection {
    fn from(d: Direction) -> WireDirection {
        match d {
            Direction::North => WireDirection::North,
            Direction::East => WireDirection::East,
            Direction::South => WireDirection::South,
            Direction::West => WireDirection::West,
        }
    }
}
```

`RobotView` 구조체(기존 99-108번째 줄)를 교체:

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
    pub path: Vec<WireCellId>,
    pub facing: WireDirection,
    pub arm_pose: WireArmPose,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct WireArmPose {
    pub shoulder_angle: f32,
    pub elbow_angle: f32,
}

// 아래 네 상수는 클라이언트(`client/src/render/projection.ts`)에도 그대로
// 미러링해서 유지해야 한다 — 와이어로 안 보내는 이유는 안 바뀌는 튜닝
// 상수를 매 메시지에 싣는 게 낭비이기 때문(설계문서 "서버 쪽 변경" 절 참고).
const WORK_TARGET_HEIGHT: f32 = 0.75;
const WORK_TARGET_FORWARD: f32 = 0.6;
const UPPER_ARM_LEN: f32 = 0.7;
const LOWER_ARM_LEN: f32 = 0.6;
const IDLE_ARM_POSE: WireArmPose = WireArmPose { shoulder_angle: 0.0, elbow_angle: 0.0 };

/// `task`만 보고 자세(Standing/Crouching)를 결정한다 — 컨베이어 칸별
/// 높이 같은 위치 기반 데이터는 시뮬레이션에 없고(설계문서 "서버 쪽 변경"
/// 절 참고), 그런 걸 새로 만들 필요도 없다고 확인됐다.
fn pose_for(task: Task) -> BodyPose {
    if task == Task::Idle {
        BodyPose::Standing
    } else {
        BodyPose::Crouching
    }
}

fn arm_pose_for(robot: &Robot) -> WireArmPose {
    if robot.task == Task::Idle {
        return IDLE_ARM_POSE;
    }
    let local_target = world_target_to_body_local(WORK_TARGET_HEIGHT, WORK_TARGET_FORWARD, pose_for(robot.task));
    let solved = solve_two_bone_ik(UPPER_ARM_LEN, LOWER_ARM_LEN, local_target);
    WireArmPose { shoulder_angle: solved.shoulder_angle, elbow_angle: solved.elbow_angle }
}
```

`impl From<&Robot> for RobotView`(기존 110-122번째 줄)를 교체:

```rust
impl From<&Robot> for RobotView {
    fn from(r: &Robot) -> RobotView {
        RobotView {
            id: r.id,
            pos: r.pos.into(),
            pose: pose_for(r.task).into(),
            leg_cycle_progress: r.leg_cycle_progress,
            task: r.task.into(),
            status: r.status.into(),
            durability_remaining: quantize_durability(r.wear_ratio()),
            path: r.path.iter().map(|&c| c.into()).collect(),
            facing: r.facing.into(),
            arm_pose: arm_pose_for(r),
        }
    }
}
```

이제 `server/src/sim.rs`에서 죽어있던 `pose` 필드를 제거한다 — `Robot` 구조체에서 `pub pose: BodyPose,` 줄을 삭제하고, `Robot::new`에서 `pose: BodyPose::Standing,` 줄을 삭제한다. `BodyPose` 타입 자체와 `shoulder_height()`는 그대로 둔다(`protocol.rs`의 `pose_for`/`world_target_to_body_local`이 계속 사용).

마지막으로 `server/src/delta.rs`의 테스트 헬퍼(33-49번째 줄)를 갱신:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{WireArmPose, WireCellId, WireDirection, WireStatus, WireTask};
    use sim_core::sim::BodyPose;

    fn robot_view(id: u32, x: i32) -> RobotView {
        RobotView {
            id,
            pos: WireCellId { x, y: 0 },
            pose: BodyPose::Standing.into(),
            leg_cycle_progress: 0.0,
            task: WireTask::Idle,
            status: WireStatus::Operational,
            durability_remaining: 1.0,
            path: Vec::new(),
            facing: WireDirection::East,
            arm_pose: WireArmPose { shoulder_angle: 0.0, elbow_angle: 0.0 },
        }
    }
```

(나머지 테스트 함수들은 변경 없음 — `robot_view` 헬퍼만 새 필드를 채우도록 고쳤다.)

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test --manifest-path server/Cargo.toml --lib`
Expected: PASS (전체 스위트)

Run: `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings`
Expected: 경고 0개

- [ ] **Step 5: 통합테스트 확인**

`server/tests/ws_integration.rs`/`server/tests/rest_integration.rs`/`server/tests/tick_properties.rs`는 `RobotView`를 직접 리터럴로 만들지 않고 실제 서버를 통해 JSON을 받거나 `Robot::new`를 쓰므로 이 변경에 영향받지 않아야 한다.

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: PASS (전체 스위트, 유닛+통합+프로퍼티 전부)

- [ ] **Step 6: 커밋**

```bash
git add server/src/protocol.rs server/src/sim.rs server/src/delta.rs
git commit -m "feat: RobotView에 path/facing/arm_pose 노출, 자세를 task 기반으로 전환"
```

---

### Task 3: 클라이언트 프로젝트 스캐폴드 (Vite + TS + vitest + Playwright)

**Files:**
- Create: `client/package.json`
- Create: `client/tsconfig.json`
- Create: `client/vite.config.ts`
- Create: `client/vitest.config.ts`
- Create: `client/vitest.integration.config.ts`
- Create: `client/playwright.config.ts`
- Create: `client/index.html`
- Create: `client/src/main.ts`
- Create: `client/.gitignore`
- Create: `client/tests/unit/smoke.test.ts`

- [ ] **Step 1: 디렉토리 확인 후 `package.json` 작성**

```json
{
  "name": "gamerobotfactory-client",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview",
    "typecheck": "tsc --noEmit",
    "test": "vitest run --config vitest.config.ts",
    "test:integration": "vitest run --config vitest.integration.config.ts",
    "test:e2e": "playwright test"
  },
  "devDependencies": {
    "@playwright/test": "^1.48.0",
    "@types/node": "^22.0.0",
    "@types/ws": "^8.5.0",
    "jsdom": "^25.0.0",
    "typescript": "^5.6.0",
    "vite": "^5.4.0",
    "vitest": "^2.1.0",
    "ws": "^8.18.0"
  }
}
```

- [ ] **Step 2: `tsconfig.json` 작성**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022", "DOM"],
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "noEmit": true
  },
  "include": ["src", "tests"]
}
```

- [ ] **Step 3: `vite.config.ts` 작성**

```ts
import { defineConfig } from 'vite'

export default defineConfig({
  root: '.',
  build: {
    outDir: 'dist',
  },
})
```

- [ ] **Step 4: `vitest.config.ts`(단위) 작성**

```ts
import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    include: ['tests/unit/**/*.test.ts'],
    environment: 'node',
  },
})
```

- [ ] **Step 5: `vitest.integration.config.ts` 작성**

```ts
import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    include: ['tests/integration/**/*.test.ts'],
    environment: 'node',
    testTimeout: 20000,
    hookTimeout: 20000,
    // 서버 바이너리를 기동하는 자식 프로세스를 여러 테스트 파일이 동시에
    // 띄우면 스레드 풀에서 불필요하게 경합하므로 순차 실행한다(포트는
    // 서버가 각자 임의 할당하므로 충돌은 안 나지만, 프로세스 기동 자체가
    // 무거워서 직렬화가 더 안정적).
    fileParallelism: false,
  },
})
```

- [ ] **Step 6: `playwright.config.ts` 작성**

```ts
import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: './tests/e2e',
  timeout: 30000,
  fullyParallel: false,
  globalSetup: './tests/e2e/global-setup.ts',
  globalTeardown: './tests/e2e/global-teardown.ts',
  webServer: {
    command: 'npm run build && npm run preview -- --port 4173 --strictPort',
    port: 4173,
    reuseExistingServer: false,
  },
  use: {
    baseURL: 'http://localhost:4173',
  },
})
```

(`globalSetup`/`globalTeardown`이 가리키는 파일은 Task 13에서 작성한다 — 이 태스크에서는 아직 없어도 `playwright.config.ts` 자체는 문법 오류 없이 존재해야 하므로, 이 설정 파일은 만들어두되 Task 13 전까지는 `npm run test:e2e`를 실행하지 않는다.)

- [ ] **Step 7: `index.html` 작성**

```html
<!doctype html>
<html lang="ko">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>로봇팔 컨베이어 — 클라이언트</title>
    <style>
      html, body { margin: 0; height: 100%; background: #0a0f0a; color: #e8e8e8; font-family: sans-serif; }
      #app { display: flex; height: 100%; }
    </style>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
```

- [ ] **Step 8: 최소 `main.ts` 작성 (뒤 Task 10에서 전체 교체됨)**

```ts
const app = document.getElementById('app')
if (app) {
  app.textContent = '로딩 중...'
}
```

- [ ] **Step 9: `.gitignore` 작성**

```
node_modules/
dist/
playwright-report/
test-results/
tests/e2e/.server-info.json
```

- [ ] **Step 10: 스모크 테스트 작성 — 툴체인 배선 확인**

`client/tests/unit/smoke.test.ts`:

```ts
import { describe, it, expect } from 'vitest'

describe('vitest wiring', () => {
  it('runs a trivial assertion', () => {
    expect(1 + 1).toBe(2)
  })
})
```

- [ ] **Step 11: 의존성 설치 + 타입체크 + 스모크 테스트 실행**

Run: `cd client && npm install`
Expected: 설치 완료, 에러 없음

Run: `npm run typecheck`
Expected: 에러 없음(아직 코드가 거의 없으므로 통과가 자명하지만, tsconfig 자체가 유효한지 확인하는 게 목적)

Run: `npm test`
Expected: PASS — `smoke.test.ts`의 1개 테스트 통과

- [ ] **Step 12: 커밋**

```bash
git add client/package.json client/package-lock.json client/tsconfig.json client/vite.config.ts client/vitest.config.ts client/vitest.integration.config.ts client/playwright.config.ts client/index.html client/src/main.ts client/.gitignore client/tests/unit/smoke.test.ts
git commit -m "chore: 클라이언트 프로젝트 스캐폴드(Vite+TS+vitest+Playwright)"
```

---

### Task 4: 프로토콜 타입 (`net/protocol.ts`)

서버 `protocol.rs`의 와이어 타입을 그대로 미러링하는 TS 타입 + JSON 파싱/인코딩 순수 함수. 서버가 이미 모든 커맨드를 검증하므로 클라이언트 쪽엔 별도 런타임 스키마 검증 라이브러리를 두지 않는다(설계문서 "클라이언트 아키텍처" 절).

**Files:**
- Create: `client/src/net/protocol.ts`
- Create: `client/tests/unit/protocol.test.ts`

- [ ] **Step 1: 실패하는 테스트 작성**

`client/tests/unit/protocol.test.ts`:

```ts
import { describe, it, expect } from 'vitest'
import { parseServerMessage, encodeClientCommand } from '../../src/net/protocol'

describe('parseServerMessage', () => {
  it('parses a Snapshot message', () => {
    const raw = JSON.stringify({
      kind: 'Snapshot',
      v: 1,
      tick: 5,
      session_id: '00000000-0000-0000-0000-000000000000',
      conveyor: { running: true },
      robots: [],
    })

    const msg = parseServerMessage(raw)

    expect(msg).not.toBeNull()
    expect(msg?.kind).toBe('Snapshot')
  })

  it('parses a Delta message with a null conveyor (unchanged)', () => {
    const raw = JSON.stringify({
      kind: 'Delta',
      v: 1,
      tick: 6,
      conveyor: null,
      changed_robots: [],
      removed_robot_ids: [],
    })

    const msg = parseServerMessage(raw)

    expect(msg).not.toBeNull()
    if (msg?.kind === 'Delta') {
      expect(msg.conveyor).toBeNull()
    } else {
      throw new Error('expected Delta')
    }
  })

  it('returns null for invalid JSON instead of throwing', () => {
    expect(parseServerMessage('not valid json')).toBeNull()
  })

  it('returns null for JSON missing a kind field', () => {
    expect(parseServerMessage(JSON.stringify({ foo: 'bar' }))).toBeNull()
  })
})

describe('encodeClientCommand', () => {
  it('encodes SelectRobot matching the server tagged-union shape', () => {
    const json = encodeClientCommand({ type: 'SelectRobot', robot_id: 7 })
    expect(JSON.parse(json)).toEqual({ type: 'SelectRobot', robot_id: 7 })
  })

  it('encodes TriggerArmAction with a nested task string', () => {
    const json = encodeClientCommand({ type: 'TriggerArmAction', robot_id: 3, task: 'Picking' })
    expect(JSON.parse(json)).toEqual({ type: 'TriggerArmAction', robot_id: 3, task: 'Picking' })
  })
})
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cd client && npm test`
Expected: FAIL — `Cannot find module '../../src/net/protocol'`

- [ ] **Step 3: `net/protocol.ts` 구현**

```ts
// client/src/net/protocol.ts
//
// server/src/protocol.rs의 와이어 타입을 그대로 미러링한다. 서버가 이미
// 모든 커맨드를 검증하므로(존재하지 않는 로봇 거부 등) 여기엔 별도
// 런타임 스키마 검증을 두지 않는다.

export interface WireCellId {
  x: number
  y: number
}

export type WireTask = 'Idle' | 'Picking' | 'Placing'

export type WirePose = 'Standing' | 'Crouching'

export type WireDirection = 'North' | 'East' | 'South' | 'West'

export interface WireArmPose {
  shoulder_angle: number
  elbow_angle: number
}

export type WireStatus =
  | { kind: 'Operational' }
  | { kind: 'Failed' }
  | { kind: 'Repairing'; remaining_ticks: number }

export interface RobotView {
  id: number
  pos: WireCellId
  pose: WirePose
  leg_cycle_progress: number
  task: WireTask
  status: WireStatus
  durability_remaining: number
  path: WireCellId[]
  facing: WireDirection
  arm_pose: WireArmPose
}

export interface ConveyorView {
  running: boolean
}

export type ServerMessage =
  | { kind: 'Snapshot'; v: number; tick: number; session_id: string; conveyor: ConveyorView; robots: RobotView[] }
  | {
      kind: 'Delta'
      v: number
      tick: number
      conveyor: ConveyorView | null
      changed_robots: RobotView[]
      removed_robot_ids: number[]
    }
  | { kind: 'ResumeAck'; v: number; session_id: string; resumed: boolean }

export type ClientCommand =
  | { type: 'SelectRobot'; robot_id: number }
  | { type: 'ReleaseRobot' }
  | { type: 'ToggleConveyor' }
  | { type: 'SetRobotCount'; count: number }
  | { type: 'TriggerArmAction'; robot_id: number; task: WireTask }
  | { type: 'RepairRobot'; robot_id: number }
  | { type: 'Resume'; session_id: string }

/** 파싱 실패(잘못된 JSON, `kind` 없음)는 예외 대신 `null` — 서버의
 * "잘못된 메시지는 로그만 남기고 연결 유지" 정책과 대칭. */
export function parseServerMessage(raw: string): ServerMessage | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return null
  }
  if (typeof parsed !== 'object' || parsed === null || typeof (parsed as { kind?: unknown }).kind !== 'string') {
    return null
  }
  return parsed as ServerMessage
}

export function encodeClientCommand(command: ClientCommand): string {
  return JSON.stringify(command)
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cd client && npm test`
Expected: PASS

- [ ] **Step 5: 타입체크**

Run: `npm run typecheck`
Expected: 에러 없음

- [ ] **Step 6: 커밋**

```bash
git add client/src/net/protocol.ts client/tests/unit/protocol.test.ts
git commit -m "feat: 서버 와이어 프로토콜을 미러링하는 TS 타입 추가"
```

---

### Task 5: 로컬 미러 상태 (`state/mirror.ts`)

서버의 `Snapshot`/`Delta`를 그대로 재생해 "현재 알려진 전체 로봇 상태 맵"을 유지하는 순수 함수(설계문서 "프로토콜 연동 > 로컬 미러 상태" 절).

**Files:**
- Create: `client/src/state/mirror.ts`
- Create: `client/tests/unit/mirror.test.ts`

- [ ] **Step 1: 실패하는 테스트 작성**

`client/tests/unit/mirror.test.ts`:

```ts
import { describe, it, expect } from 'vitest'
import { applyServerMessage, createEmptyMirror } from '../../src/state/mirror'
import type { RobotView } from '../../src/net/protocol'

function robot(id: number, x: number): RobotView {
  return {
    id,
    pos: { x, y: 0 },
    pose: 'Standing',
    leg_cycle_progress: 0,
    task: 'Idle',
    status: { kind: 'Operational' },
    durability_remaining: 1,
    path: [],
    facing: 'East',
    arm_pose: { shoulder_angle: 0, elbow_angle: 0 },
  }
}

describe('applyServerMessage', () => {
  it('replaces the whole robot map on Snapshot', () => {
    const mirror = createEmptyMirror()
    const next = applyServerMessage(mirror, {
      kind: 'Snapshot',
      v: 1,
      tick: 1,
      session_id: 'abc',
      conveyor: { running: true },
      robots: [robot(1, 0), robot(2, 5)],
    })

    expect(next.conveyor).toEqual({ running: true })
    expect(next.robots.size).toBe(2)
    expect(next.robots.get(1)?.pos).toEqual({ x: 0, y: 0 })
  })

  it('overwrites changed robots on Delta', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [robot(1, 0)],
    })

    mirror = applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: null, changed_robots: [robot(1, 3)], removed_robot_ids: [],
    })

    expect(mirror.robots.get(1)?.pos).toEqual({ x: 3, y: 0 })
  })

  it('removes robots listed in removed_robot_ids', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [robot(1, 0), robot(2, 1)],
    })

    mirror = applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: null, changed_robots: [], removed_robot_ids: [2],
    })

    expect(mirror.robots.has(2)).toBe(false)
    expect(mirror.robots.has(1)).toBe(true)
  })

  it('keeps the previous conveyor state when Delta.conveyor is null', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [],
    })

    mirror = applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: null, changed_robots: [], removed_robot_ids: [],
    })

    expect(mirror.conveyor).toEqual({ running: true })
  })

  it('adopts the new conveyor state when Delta.conveyor is present', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [],
    })

    mirror = applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: { running: false }, changed_robots: [], removed_robot_ids: [],
    })

    expect(mirror.conveyor).toEqual({ running: false })
  })

  it('leaves the mirror untouched on ResumeAck', () => {
    const mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [robot(1, 0)],
    })

    const next = applyServerMessage(mirror, { kind: 'ResumeAck', v: 1, session_id: 'abc', resumed: true })

    expect(next).toBe(mirror)
  })

  it('does not mutate the previous mirror object (pure function)', () => {
    // 뮤테이션 테스트 대상: applyServerMessage가 mirror.robots를 제자리에서
    // 고치면, 보간 스토어(Task 7)가 들고 있는 "직전 틱" 스냅샷까지 같이
    // 바뀌어버려 보간이 항상 alpha=1처럼 동작하는 조용한 버그가 된다.
    const mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [robot(1, 0)],
    })
    const robotsBefore = mirror.robots

    applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: null, changed_robots: [robot(1, 9)], removed_robot_ids: [],
    })

    expect(mirror.robots).toBe(robotsBefore)
    expect(mirror.robots.get(1)?.pos).toEqual({ x: 0, y: 0 })
  })
})
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cd client && npm test`
Expected: FAIL — `Cannot find module '../../src/state/mirror'`

- [ ] **Step 3: `state/mirror.ts` 구현**

```ts
// client/src/state/mirror.ts
import type { ConveyorView, RobotView, ServerMessage } from '../net/protocol'

export interface MirrorState {
  conveyor: ConveyorView
  robots: Map<number, RobotView>
}

export function createEmptyMirror(): MirrorState {
  return { conveyor: { running: false }, robots: new Map() }
}

/** 서버의 Snapshot/Delta 프로토콜을 그대로 재생하는 순수 함수. 입력
 * `mirror`를 절대 제자리에서 고치지 않는다 — 항상 새 객체를 반환한다. */
export function applyServerMessage(mirror: MirrorState, message: ServerMessage): MirrorState {
  switch (message.kind) {
    case 'Snapshot':
      return {
        conveyor: message.conveyor,
        robots: new Map(message.robots.map((r) => [r.id, r])),
      }
    case 'Delta': {
      const robots = new Map(mirror.robots)
      for (const robot of message.changed_robots) {
        robots.set(robot.id, robot)
      }
      for (const id of message.removed_robot_ids) {
        robots.delete(id)
      }
      return {
        conveyor: message.conveyor ?? mirror.conveyor,
        robots,
      }
    }
    case 'ResumeAck':
      return mirror
  }
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cd client && npm test`
Expected: PASS (전체 스위트)

- [ ] **Step 5: 커밋**

```bash
git add client/src/state/mirror.ts client/tests/unit/mirror.test.ts
git commit -m "feat: 서버 델타 프로토콜을 재생하는 로컬 미러 상태 추가"
```

---

### Task 6: 아이소메트릭 투영 + 팔 좌표 복원 + z-order (`render/projection.ts`)

그리드 좌표 → 화면 좌표 변환, 서버 `ik.rs::forward_kinematics`를 TS로 재구현한 팔 좌표 복원, 바운딩박스 기준 z-order 키 계산. 전부 순수 함수(설계문서 "렌더링" 절).

**Files:**
- Create: `client/src/render/projection.ts`
- Create: `client/tests/unit/projection.test.ts`

- [ ] **Step 1: 실패하는 테스트 작성**

`client/tests/unit/projection.test.ts`:

```ts
import { describe, it, expect } from 'vitest'
import {
  gridToScreen,
  forwardKinematics,
  forwardDirectionVector,
  wristWorldOffset,
  zOrderKey,
  UPPER_ARM_LEN,
  LOWER_ARM_LEN,
} from '../../src/render/projection'

describe('gridToScreen', () => {
  it('maps the origin to the screen origin', () => {
    expect(gridToScreen(0, 0)).toEqual({ x: 0, y: 0 })
  })

  it('maps grid axes to the expected diamond offsets', () => {
    expect(gridToScreen(1, 0)).toEqual({ x: 32, y: 16 })
    expect(gridToScreen(0, 1)).toEqual({ x: -32, y: 16 })
    expect(gridToScreen(1, 1)).toEqual({ x: 0, y: 32 })
  })
})

describe('forwardKinematics', () => {
  it('points straight forward when both angles are zero', () => {
    const p = forwardKinematics(UPPER_ARM_LEN, LOWER_ARM_LEN, 0, 0)
    expect(p.x).toBeCloseTo(UPPER_ARM_LEN + LOWER_ARM_LEN)
    expect(p.y).toBeCloseTo(0)
  })

  it('matches manual trigonometry for a 90-degree shoulder angle', () => {
    const p = forwardKinematics(UPPER_ARM_LEN, LOWER_ARM_LEN, Math.PI / 2, 0)
    expect(p.x).toBeCloseTo(0, 5)
    expect(p.y).toBeCloseTo(UPPER_ARM_LEN + LOWER_ARM_LEN)
  })
})

describe('forwardDirectionVector', () => {
  it('maps all four facings to axis-aligned unit vectors', () => {
    expect(forwardDirectionVector('East')).toEqual({ dx: 1, dy: 0 })
    expect(forwardDirectionVector('West')).toEqual({ dx: -1, dy: 0 })
    expect(forwardDirectionVector('North')).toEqual({ dx: 0, dy: 1 })
    expect(forwardDirectionVector('South')).toEqual({ dx: 0, dy: -1 })
  })
})

describe('wristWorldOffset', () => {
  it('extends the wrist forward of the body in the facing direction', () => {
    const wrist = wristWorldOffset({ pos: { x: 2, y: 3 }, facing: 'East', shoulderAngle: 0, elbowAngle: 0 })
    expect(wrist.x).toBeCloseTo(2 + UPPER_ARM_LEN + LOWER_ARM_LEN)
    expect(wrist.y).toBeCloseTo(3)
  })
})

describe('zOrderKey', () => {
  it('extends past the body cell when the arm reaches forward', () => {
    const key = zOrderKey({ pos: { x: 2, y: 3 }, facing: 'East', shoulderAngle: 0, elbowAngle: 0 })
    expect(key).toBeCloseTo(2 + UPPER_ARM_LEN + LOWER_ARM_LEN + 3)
  })

  it('falls back to the body cell key when the arm folds backward past the body', () => {
    // shoulderAngle = PI (뒤쪽을 향함) -> 로컬 x가 음수 -> 월드 좌표로는
    // facing 방향의 "뒤"로 접히므로, max()가 몸체 칸 자체를 골라야 한다
    // (음수 방향으로 바운딩박스가 넓어지면 안 된다).
    const key = zOrderKey({ pos: { x: 2, y: 3 }, facing: 'East', shoulderAngle: Math.PI, elbowAngle: 0 })
    expect(key).toBeCloseTo(2 + 3)
  })
})
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cd client && npm test`
Expected: FAIL — `Cannot find module '../../src/render/projection'`

- [ ] **Step 3: `render/projection.ts` 구현**

```ts
// client/src/render/projection.ts

export const TILE_WIDTH = 64
export const TILE_HEIGHT = 32

// server/src/protocol.rs의 UPPER_ARM_LEN/LOWER_ARM_LEN과 반드시 같은
// 값으로 유지해야 한다 — 와이어로 안 보내는 튜닝 상수(설계문서 참고).
export const UPPER_ARM_LEN = 0.7
export const LOWER_ARM_LEN = 0.6

export interface ScreenPoint {
  x: number
  y: number
}

export function gridToScreen(x: number, y: number): ScreenPoint {
  return {
    x: (x - y) * (TILE_WIDTH / 2),
    y: (x + y) * (TILE_HEIGHT / 2),
  }
}

export interface LocalPoint {
  x: number // 전방
  y: number // 높이
}

/** 서버 ik.rs::forward_kinematics와 동일한 공식 — 각도를 몸체-로컬
 * (전방, 높이) 좌표로 되돌린다. */
export function forwardKinematics(upperLen: number, lowerLen: number, shoulderAngle: number, elbowAngle: number): LocalPoint {
  const elbowX = upperLen * Math.cos(shoulderAngle)
  const elbowY = upperLen * Math.sin(shoulderAngle)
  const wristAngle = shoulderAngle + elbowAngle
  return {
    x: elbowX + lowerLen * Math.cos(wristAngle),
    y: elbowY + lowerLen * Math.sin(wristAngle),
  }
}

export type Facing = 'North' | 'East' | 'South' | 'West'

/** facing은 그리드의 4방향과 1:1 대응하므로(대각선 없음), 회전은 항상
 * 축정렬된 단위 벡터를 곱하는 것으로 충분하다. */
export function forwardDirectionVector(facing: Facing): { dx: number; dy: number } {
  switch (facing) {
    case 'East':
      return { dx: 1, dy: 0 }
    case 'West':
      return { dx: -1, dy: 0 }
    case 'North':
      return { dx: 0, dy: 1 }
    case 'South':
      return { dx: 0, dy: -1 }
  }
}

export interface RobotPoseInput {
  pos: { x: number; y: number }
  facing: Facing
  shoulderAngle: number
  elbowAngle: number
}

/** 로봇 손목의 월드 그리드 좌표 — z-order와 팔 드로잉이 공유해서 쓴다. */
export function wristWorldOffset(input: RobotPoseInput): { x: number; y: number; height: number } {
  const local = forwardKinematics(UPPER_ARM_LEN, LOWER_ARM_LEN, input.shoulderAngle, input.elbowAngle)
  const dir = forwardDirectionVector(input.facing)
  return {
    x: input.pos.x + dir.dx * local.x,
    y: input.pos.y + dir.dy * local.x,
    height: local.y,
  }
}

/** z-order 정렬 키 — 몸체 칸이 아니라 (몸체+팔이 차지하는) 바운딩 박스의
 * 가장 먼 안쪽 모서리 기준 x+y (마스터 설계문서 line 120). 팔이 몸체
 * 뒤쪽으로 접히는 경우(로컬 x가 음수)엔 몸체 칸 자체가 최댓값이 된다. */
export function zOrderKey(input: RobotPoseInput): number {
  const wrist = wristWorldOffset(input)
  return Math.max(input.pos.x, wrist.x) + Math.max(input.pos.y, wrist.y)
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cd client && npm test`
Expected: PASS (전체 스위트)

- [ ] **Step 5: 커밋**

```bash
git add client/src/render/projection.ts client/tests/unit/projection.test.ts
git commit -m "feat: 아이소메트릭 투영, 팔 좌표 복원, z-order 키 계산 추가"
```

---

### Task 7: 보간/외삽 (`state/interpolation.ts`)

최근 2개 틱(prev/curr) 사이를 선형 보간하고, 다음 틱이 지연되면 짧게 외삽한 뒤 정지한다(설계문서 "프로토콜 연동 > 보간/외삽" 절).

**Files:**
- Create: `client/src/state/interpolation.ts`
- Create: `client/tests/unit/interpolation.test.ts`

- [ ] **Step 1: 실패하는 테스트 작성**

`client/tests/unit/interpolation.test.ts`:

```ts
import { describe, it, expect } from 'vitest'
import { computeRenderFactor, computeRenderRobots, TICK_DURATION_MS } from '../../src/state/interpolation'
import { createEmptyMirror, applyServerMessage } from '../../src/state/mirror'
import type { RobotView } from '../../src/net/protocol'

function robot(id: number, x: number): RobotView {
  return {
    id,
    pos: { x, y: 0 },
    pose: 'Standing',
    leg_cycle_progress: 0,
    task: 'Idle',
    status: { kind: 'Operational' },
    durability_remaining: 1,
    path: [],
    facing: 'East',
    arm_pose: { shoulder_angle: 0, elbow_angle: 0 },
  }
}

function mirrorWith(...robots: RobotView[]) {
  return applyServerMessage(createEmptyMirror(), {
    kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots,
  })
}

describe('computeRenderFactor', () => {
  it('is 0 at the moment curr was received', () => {
    expect(computeRenderFactor(0)).toBe(0)
  })

  it('is 0.5 halfway through the tick window', () => {
    expect(computeRenderFactor(TICK_DURATION_MS / 2)).toBeCloseTo(0.5)
  })

  it('is 1 exactly at the tick boundary', () => {
    expect(computeRenderFactor(TICK_DURATION_MS)).toBeCloseTo(1)
  })

  it('extrapolates past 1 when the next tick is late', () => {
    expect(computeRenderFactor(TICK_DURATION_MS + TICK_DURATION_MS / 2)).toBeCloseTo(1.5)
  })

  it('caps extrapolation instead of growing without bound', () => {
    const atCap = computeRenderFactor(TICK_DURATION_MS + 100)
    const wayPastCap = computeRenderFactor(TICK_DURATION_MS + 100_000)
    expect(atCap).toBeCloseTo(wayPastCap, 5)
  })
})

describe('computeRenderRobots', () => {
  it('interpolates halfway between prev and curr positions', () => {
    const prev = { mirror: mirrorWith(robot(1, 0)), receivedAtMs: 1000 }
    const curr = { mirror: mirrorWith(robot(1, 2)), receivedAtMs: 1050 }

    const rendered = computeRenderRobots(prev, curr, 1075) // 25ms into the 50ms window

    expect(rendered[0].renderPos.x).toBeCloseTo(1)
  })

  it('shows a newly-appeared robot at its curr position with no interpolation partner', () => {
    const curr = { mirror: mirrorWith(robot(1, 3)), receivedAtMs: 1000 }

    const rendered = computeRenderRobots(null, curr, 1000)

    expect(rendered[0].renderPos).toEqual({ x: 3, y: 0 })
  })

  it('extrapolates beyond curr when the next tick is late', () => {
    const prev = { mirror: mirrorWith(robot(1, 0)), receivedAtMs: 1000 }
    const curr = { mirror: mirrorWith(robot(1, 2)), receivedAtMs: 1050 }

    // curr로부터 25ms 지남(=elapsed 75ms, factor 1.5) -> 2 + (2-0)*0.5 = 3
    const rendered = computeRenderRobots(prev, curr, 1125)

    expect(rendered[0].renderPos.x).toBeCloseTo(3)
  })
})
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cd client && npm test`
Expected: FAIL — `Cannot find module '../../src/state/interpolation'`

- [ ] **Step 3: `state/interpolation.ts` 구현**

```ts
// client/src/state/interpolation.ts
import type { MirrorState } from './mirror'
import type { RobotView } from '../net/protocol'

export const TICK_DURATION_MS = 50
// 다음 틱이 지연되면 이 시간(ms)만큼만 짧게 외삽한 뒤 그 지점에서
// 정지한다 — 탭이 백그라운드에서 오래 스로틀돼도 무한정 앞서나가지
// 않는다(설계문서 "보간/외삽" 절). 튜닝 대상.
export const MAX_EXTRAPOLATION_MS = 100

export interface InterpolatedRobot extends RobotView {
  renderPos: { x: number; y: number }
}

export interface TickSnapshot {
  mirror: MirrorState
  receivedAtMs: number
}

/** prev->curr 진행 계수. [0,1]은 보간, 1보다 크면(최대
 * `1 + MAX_EXTRAPOLATION_MS/TICK_DURATION_MS`까지) curr 시점 속도로
 * 외삽한 값이고, 그 이상 지나도 더 커지지 않는다. */
export function computeRenderFactor(elapsedMs: number): number {
  const clampedElapsed = Math.max(elapsedMs, 0)
  if (clampedElapsed <= TICK_DURATION_MS) {
    return clampedElapsed / TICK_DURATION_MS
  }
  const overage = Math.min(clampedElapsed - TICK_DURATION_MS, MAX_EXTRAPOLATION_MS)
  return 1 + overage / TICK_DURATION_MS
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t
}

/** curr에 있는 로봇만 렌더링 대상이다(제거된 로봇은 mirror.robots에 이미
 * 없다). prev에 짝이 없는(새로 등장한) 로봇은 보간 없이 curr 위치 그대로
 * 표시한다. */
export function computeRenderRobots(prev: TickSnapshot | null, curr: TickSnapshot, nowMs: number): InterpolatedRobot[] {
  const factor = computeRenderFactor(nowMs - curr.receivedAtMs)
  const result: InterpolatedRobot[] = []

  for (const robot of curr.mirror.robots.values()) {
    const prevRobot = prev?.mirror.robots.get(robot.id)
    if (!prevRobot) {
      result.push({ ...robot, renderPos: { x: robot.pos.x, y: robot.pos.y } })
      continue
    }
    result.push({
      ...robot,
      renderPos: {
        x: lerp(prevRobot.pos.x, robot.pos.x, factor),
        y: lerp(prevRobot.pos.y, robot.pos.y, factor),
      },
    })
  }
  return result
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cd client && npm test`
Expected: PASS (전체 스위트)

- [ ] **Step 5: 커밋**

```bash
git add client/src/state/interpolation.ts client/tests/unit/interpolation.test.ts
git commit -m "feat: 틱 사이 보간 + 지연 시 짧은 외삽 로직 추가"
```

---

### Task 8: WebSocket 연결 + 세션/재접속 (`net/connection.ts`)

브라우저 `WebSocket`/`sessionStorage`를 직접 쓰지 않고 의존성 주입(팩토리 함수)으로 감싸 실제 네트워크/브라우저 없이 순수 로직을 유닛테스트한다(설계문서 "프로토콜 연동 > 세션/재접속" 절). 실제 `WebSocket`/`sessionStorage` 배선은 Task 10(`main.ts`)에서 한다.

**Files:**
- Create: `client/src/net/connection.ts`
- Create: `client/tests/unit/connection.test.ts`

- [ ] **Step 1: 실패하는 테스트 작성**

`client/tests/unit/connection.test.ts`:

```ts
import { describe, it, expect } from 'vitest'
import { Connection, type WebSocketLike, type SessionStorageLike } from '../../src/net/connection'

class FakeWebSocket implements WebSocketLike {
  sent: string[] = []
  closed = false
  onopen: WebSocketLike['onopen'] = null
  onmessage: WebSocketLike['onmessage'] = null
  onclose: WebSocketLike['onclose'] = null
  onerror: WebSocketLike['onerror'] = null

  send(data: string) {
    this.sent.push(data)
  }
  close() {
    this.closed = true
  }
}

function memoryStorage(): SessionStorageLike {
  const map = new Map<string, string>()
  return {
    getItem: (key) => map.get(key) ?? null,
    setItem: (key, value) => {
      map.set(key, value)
    },
  }
}

describe('Connection', () => {
  it('reports connecting then open, and does not send Resume without a saved session', () => {
    const sockets: FakeWebSocket[] = []
    const statuses: string[] = []
    const conn = new Connection(
      'ws://x',
      () => {
        const s = new FakeWebSocket()
        sockets.push(s)
        return s
      },
      memoryStorage(),
      { onMessage: () => {}, onStatusChange: (s) => statuses.push(s.kind) },
    )

    conn.connect()
    expect(statuses).toEqual(['connecting'])

    sockets[0].onopen?.(undefined)
    expect(statuses).toEqual(['connecting', 'open'])
    expect(sockets[0].sent).toEqual([])
  })

  it('sends Resume with the saved session id once connected', () => {
    const sockets: FakeWebSocket[] = []
    const storage = memoryStorage()
    storage.setItem('gamerobotfactory.session_id', 'saved-session')
    const conn = new Connection(
      'ws://x',
      () => {
        const s = new FakeWebSocket()
        sockets.push(s)
        return s
      },
      storage,
      { onMessage: () => {}, onStatusChange: () => {} },
    )

    conn.connect()
    sockets[0].onopen?.(undefined)

    expect(sockets[0].sent).toEqual([JSON.stringify({ type: 'Resume', session_id: 'saved-session' })])
  })

  it('saves the session id from a Snapshot message and forwards the parsed message', () => {
    const sockets: FakeWebSocket[] = []
    const storage = memoryStorage()
    const messages: unknown[] = []
    const conn = new Connection(
      'ws://x',
      () => {
        const s = new FakeWebSocket()
        sockets.push(s)
        return s
      },
      storage,
      { onMessage: (m) => messages.push(m), onStatusChange: () => {} },
    )

    conn.connect()
    const raw = JSON.stringify({
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'new-session', conveyor: { running: true }, robots: [],
    })
    sockets[0].onmessage?.({ data: raw })

    expect(storage.getItem('gamerobotfactory.session_id')).toBe('new-session')
    expect(messages).toHaveLength(1)
  })

  it('reconnects with exponential backoff after an unexpected close', () => {
    const sockets: FakeWebSocket[] = []
    const scheduled: Array<{ delayMs: number; fn: () => void }> = []
    const conn = new Connection(
      'ws://x',
      () => {
        const s = new FakeWebSocket()
        sockets.push(s)
        return s
      },
      memoryStorage(),
      { onMessage: () => {}, onStatusChange: () => {} },
      (delayMs, fn) => scheduled.push({ delayMs, fn }),
    )

    conn.connect()
    sockets[0].onclose?.(undefined)

    expect(scheduled).toHaveLength(1)
    expect(scheduled[0].delayMs).toBe(500)

    scheduled[0].fn() // 재연결 실행
    sockets[1].onclose?.(undefined) // 두 번째도 실패

    expect(scheduled).toHaveLength(2)
    expect(scheduled[1].delayMs).toBe(1000) // 지수 백오프: 500 -> 1000
  })

  it('does not reconnect after the user calls close()', () => {
    const sockets: FakeWebSocket[] = []
    const scheduled: Array<() => void> = []
    const conn = new Connection(
      'ws://x',
      () => {
        const s = new FakeWebSocket()
        sockets.push(s)
        return s
      },
      memoryStorage(),
      { onMessage: () => {}, onStatusChange: () => {} },
      (_delayMs, fn) => scheduled.push(fn),
    )

    conn.connect()
    conn.close()
    sockets[0].onclose?.(undefined)

    expect(scheduled).toHaveLength(0)
    expect(sockets[0].closed).toBe(true)
  })
})
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cd client && npm test`
Expected: FAIL — `Cannot find module '../../src/net/connection'`

- [ ] **Step 3: `net/connection.ts` 구현**

```ts
// client/src/net/connection.ts
import { parseServerMessage, encodeClientCommand } from './protocol'
import type { ClientCommand, ServerMessage } from './protocol'

export type ConnectionStatus =
  | { kind: 'connecting' }
  | { kind: 'open' }
  | { kind: 'reconnecting'; attempt: number }

export interface ConnectionCallbacks {
  onMessage: (message: ServerMessage) => void
  onStatusChange: (status: ConnectionStatus) => void
}

/** 브라우저 `WebSocket`과 이 인터페이스만 맞으면 되므로, 테스트에서는
 * 진짜 소켓 없이 이 형태만 흉내내는 가짜 객체를 쓴다. */
export interface WebSocketLike {
  send(data: string): void
  close(): void
  onopen: ((this: WebSocketLike, ev: unknown) => void) | null
  onmessage: ((this: WebSocketLike, ev: { data: string }) => void) | null
  onclose: ((this: WebSocketLike, ev: unknown) => void) | null
  onerror: ((this: WebSocketLike, ev: unknown) => void) | null
}

export type WebSocketFactory = (url: string) => WebSocketLike

export interface SessionStorageLike {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
}

const SESSION_STORAGE_KEY = 'gamerobotfactory.session_id'
const BASE_RECONNECT_DELAY_MS = 500
const MAX_RECONNECT_DELAY_MS = 8000

export class Connection {
  private socket: WebSocketLike | null = null
  private reconnectAttempt = 0
  private closedByUser = false

  constructor(
    private readonly url: string,
    private readonly factory: WebSocketFactory,
    private readonly storage: SessionStorageLike,
    private readonly callbacks: ConnectionCallbacks,
    private readonly scheduleReconnect: (delayMs: number, fn: () => void) => void = (delayMs, fn) => setTimeout(fn, delayMs),
  ) {}

  connect(): void {
    this.closedByUser = false
    this.callbacks.onStatusChange(
      this.reconnectAttempt === 0 ? { kind: 'connecting' } : { kind: 'reconnecting', attempt: this.reconnectAttempt },
    )
    const socket = this.factory(this.url)
    this.socket = socket

    socket.onopen = () => {
      this.reconnectAttempt = 0
      this.callbacks.onStatusChange({ kind: 'open' })
      const savedSessionId = this.storage.getItem(SESSION_STORAGE_KEY)
      if (savedSessionId) {
        this.send({ type: 'Resume', session_id: savedSessionId })
      }
    }

    socket.onmessage = (ev) => {
      const message = parseServerMessage(ev.data)
      if (!message) {
        return // 서버의 "잘못된 메시지는 로그만 남기고 연결 유지" 정책과 대칭
      }
      if (message.kind === 'Snapshot') {
        this.storage.setItem(SESSION_STORAGE_KEY, message.session_id)
      }
      this.callbacks.onMessage(message)
    }

    socket.onclose = () => {
      if (this.closedByUser) {
        return
      }
      this.reconnectAttempt += 1
      const delay = Math.min(BASE_RECONNECT_DELAY_MS * 2 ** (this.reconnectAttempt - 1), MAX_RECONNECT_DELAY_MS)
      this.callbacks.onStatusChange({ kind: 'reconnecting', attempt: this.reconnectAttempt })
      this.scheduleReconnect(delay, () => this.connect())
    }

    socket.onerror = () => {
      // onclose가 뒤따라 호출되므로 재연결 스케줄링은 onclose에 맡긴다.
    }
  }

  send(command: ClientCommand): void {
    this.socket?.send(encodeClientCommand(command))
  }

  close(): void {
    this.closedByUser = true
    this.socket?.close()
  }
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cd client && npm test`
Expected: PASS (전체 스위트)

- [ ] **Step 5: 커밋**

```bash
git add client/src/net/connection.ts client/tests/unit/connection.test.ts
git commit -m "feat: 세션 저장/지수 백오프 재접속을 포함한 WS 연결 래퍼 추가"
```

---

### Task 9: 캔버스 드로우 (`render/canvas.ts`)

바닥 타일 + 장식용 U자 컨베이어 + z-order 정렬된 로봇(몸체/다리/팔) + 경로 디버그 라인. 실제 픽셀 결과의 최종 검증은 Task 13(Playwright E2E)에서 하고, 여기서는 z-order 정렬과 "어느 칸이 컨베이어인지" 같은 순수 로직만 유닛테스트한다.

**Files:**
- Create: `client/src/render/canvas.ts`
- Create: `client/tests/unit/canvas.test.ts`

- [ ] **Step 1: 실패하는 테스트 작성**

`client/tests/unit/canvas.test.ts`:

```ts
import { describe, it, expect } from 'vitest'
import { isConveyorCell, sortRobotsForDrawing } from '../../src/render/canvas'
import type { InterpolatedRobot } from '../../src/state/interpolation'

function robotAt(id: number, x: number, y: number): InterpolatedRobot {
  return {
    id,
    pos: { x, y },
    renderPos: { x, y },
    pose: 'Standing',
    leg_cycle_progress: 0,
    task: 'Idle',
    status: { kind: 'Operational' },
    durability_remaining: 1,
    path: [],
    facing: 'East',
    arm_pose: { shoulder_angle: 0, elbow_angle: 0 },
  }
}

describe('isConveyorCell', () => {
  const grid = { width: 7, height: 6 }

  it('marks the top row, left column, and bottom row as belt', () => {
    expect(isConveyorCell(grid, 3, 0)).toBe(true)
    expect(isConveyorCell(grid, 0, 3)).toBe(true)
    expect(isConveyorCell(grid, 3, 5)).toBe(true)
  })

  it('leaves the right column and interior open (not belt) — U opens toward the sidebar', () => {
    expect(isConveyorCell(grid, 6, 3)).toBe(false)
    expect(isConveyorCell(grid, 3, 3)).toBe(false)
  })
})

describe('sortRobotsForDrawing', () => {
  it('orders robots from smallest to largest z-order key so nearer robots draw last (on top)', () => {
    const far = robotAt(1, 5, 5)
    const near = robotAt(2, 0, 0)
    const mid = robotAt(3, 2, 2)

    const sorted = sortRobotsForDrawing([far, near, mid])

    expect(sorted.map((r) => r.id)).toEqual([2, 3, 1])
  })
})
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cd client && npm test`
Expected: FAIL — `Cannot find module '../../src/render/canvas'`

- [ ] **Step 3: `render/canvas.ts` 구현**

```ts
// client/src/render/canvas.ts
import { gridToScreen, zOrderKey, wristWorldOffset, TILE_WIDTH, TILE_HEIGHT } from './projection'
import type { InterpolatedRobot } from '../state/interpolation'
import type { ConveyorView } from '../net/protocol'

export interface GridSize {
  width: number
  height: number
}

/** U자형 컨베이어 장식이 차지하는 칸 — 위/왼쪽/아래 세 변, 오른쪽(사이드바
 * 쪽) 개방(브레인스토밍에서 확정, 설계문서 "컨베이어 시각화" 절). 서버는
 * 이 개념을 전혀 모른다 — 순수 클라이언트 배경 장식이고 로봇 이동/작업에
 * 아무 영향도 주지 않는다. */
export function isConveyorCell(grid: GridSize, x: number, y: number): boolean {
  return y === 0 || y === grid.height - 1 || x === 0
}

/** z-order 오름차순 — 화면 안쪽(작은 x+y)부터 그려서, 앞쪽(큰 x+y) 로봇이
 * 나중에 그려져 위에 겹치게 한다(마스터 설계문서 line 120). */
export function sortRobotsForDrawing(robots: InterpolatedRobot[]): InterpolatedRobot[] {
  return [...robots].sort((a, b) => {
    const keyA = zOrderKey({
      pos: a.renderPos, facing: a.facing, shoulderAngle: a.arm_pose.shoulder_angle, elbowAngle: a.arm_pose.elbow_angle,
    })
    const keyB = zOrderKey({
      pos: b.renderPos, facing: b.facing, shoulderAngle: b.arm_pose.shoulder_angle, elbowAngle: b.arm_pose.elbow_angle,
    })
    return keyA - keyB
  })
}

export interface DrawSceneInput {
  grid: GridSize
  conveyor: ConveyorView
  robots: InterpolatedRobot[]
  showPaths: boolean
  animationTimeMs: number
  selectedRobotId: number | null
}

export function drawScene(ctx: CanvasRenderingContext2D, canvasWidth: number, canvasHeight: number, input: DrawSceneInput): void {
  ctx.clearRect(0, 0, canvasWidth, canvasHeight)
  ctx.save()
  ctx.translate(canvasWidth / 2, 40)

  drawFloor(ctx, input.grid, input.conveyor, input.animationTimeMs)

  for (const robot of sortRobotsForDrawing(input.robots)) {
    if (input.showPaths) {
      drawPath(ctx, robot)
    }
    drawRobot(ctx, robot, robot.id === input.selectedRobotId)
  }

  ctx.restore()
}

function drawFloor(ctx: CanvasRenderingContext2D, grid: GridSize, conveyor: ConveyorView, animationTimeMs: number): void {
  for (let y = 0; y < grid.height; y++) {
    for (let x = 0; x < grid.width; x++) {
      const screen = gridToScreen(x, y)
      drawTile(ctx, screen.x, screen.y, isConveyorCell(grid, x, y), conveyor.running, animationTimeMs)
    }
  }
}

function drawTile(ctx: CanvasRenderingContext2D, sx: number, sy: number, isBelt: boolean, running: boolean, animationTimeMs: number): void {
  ctx.save()
  ctx.translate(sx, sy)
  ctx.beginPath()
  ctx.moveTo(0, -TILE_HEIGHT / 2)
  ctx.lineTo(TILE_WIDTH / 2, 0)
  ctx.lineTo(0, TILE_HEIGHT / 2)
  ctx.lineTo(-TILE_WIDTH / 2, 0)
  ctx.closePath()

  const gradient = ctx.createLinearGradient(-TILE_WIDTH / 2, 0, TILE_WIDTH / 2, 0)
  if (isBelt) {
    gradient.addColorStop(0, '#5b84c9')
    gradient.addColorStop(1, '#33538f')
  } else {
    gradient.addColorStop(0, '#4a9d6f')
    gradient.addColorStop(1, '#2c6b47')
  }
  ctx.fillStyle = gradient
  ctx.fill()
  ctx.strokeStyle = 'rgba(0,0,0,0.3)'
  ctx.stroke()

  if (isBelt && running) {
    // 흐르는 느낌의 대각선 스트라이프 — 시간에 따라 위치가 이동한다.
    // 정지 상태(running=false)면 스트라이프를 그리지 않아 정적으로 보인다.
    ctx.save()
    ctx.clip()
    const stripeOffset = (animationTimeMs / 20) % 12
    ctx.strokeStyle = 'rgba(255,255,255,0.35)'
    ctx.lineWidth = 3
    for (let i = -TILE_WIDTH; i < TILE_WIDTH; i += 12) {
      ctx.beginPath()
      ctx.moveTo(i + stripeOffset, -TILE_HEIGHT / 2)
      ctx.lineTo(i + stripeOffset + TILE_HEIGHT / 2, TILE_HEIGHT / 2)
      ctx.stroke()
    }
    ctx.restore()
  }
  ctx.restore()
}

function drawRobot(ctx: CanvasRenderingContext2D, robot: InterpolatedRobot, selected: boolean): void {
  const screen = gridToScreen(robot.renderPos.x, robot.renderPos.y)
  const wrist = wristWorldOffset({
    pos: robot.renderPos, facing: robot.facing, shoulderAngle: robot.arm_pose.shoulder_angle, elbowAngle: robot.arm_pose.elbow_angle,
  })
  const wristScreen = gridToScreen(wrist.x, wrist.y)
  const bodyLift = robot.pose === 'Crouching' ? 6 : 12 // 자세에 따른 몸체 높이(화면 픽셀, 튜닝 대상)

  ctx.save()
  ctx.translate(screen.x, screen.y)

  ctx.strokeStyle = '#6b4810'
  ctx.lineWidth = 3
  for (let i = 0; i < 4; i++) {
    const phase = (robot.leg_cycle_progress + i * 0.25) % 1
    const legX = (i < 2 ? -8 : 8) + (phase < 0.5 ? -3 : 3)
    ctx.beginPath()
    ctx.moveTo(legX, -bodyLift)
    ctx.lineTo(legX, -bodyLift + 8)
    ctx.stroke()
  }

  const bodyGradient = ctx.createLinearGradient(-11, -bodyLift - 8, 11, -bodyLift + 8)
  bodyGradient.addColorStop(0, '#ffd27a')
  bodyGradient.addColorStop(1, '#d99a2e')
  ctx.fillStyle = bodyGradient
  ctx.fillRect(-11, -bodyLift - 8, 22, 16)
  if (selected) {
    ctx.strokeStyle = '#ffffff'
    ctx.lineWidth = 2
    ctx.strokeRect(-11, -bodyLift - 8, 22, 16)
  }

  ctx.strokeStyle = '#a06f1a'
  ctx.lineWidth = 3
  ctx.beginPath()
  ctx.moveTo(0, -bodyLift)
  ctx.lineTo(wristScreen.x - screen.x, wristScreen.y - screen.y - bodyLift)
  ctx.stroke()

  ctx.restore()
}

function drawPath(ctx: CanvasRenderingContext2D, robot: InterpolatedRobot): void {
  if (robot.path.length === 0) {
    return
  }
  ctx.save()
  ctx.strokeStyle = 'rgba(93, 214, 255, 0.7)'
  ctx.lineWidth = 2
  ctx.setLineDash([4, 4])
  ctx.beginPath()
  const start = gridToScreen(robot.renderPos.x, robot.renderPos.y)
  ctx.moveTo(start.x, start.y)
  for (const cell of robot.path) {
    const p = gridToScreen(cell.x, cell.y)
    ctx.lineTo(p.x, p.y)
  }
  ctx.stroke()
  ctx.restore()
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cd client && npm test`
Expected: PASS (전체 스위트)

- [ ] **Step 5: 커밋**

```bash
git add client/src/render/canvas.ts client/tests/unit/canvas.test.ts
git commit -m "feat: 캔버스 드로우(바닥/컨베이어/로봇/경로 디버그) 추가"
```

---

### Task 10: 사이드바 UI (`ui/sidebar.ts`)

우측 고정폭 사이드바 — 전역 컨트롤, 선택된 로봇 패널, 경로 디버그 토글(설계문서 "UI/HUD" 절). `jsdom` 환경에서 실제 DOM을 만들고 검증한다.

**Files:**
- Create: `client/src/ui/sidebar.ts`
- Create: `client/tests/unit/sidebar.test.ts`

- [ ] **Step 1: 실패하는 테스트 작성**

`client/tests/unit/sidebar.test.ts` (파일 맨 첫 줄의 `// @vitest-environment jsdom` 독블록이 이 파일만 jsdom 환경으로 돌게 한다 — 다른 유닛테스트는 계속 순수 `node` 환경 유지):

```ts
// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { Sidebar } from '../../src/ui/sidebar'
import type { SidebarCallbacks } from '../../src/ui/sidebar'
import type { RobotView } from '../../src/net/protocol'

function robot(overrides: Partial<RobotView> = {}): RobotView {
  return {
    id: 1,
    pos: { x: 0, y: 0 },
    pose: 'Standing',
    leg_cycle_progress: 0,
    task: 'Idle',
    status: { kind: 'Operational' },
    durability_remaining: 0.8,
    path: [],
    facing: 'East',
    arm_pose: { shoulder_angle: 0, elbow_angle: 0 },
    ...overrides,
  }
}

function makeSidebar(container: HTMLElement, overrides: Partial<SidebarCallbacks> = {}): Sidebar {
  const callbacks: SidebarCallbacks = {
    onToggleConveyor: vi.fn(),
    onChangeRobotCount: vi.fn(),
    onSelectArmAction: vi.fn(),
    onRepair: vi.fn(),
    onTogglePathDebug: vi.fn(),
    ...overrides,
  }
  return new Sidebar(container, callbacks)
}

describe('Sidebar', () => {
  it('shows "선택된 로봇 없음" when nothing is selected', () => {
    const container = document.createElement('div')
    const sidebar = makeSidebar(container)

    sidebar.update({ connection: { kind: 'open' }, conveyor: { running: true }, robotCount: 3, selectedRobot: null, pathDebugEnabled: false })

    expect(container.textContent).toContain('선택된 로봇 없음')
  })

  it('renders the selected robot battery/task', () => {
    const container = document.createElement('div')
    const sidebar = makeSidebar(container)

    sidebar.update({
      connection: { kind: 'open' }, conveyor: { running: true }, robotCount: 1,
      selectedRobot: robot({ durability_remaining: 0.45, task: 'Picking' }), pathDebugEnabled: false,
    })

    expect(container.textContent).toContain('45%')
    expect(container.textContent).toContain('Picking')
  })

  it('disables the repair button unless the robot is Failed', () => {
    const container = document.createElement('div')
    const sidebar = makeSidebar(container)

    sidebar.update({ connection: { kind: 'open' }, conveyor: { running: true }, robotCount: 1, selectedRobot: robot(), pathDebugEnabled: false })
    const repairButtonWhileOperational = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === '수리') as HTMLButtonElement
    expect(repairButtonWhileOperational.disabled).toBe(true)

    sidebar.update({
      connection: { kind: 'open' }, conveyor: { running: true }, robotCount: 1,
      selectedRobot: robot({ status: { kind: 'Failed' } }), pathDebugEnabled: false,
    })
    const repairButtonWhileFailed = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === '수리') as HTMLButtonElement
    expect(repairButtonWhileFailed.disabled).toBe(false)
  })

  it('calls onRepair when the repair button is clicked', () => {
    const container = document.createElement('div')
    const onRepair = vi.fn()
    const sidebar = makeSidebar(container, { onRepair })

    sidebar.update({
      connection: { kind: 'open' }, conveyor: { running: true }, robotCount: 1,
      selectedRobot: robot({ status: { kind: 'Failed' } }), pathDebugEnabled: false,
    })
    const repairButton = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === '수리') as HTMLButtonElement
    repairButton.click()

    expect(onRepair).toHaveBeenCalledOnce()
  })

  it('shows the reconnecting attempt count', () => {
    const container = document.createElement('div')
    const sidebar = makeSidebar(container)

    sidebar.update({
      connection: { kind: 'reconnecting', attempt: 3 }, conveyor: { running: true }, robotCount: 0, selectedRobot: null, pathDebugEnabled: false,
    })

    expect(container.textContent).toContain('3')
  })

  it('calls onToggleConveyor when the conveyor button is clicked', () => {
    const container = document.createElement('div')
    const onToggleConveyor = vi.fn()
    const sidebar = makeSidebar(container, { onToggleConveyor })

    sidebar.update({ connection: { kind: 'open' }, conveyor: { running: true }, robotCount: 0, selectedRobot: null, pathDebugEnabled: false })
    const button = Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.includes('컨베이어')) as HTMLButtonElement
    button.click()

    expect(onToggleConveyor).toHaveBeenCalledOnce()
  })
})
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cd client && npm test`
Expected: FAIL — `Cannot find module '../../src/ui/sidebar'`

- [ ] **Step 3: `ui/sidebar.ts` 구현**

```ts
// client/src/ui/sidebar.ts
import type { ConveyorView, RobotView } from '../net/protocol'
import type { ConnectionStatus } from '../net/connection'

export interface SidebarCallbacks {
  onToggleConveyor: () => void
  onChangeRobotCount: (delta: number) => void
  onSelectArmAction: (task: 'Idle' | 'Picking' | 'Placing') => void
  onRepair: () => void
  onTogglePathDebug: (enabled: boolean) => void
}

export interface SidebarState {
  connection: ConnectionStatus
  conveyor: ConveyorView
  robotCount: number
  selectedRobot: RobotView | null
  pathDebugEnabled: boolean
}

export class Sidebar {
  private readonly connectionEl: HTMLElement
  private readonly conveyorButton: HTMLButtonElement
  private readonly robotCountEl: HTMLElement
  private readonly pathToggle: HTMLInputElement
  private readonly selectedPanel: HTMLElement
  private readonly callbacks: SidebarCallbacks

  constructor(container: HTMLElement, callbacks: SidebarCallbacks) {
    this.callbacks = callbacks

    const root = document.createElement('div')
    root.className = 'sidebar'
    container.appendChild(root)

    const globalSection = document.createElement('section')
    this.connectionEl = document.createElement('p')
    this.connectionEl.className = 'connection-status'
    globalSection.appendChild(this.connectionEl)

    this.conveyorButton = document.createElement('button')
    this.conveyorButton.addEventListener('click', () => callbacks.onToggleConveyor())
    globalSection.appendChild(this.conveyorButton)

    const decButton = document.createElement('button')
    decButton.textContent = '-'
    decButton.addEventListener('click', () => callbacks.onChangeRobotCount(-1))
    const incButton = document.createElement('button')
    incButton.textContent = '+'
    incButton.addEventListener('click', () => callbacks.onChangeRobotCount(1))
    this.robotCountEl = document.createElement('span')
    this.robotCountEl.className = 'robot-count'
    globalSection.appendChild(decButton)
    globalSection.appendChild(this.robotCountEl)
    globalSection.appendChild(incButton)

    const pathLabel = document.createElement('label')
    this.pathToggle = document.createElement('input')
    this.pathToggle.type = 'checkbox'
    this.pathToggle.addEventListener('change', () => callbacks.onTogglePathDebug(this.pathToggle.checked))
    pathLabel.appendChild(this.pathToggle)
    pathLabel.appendChild(document.createTextNode('경로 표시'))
    globalSection.appendChild(pathLabel)

    root.appendChild(globalSection)

    this.selectedPanel = document.createElement('section')
    this.selectedPanel.className = 'selected-robot-panel'
    root.appendChild(this.selectedPanel)
  }

  update(state: SidebarState): void {
    this.connectionEl.textContent = connectionStatusLabel(state.connection)
    this.conveyorButton.textContent = state.conveyor.running ? '컨베이어 끄기' : '컨베이어 켜기'
    this.robotCountEl.textContent = String(state.robotCount)
    this.pathToggle.checked = state.pathDebugEnabled

    this.selectedPanel.innerHTML = ''
    if (!state.selectedRobot) {
      const empty = document.createElement('p')
      empty.textContent = '선택된 로봇 없음'
      this.selectedPanel.appendChild(empty)
      return
    }

    const robot = state.selectedRobot
    const info = document.createElement('p')
    info.textContent = `로봇 #${robot.id} · 배터리 ${Math.round(robot.durability_remaining * 100)}% · ${robot.task} · ${statusLabel(robot.status)}`
    this.selectedPanel.appendChild(info)

    for (const task of ['Idle', 'Picking', 'Placing'] as const) {
      const button = document.createElement('button')
      button.textContent = task
      button.disabled = robot.status.kind !== 'Operational'
      button.addEventListener('click', () => this.callbacks.onSelectArmAction(task))
      this.selectedPanel.appendChild(button)
    }

    const repairButton = document.createElement('button')
    repairButton.textContent = '수리'
    repairButton.disabled = robot.status.kind !== 'Failed'
    repairButton.addEventListener('click', () => this.callbacks.onRepair())
    this.selectedPanel.appendChild(repairButton)
  }
}

function connectionStatusLabel(status: ConnectionStatus): string {
  switch (status.kind) {
    case 'connecting':
      return '연결 중...'
    case 'open':
      return '🟢 연결됨'
    case 'reconnecting':
      return `🔴 재연결 중... (시도 ${status.attempt})`
  }
}

function statusLabel(status: RobotView['status']): string {
  switch (status.kind) {
    case 'Operational':
      return 'Operational'
    case 'Failed':
      return 'Failed'
    case 'Repairing':
      return `Repairing (${status.remaining_ticks})`
  }
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cd client && npm test`
Expected: PASS (전체 스위트)

- [ ] **Step 5: 타입체크**

Run: `npm run typecheck`
Expected: 에러 없음(테스트 파일의 `makeSidebar` 헬퍼 타입 포함)

- [ ] **Step 6: 커밋**

```bash
git add client/src/ui/sidebar.ts client/tests/unit/sidebar.test.ts
git commit -m "feat: 우측 사이드바 UI(전역 컨트롤/선택 로봇 패널/경로 토글) 추가"
```

---

### Task 11: 부트스트랩 (`main.ts`) — 전체 배선 + 실제 서버로 수동 확인

지금까지 만든 net/state/render/ui 모듈을 실제 브라우저 API(`WebSocket`, `sessionStorage`, `requestAnimationFrame`, `CanvasRenderingContext2D`)에 연결한다. WS 접속 대상은 쿼리 파라미터(`?ws=ws://127.0.0.1:<포트>/ws`)로 받는다 — 서버가 항상 임의 포트(`127.0.0.1:0`)로 뜨므로(README 참고) 고정 기본값을 추측해서 넣는 대신, 없으면 명확한 안내 문구를 보여준다.

**Files:**
- Modify: `client/src/main.ts`

- [ ] **Step 1: `main.ts` 구현**

```ts
// client/src/main.ts
import { Connection } from './net/connection'
import type { ConnectionStatus } from './net/connection'
import { createEmptyMirror, applyServerMessage } from './state/mirror'
import type { MirrorState } from './state/mirror'
import { computeRenderRobots } from './state/interpolation'
import type { TickSnapshot } from './state/interpolation'
import { drawScene } from './render/canvas'
import { Sidebar } from './ui/sidebar'
import type { ServerMessage } from './net/protocol'

// U자 컨베이어가 이소메트릭 각도에서 알아볼 수 있는 최소 크기(설계문서
// "컨베이어 시각화" 절 — 브레인스토밍 목업 기준 7x6 이상 권장).
const GRID_SIZE = { width: 9, height: 7 }

function resolveWsUrl(): string | null {
  return new URLSearchParams(location.search).get('ws')
}

function setupLayout(): { canvas: HTMLCanvasElement; sidebarContainer: HTMLElement } {
  const app = document.getElementById('app')
  if (!app) {
    throw new Error('#app element not found')
  }
  app.innerHTML = ''

  const canvas = document.createElement('canvas')
  canvas.style.flex = '1'
  app.appendChild(canvas)

  const sidebarContainer = document.createElement('div')
  sidebarContainer.style.width = '280px'
  sidebarContainer.style.flexShrink = '0'
  app.appendChild(sidebarContainer)

  function resizeCanvas(): void {
    canvas.width = canvas.clientWidth
    canvas.height = canvas.clientHeight
  }
  window.addEventListener('resize', resizeCanvas)
  resizeCanvas()

  return { canvas, sidebarContainer }
}

function main(): void {
  const wsUrl = resolveWsUrl()
  const { canvas, sidebarContainer } = setupLayout()
  const ctx = canvas.getContext('2d')
  if (!ctx) {
    throw new Error('2D canvas context unavailable')
  }

  if (!wsUrl) {
    sidebarContainer.textContent = '서버 WS URL이 지정되지 않았습니다 — ?ws=ws://127.0.0.1:<포트>/ws 로 접속하세요.'
    return
  }

  let mirror: MirrorState = createEmptyMirror()
  let prevSnapshot: TickSnapshot | null = null
  let currSnapshot: TickSnapshot | null = null
  let connectionStatus: ConnectionStatus = { kind: 'connecting' }
  let selectedRobotId: number | null = null
  let pathDebugEnabled = false

  const sidebar = new Sidebar(sidebarContainer, {
    onToggleConveyor: () => connection.send({ type: 'ToggleConveyor' }),
    onChangeRobotCount: (delta) => {
      const nextCount = Math.max(0, mirror.robots.size + delta)
      connection.send({ type: 'SetRobotCount', count: nextCount })
    },
    onSelectArmAction: (task) => {
      if (selectedRobotId !== null) {
        connection.send({ type: 'TriggerArmAction', robot_id: selectedRobotId, task })
      }
    },
    onRepair: () => {
      if (selectedRobotId !== null) {
        connection.send({ type: 'RepairRobot', robot_id: selectedRobotId })
      }
    },
    onTogglePathDebug: (enabled) => {
      pathDebugEnabled = enabled
    },
  })

  function handleMessage(message: ServerMessage): void {
    mirror = applyServerMessage(mirror, message)
    if (message.kind === 'Snapshot' || message.kind === 'Delta') {
      prevSnapshot = currSnapshot
      currSnapshot = { mirror, receivedAtMs: performance.now() }
    }
  }

  const connection = new Connection(wsUrl, (url) => new WebSocket(url), window.sessionStorage, {
    onMessage: handleMessage,
    onStatusChange: (status) => {
      connectionStatus = status
    },
  })
  connection.connect()

  canvas.addEventListener('click', (ev) => {
    if (!currSnapshot) return
    const rect = canvas.getBoundingClientRect()
    const clickX = ev.clientX - rect.left - canvas.width / 2
    const clickY = ev.clientY - rect.top - 40
    const rendered = computeRenderRobots(prevSnapshot, currSnapshot, performance.now())

    // 가장 가까운 로봇을 선택한다 — 로봇 수가 적은 v1 스코프에서는 정밀한
    // 폴리곤 히트테스트 없이 화면 좌표 거리만으로 충분히 정확하다.
    let closestId: number | null = null
    let closestDist = Infinity
    for (const robot of rendered) {
      const screenX = (robot.renderPos.x - robot.renderPos.y) * 32
      const screenY = (robot.renderPos.x + robot.renderPos.y) * 16
      const dist = Math.hypot(screenX - clickX, screenY - clickY)
      if (dist < closestDist) {
        closestDist = dist
        closestId = robot.id
      }
    }
    if (closestId !== null && closestDist < 24) {
      selectedRobotId = closestId
      connection.send({ type: 'SelectRobot', robot_id: closestId })
    }
  })

  function frame(): void {
    const now = performance.now()
    const rendered = currSnapshot ? computeRenderRobots(prevSnapshot, currSnapshot, now) : []
    drawScene(ctx, canvas.width, canvas.height, {
      grid: GRID_SIZE,
      conveyor: mirror.conveyor,
      robots: rendered,
      showPaths: pathDebugEnabled,
      animationTimeMs: now,
      selectedRobotId,
    })
    sidebar.update({
      connection: connectionStatus,
      conveyor: mirror.conveyor,
      robotCount: mirror.robots.size,
      selectedRobot: selectedRobotId !== null ? (mirror.robots.get(selectedRobotId) ?? null) : null,
      pathDebugEnabled,
    })
    requestAnimationFrame(frame)
  }
  requestAnimationFrame(frame)
}

main()
```

- [ ] **Step 2: 타입체크 + 빌드 확인**

Run: `cd client && npm run typecheck`
Expected: 에러 없음

Run: `npm run build`
Expected: `dist/` 생성, 에러 없음

- [ ] **Step 3: 실제 서버로 수동 확인**

다른 터미널에서 서버 기동:

```bash
cargo run --manifest-path server/Cargo.toml
```

`LISTENING_PORT=<포트>` 줄에서 포트를 확인한다. 클라이언트 dev 서버 기동:

```bash
cd client && npm run dev
```

브라우저에서 `http://localhost:5173/?ws=ws://127.0.0.1:<포트>/ws` 접속(포트는 위에서 확인한 값으로 교체). 확인 항목:
- 콘솔에 에러 없이 연결됨(사이드바에 🟢 연결됨 표시)
- 사이드바 `+` 버튼으로 로봇 수를 늘리면 캔버스에 로봇이 나타남
- 로봇 클릭 시 사이드바에 선택된 로봇 정보(배터리/작업/상태) 표시
- 컨베이어 토글 버튼이 배경 스트라이프 애니메이션을 켜고 끔
- 팔 동작 버튼(Picking 등) 클릭 시 로봇 자세가 바뀜(웅크림)

- [ ] **Step 4: 커밋**

```bash
git add client/src/main.ts
git commit -m "feat: net/state/render/ui 모듈을 실제 브라우저 API에 배선"
```

---

### Task 12: 통합테스트 — 실제 서버 바이너리 + 진짜 WS (`tests/integration/`)

서버 통합테스트(`server/tests/ws_integration.rs`)와 동일한 패턴: `cargo build`로 만든 실제 서버 바이너리를 자식 프로세스로 기동하고, Node `ws` 패키지로 진짜 WS 연결을 맺어 클라이언트의 상태 레이어(파싱/미러/필드)가 실제 서버 출력과 맞는지 검증한다. 모킹 없음(설계문서 "테스트 전략" 절).

**Files:**
- Create: `client/tests/helpers/spawn-server.ts`
- Create: `client/tests/integration/session.test.ts`

- [ ] **Step 1: 서버 바이너리 빌드(사전 조건)**

Run: `cargo build --manifest-path server/Cargo.toml`
Expected: `server/target/debug/server`(Windows는 `server.exe`) 생성됨

- [ ] **Step 2: 서버 프로세스 스폰 헬퍼 작성**

`client/tests/helpers/spawn-server.ts` — Rust 쪽 `ServerProcess`(`server/tests/ws_integration.rs:8-43`)와 같은 패턴(임의 포트 + `LISTENING_PORT=` announce 줄 읽기 + 격리된 SQLite 경로):

```ts
// client/tests/helpers/spawn-server.ts
import { spawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'
import { createInterface } from 'node:readline'
import path from 'node:path'
import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'

export interface SpawnedServer {
  process: ChildProcess
  port: number
}

function resolveServerBinaryPath(): string {
  const exeName = process.platform === 'win32' ? 'server.exe' : 'server'
  return path.resolve(__dirname, '../../../server/target/debug', exeName)
}

/** 서버 바이너리를 임의 포트로 띄우고, 표준출력의 `LISTENING_PORT={port}`
 * 줄에서 실제 포트를 읽는다. 테스트마다 격리된 임시 SQLite 경로를 써서
 * 병렬로 돌려도 서로 간섭하지 않는다(서버 쪽 `rest_integration.rs`와
 * 동일한 이유). */
export async function spawnServer(): Promise<SpawnedServer> {
  const dbDir = mkdtempSync(path.join(tmpdir(), 'gamerobotfactory-client-test-'))
  const dbPath = path.join(dbDir, 'test.sqlite3')

  const child = spawn(resolveServerBinaryPath(), [], {
    env: { ...process.env, GAMEROBOTFACTORY_DB_PATH: dbPath },
    stdio: ['ignore', 'pipe', 'ignore'],
  })

  const port = await new Promise<number>((resolve, reject) => {
    if (!child.stdout) {
      reject(new Error('server stdout was not piped'))
      return
    }
    const rl = createInterface({ input: child.stdout })
    const timeout = setTimeout(() => {
      rl.close()
      reject(new Error('timed out waiting for LISTENING_PORT announce line'))
    }, 10000)
    rl.on('line', (line) => {
      const match = /^LISTENING_PORT=(\d+)$/.exec(line.trim())
      if (match) {
        clearTimeout(timeout)
        rl.close()
        resolve(Number(match[1]))
      }
    })
    child.on('error', (err) => {
      clearTimeout(timeout)
      reject(err)
    })
  })

  return { process: child, port }
}

export function stopServer(server: SpawnedServer): void {
  server.process.kill()
}
```

- [ ] **Step 3: 통합테스트 작성**

`client/tests/integration/session.test.ts`:

```ts
import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import WebSocket from 'ws'
import { spawnServer, stopServer } from '../helpers/spawn-server'
import type { SpawnedServer } from '../helpers/spawn-server'
import { parseServerMessage, encodeClientCommand } from '../../src/net/protocol'
import type { ServerMessage } from '../../src/net/protocol'
import { applyServerMessage, createEmptyMirror } from '../../src/state/mirror'
import type { MirrorState } from '../../src/state/mirror'

let server: SpawnedServer

beforeAll(async () => {
  server = await spawnServer()
})

afterAll(() => {
  stopServer(server)
})

function connect(port: number): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`)
    ws.once('open', () => resolve(ws))
    ws.once('error', reject)
  })
}

function nextMessage(ws: WebSocket): Promise<ServerMessage> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out waiting for a message')), 5000)
    ws.once('message', (data) => {
      clearTimeout(timeout)
      const parsed = parseServerMessage(data.toString())
      if (!parsed) {
        reject(new Error(`failed to parse message: ${data.toString()}`))
        return
      }
      resolve(parsed)
    })
  })
}

describe('client state layer against a real running server', () => {
  it('mirrors an initial empty Snapshot from the real server', async () => {
    const ws = await connect(server.port)
    const first = await nextMessage(ws)
    ws.close()

    expect(first.kind).toBe('Snapshot')
    const mirror: MirrorState = applyServerMessage(createEmptyMirror(), first)
    expect(mirror.robots.size).toBe(0)
  })

  it('reflects SetRobotCount into the local mirror, including facing/path/arm_pose', async () => {
    const ws = await connect(server.port)
    await nextMessage(ws) // 초기 스냅샷 소비

    ws.send(encodeClientCommand({ type: 'SetRobotCount', count: 2 }))

    let mirror: MirrorState = createEmptyMirror()
    const deadline = Date.now() + 5000
    while (mirror.robots.size < 2 && Date.now() < deadline) {
      const msg = await nextMessage(ws)
      mirror = applyServerMessage(mirror, msg)
    }
    ws.close()

    expect(mirror.robots.size).toBe(2)
    for (const robot of mirror.robots.values()) {
      expect(['North', 'East', 'South', 'West']).toContain(robot.facing)
      expect(Array.isArray(robot.path)).toBe(true)
      expect(typeof robot.arm_pose.shoulder_angle).toBe('number')
    }
  })

  it('resyncs after Resume with a valid session id', async () => {
    const ws1 = await connect(server.port)
    const snapshot = await nextMessage(ws1)
    if (snapshot.kind !== 'Snapshot') throw new Error('expected Snapshot')
    const sessionId = snapshot.session_id
    ws1.close()

    const ws2 = await connect(server.port)
    await nextMessage(ws2) // 재접속도 항상 새 Snapshot을 먼저 보낸다
    ws2.send(encodeClientCommand({ type: 'Resume', session_id: sessionId }))
    const ack = await nextMessage(ws2)
    ws2.close()

    expect(ack.kind).toBe('ResumeAck')
    if (ack.kind === 'ResumeAck') {
      expect(ack.resumed).toBe(true)
    }
  })
})
```

- [ ] **Step 4: 테스트 실행**

Run: `cd client && npm run test:integration`
Expected: PASS — 3개 테스트 전부(실제 서버 프로세스가 각 테스트 전에 뜨고 끝나면 종료됨)

- [ ] **Step 5: 커밋**

```bash
git add client/tests/helpers/spawn-server.ts client/tests/integration/session.test.ts
git commit -m "test: 실제 서버 바이너리+진짜 WS로 클라이언트 상태 레이어 통합테스트 추가"
```

---

### Task 13: E2E 테스트 — 실제 서버 + 빌드된 클라이언트 + 실제 브라우저 (Playwright)

전체 페이지 스크린샷 diff(폰트/OS 차이로 flaky) 대신, 알려진 좌표의 캔버스 픽셀 샘플링 + 사이드바 DOM 텍스트로 "로봇이 실제로 그 자리에 그려졌는지"를 결정적으로 검증한다(설계문서 "테스트 전략" 절). `game_state.rs::set_robot_count`(`server/src/game_state.rs:66`)가 새 로봇을 항상 `(0,0)`에 `goal`도 `(0,0)`으로 스폰하므로(`plan_robot`이 `pos==goal`이면 즉시 반환) 로봇이 절대 움직이지 않아 타이밍 경쟁 없이 결정적으로 검증 가능하다.

**Files:**
- Create: `client/tests/e2e/global-setup.ts`
- Create: `client/tests/e2e/global-teardown.ts`
- Create: `client/tests/e2e/render.spec.ts`

- [ ] **Step 1: 사전 조건 확인**

Run: `cargo build --manifest-path server/Cargo.toml` (Task 12에서 이미 했다면 생략)
Run: `cd client && npx playwright install chromium` (최초 1회, 브라우저 바이너리 설치)

- [ ] **Step 2: 전역 셋업/해제 작성**

`client/tests/e2e/global-setup.ts` — 서버를 한 번만 띄우고 포트/PID를 파일로 남긴다(Playwright의 `globalSetup`은 별도 프로세스라 `spawnServer()`가 반환한 자식 프로세스 핸들이 이후 테스트 워커로 넘어가지 않으므로, PID를 저장했다가 `globalTeardown`에서 별도로 kill한다):

```ts
// client/tests/e2e/global-setup.ts
import { writeFileSync } from 'node:fs'
import path from 'node:path'
import { spawnServer } from '../helpers/spawn-server'

const INFO_PATH = path.resolve(__dirname, '.server-info.json')

export default async function globalSetup(): Promise<void> {
  const server = await spawnServer()
  writeFileSync(INFO_PATH, JSON.stringify({ port: server.port, pid: server.process.pid }))
}
```

`client/tests/e2e/global-teardown.ts`:

```ts
// client/tests/e2e/global-teardown.ts
import { readFileSync, rmSync } from 'node:fs'
import path from 'node:path'

const INFO_PATH = path.resolve(__dirname, '.server-info.json')

export default async function globalTeardown(): Promise<void> {
  const info = JSON.parse(readFileSync(INFO_PATH, 'utf-8')) as { port: number; pid: number }
  try {
    process.kill(info.pid)
  } catch {
    // 이미 종료된 경우 무시
  }
  rmSync(INFO_PATH, { force: true })
}
```

- [ ] **Step 3: E2E 테스트 작성**

`client/tests/e2e/render.spec.ts`:

```ts
import { test, expect } from '@playwright/test'
import { readFileSync } from 'node:fs'
import path from 'node:path'

function backendPort(): number {
  const info = JSON.parse(readFileSync(path.resolve(__dirname, '.server-info.json'), 'utf-8')) as { port: number }
  return info.port
}

test.describe('client renders against a real server', () => {
  test('draws a spawned robot at its projected screen position', async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 })
    await page.goto(`/?ws=ws://127.0.0.1:${backendPort()}/ws`)

    const incButton = page.locator('.sidebar button', { hasText: '+' })
    await incButton.click()
    await expect(page.locator('.robot-count')).toHaveText('1', { timeout: 5000 })

    const canvas = page.locator('canvas')
    const box = await canvas.boundingBox()
    if (!box) throw new Error('canvas has no bounding box')

    // 로봇은 항상 (0,0)에 스폰되고 goal도 (0,0)이라 움직이지 않는다
    // (game_state.rs::set_robot_count). 아이소메트릭 투영 원점은 캔버스
    // 중앙 상단(width/2, 40)이므로 그 픽셀이 배경색이 아닌지 확인한다.
    const pixel = await page.evaluate(
      ({ x, y }) => {
        const c = document.querySelector('canvas') as HTMLCanvasElement
        const ctx = c.getContext('2d')!
        return Array.from(ctx.getImageData(x, y, 1, 1).data)
      },
      { x: Math.round(box.width / 2), y: 40 },
    )

    // 빈 바닥 타일은 초록 계열(#4a9d6f~#2c6b47), 로봇 몸체는 황토색
    // 계열(#ffd27a~#d99a2e)이라 빨간 채널이 초록 채널보다 뚜렷이 커야
    // 로봇이 실제로 그 자리에 그려졌다고 볼 수 있다.
    expect(pixel[0]).toBeGreaterThan(pixel[1])
  })

  test('shows the selected robot info in the sidebar after clicking it', async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 })
    await page.goto(`/?ws=ws://127.0.0.1:${backendPort()}/ws`)

    const incButton = page.locator('.sidebar button', { hasText: '+' })
    await incButton.click()
    await expect(page.locator('.robot-count')).toHaveText('1', { timeout: 5000 })

    const canvas = page.locator('canvas')
    const box = await canvas.boundingBox()
    if (!box) throw new Error('canvas has no bounding box')

    await page.mouse.click(box.x + box.width / 2, box.y + 40)

    await expect(page.locator('.selected-robot-panel')).toContainText('로봇 #', { timeout: 5000 })
  })
})
```

- [ ] **Step 4: 테스트 실행**

Run: `cd client && npm run test:e2e`
Expected: PASS — `playwright.config.ts`가 `npm run build && npm run preview`로 클라이언트를 띄우고, `globalSetup`이 서버를 띄운 뒤 2개 테스트 실행

- [ ] **Step 5: 커밋**

```bash
git add client/tests/e2e/global-setup.ts client/tests/e2e/global-teardown.ts client/tests/e2e/render.spec.ts
git commit -m "test: Playwright E2E — 실제 서버+브라우저로 픽셀/DOM 검증"
```

---

### Task 14: 전체 검증 + 클라이언트 성능 실측 + 문서 갱신

**Files:**
- Modify: `README.md`
- Modify: `docs/robot-arm-conveyor-game-design.md`
- Modify: `docs/KANBAN.md`

- [ ] **Step 1: 서버 전체 스위트 회귀 확인**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: PASS(Task 1-2에서 바뀐 `sim.rs`/`protocol.rs`/`delta.rs`가 기존 123개 테스트 + 이번에 추가한 테스트 전부 통과)

Run: `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings`
Expected: 경고 0개

- [ ] **Step 2: 클라이언트 전체 스위트 확인**

```bash
cd client
npm test               # 단위
npm run test:integration
npm run test:e2e
npm run typecheck
```
Expected: 전부 PASS/에러 없음. 플레이키니스 없는지 각 스위트 3회 반복 실행(서버 쪽에서 지금까지 지켜온 관행 — README "지금까지 만든 것" 참고).

- [ ] **Step 3: 클라이언트 렌더링 성능 실측**

`npm run dev`로 클라이언트를 띄우고 실제 서버에 접속한 상태에서, 사이드바로 로봇 수를 50까지 늘린다. 브라우저 개발자도구 콘솔에서 다음을 실행해 프레임 간격을 30프레임만 표본 수집한다:

```js
let last = performance.now(); let samples = [];
function sample() {
  const now = performance.now();
  samples.push(now - last);
  last = now;
  if (samples.length < 30) requestAnimationFrame(sample);
  else console.log('avg frame ms:', samples.reduce((a,b)=>a+b)/samples.length);
}
requestAnimationFrame(sample);
```

측정한 평균 프레임 시간(ms)을 기록해둔다 — 설계문서 "클라이언트 성능 목표"(로봇 50대에서 60fps 근처, 약 16.7ms/프레임)와 비교해 Step 5의 README 갱신에 실측치로 남긴다.

- [ ] **Step 4: 마스터 설계문서 v1 범위 표 갱신**

`docs/robot-arm-conveyor-game-design.md`의 v1 범위 표(134-143번째 줄 부근) 로봇 행을 다음으로 갱신 — 완료된 로봇 고장/수리 기능(2026-07-16 설계)과 이번에 실제로 연결된 자세 전환/facing이 반영 안 돼 있던 것을 고친다:

```markdown
| 로봇 | 4족 보행(스탠스/스윙 구분 프로시저럴 gait) + 긴 팔(2-본 IK, 클램프 포함), 몸체-팔 단일 기구학 체인, task 기반 자세 전환(Idle=서기/작업=웅크림), 마모/고장/수리 |
```

같은 파일의 "렌더링" 절(117-122번째 줄) 뒤에 짧은 각주를 추가해, `facing`/`arm_pose`가 실제로 프로토콜에 노출된 시점을 남긴다(선택 사항이지만, 이 문서가 "예시로만 존재하고 구현 안 된" 서술을 담고 있었다는 걸 알아챈 브레인스토밍 세션의 교훈을 재발 방지 차원에서 남겨두는 것을 권장).

- [ ] **Step 5: README 갱신**

- **프로토콜 표**(`RobotView`/`RobotView` 필드 설명 부분)에 `path`(경로 디버그용), `facing`, `arm_pose` 추가.
- **개발 환경** 절에 `client/` 디렉토리가 이제 존재함과 `cd client && npm install && npm run dev` 안내 추가, "`client/` 디렉토리는 아직 없다(Plan 4 이전)" 문장 제거.
- **플레이 안내** 절에 `wscat` 대신(또는 함께) 실제 브라우저 클라이언트로 접속하는 방법 추가.
- **지금까지 만든 것**에 "Plan 4 — 클라이언트 렌더링" 항목 추가: 아이소메트릭 렌더링(쉐이딩된 의사-3D), 보간/외삽, 우측 사이드바, 서버 쪽 IK/자세 실배선(그동안 `ik.rs`/`posture.rs`가 자기 테스트에서만 쓰이던 것을 처음으로 실사용), 3계층 테스트(vitest 단위/통합, Playwright E2E), 실측한 클라이언트 프레임 시간(Step 3 결과) 기록.
- **다음 단계**에서 "Plan 4" 항목을 제거하고 Plan 5(데모/배포)만 남긴다.
- 테스트 개수 갱신(서버 123 → 새 개수, 클라이언트 단위/통합/E2E 각각 개수 명시).

- [ ] **Step 6: KANBAN.md 갱신**

`docs/KANBAN.md`의 Backlog에 있던 "Plan 4" 항목을 Done으로 옮기고, 이 계획 문서(`docs/superpowers/plans/2026-07-18-client-rendering-plan.md`) 경로와 14개 태스크 전체 완료를 요약한다(기존 로봇 내구도 기능 Done 항목과 같은 서술 밀도로 — 뮤테이션 테스트로 잡아낸 것이 있다면 그것도 기록). "현재 건강도 스냅샷"의 `vitest: 해당 없음(client/ 없음)` 줄도 실제 테스트 개수로 갱신한다.

- [ ] **Step 7: 최종 커밋**

```bash
git add README.md docs/robot-arm-conveyor-game-design.md docs/KANBAN.md
git commit -m "docs: Plan 4(클라이언트 렌더링) 완료 반영 — README/설계문서/KANBAN 갱신"
```

---

## 참고 — 각 태스크 완료 시 KANBAN.md도 함께 갱신

프로젝트 관행(`CLAUDE.md`, 과거 Plan 1-3·로봇 내구도 기능의 커밋 이력)상 태스크 하나가 끝날 때마다 `docs/KANBAN.md`의 In Progress 항목에 커밋 SHA를 남기는 `docs:` 커밋이 뒤따른다. 이 계획의 Task 1-13 각각도 완료 직후 그렇게 갱신하고, Task 14에서 전체를 한 번에 정리한다.

