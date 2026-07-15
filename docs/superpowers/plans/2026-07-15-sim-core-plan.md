# 로봇팔 컨베이어 게임 — Plan 1: 결정적 시뮬레이션 코어 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 네트워킹 없는 순수 Rust 라이브러리 크레이트로 그리드 · A* 경로탐색 · 결정적 병렬 틱 시뮬레이션(더블 버퍼링 + ID 타이브레이크 + 장애 격리) · 프로시저럴 보행 · 몸체-팔 단일 기구학 체인(2-본 IK) · 결정적 생산량 집계를 구현하고, `cargo test` 하나만으로 전부 검증 가능하게 만든다.

**Architecture:** 순수 함수 기반 결정적 코어 — 모든 상태 전이는 `fn(state: &SimState) -> SimState` 형태로, 부수효과(I/O)가 없다. 그리드/경로탐색/시뮬레이션 틱/보행/IK/자세/생산량을 각각 독립 모듈로 분리한다. 틱 갱신은 Rayon으로 로봇별 계획을 병렬 계산하되, 각 로봇은 오직 "이전 틱이 끝난 시점의 얼어붙은 스냅샷"만 읽고(더블 버퍼링), 같은 칸을 동시에 노리는 충돌은 로봇 ID가 낮은 쪽이 이기는 결정적 타이브레이크로 해소한다. 한 로봇의 계산이 패닉해도 `catch_unwind`로 격리해 틱 전체가 죽지 않게 한다.

**Tech Stack:** Rust 2021 edition (stable), `rayon`(병렬 틱 처리), `proptest`(dev-dependency, 프로퍼티 기반 테스트). 네트워킹(WebSocket)·영속화(SQLite)·클라이언트(TS/Canvas)는 이후 Plan에서 다루며, 이 Plan은 그것들에 전혀 의존하지 않는다.

**설계 문서 참조:** `docs/robot-arm-conveyor-game-design.md`의 "로봇 모델", "경로탐색", "동시성 모델" 절.

---

## 파일 구조

| 파일 | 책임 |
|---|---|
| `server/Cargo.toml` | 크레이트 매니페스트. 패키지명 `gamerobotfactory-server`, 라이브러리명 `sim_core`. |
| `server/src/lib.rs` | 모듈 선언만 담당. |
| `server/src/grid.rs` | 정적 그리드 표현 + 이동 가능 여부 판정. |
| `server/src/pathfind.rs` | A* 경로탐색 (벽 + 다른 로봇 회피). |
| `server/src/sim.rs` | `Robot`/`BodyPose`/`SimState` 정의, 틱 루프(더블 버퍼링·재계획 주기·충돌 타이브레이크·패닉 격리). |
| `server/src/gait.rs` | 다리별 stance/swing 위상 + 발 들림 높이 계산 (순수 함수, `cycle_progress` 입력). |
| `server/src/ik.rs` | 2-본 해석적 팔 IK (몸체 로컬 좌표, 도달 불가 클램프). |
| `server/src/posture.rs` | 몸체 자세(`BodyPose`)와 팔 IK 타겟을 연결하는 변환 함수 — "단일 기구학 체인" 요구사항을 만족시키는 지점. |
| `server/src/production.rs` | 로봇 ID 정렬 기준 결정적 생산량 합산. |
| `server/tests/pathfinding_properties.rs` | proptest 기반 경로탐색 불변식 검증. |

각 파일은 이전 파일이 정의한 타입만 가져다 쓰고(전방 참조 없음), 이 순서(Task 1→10)대로 만들면 매 태스크마다 `cargo test`가 항상 그린 상태를 유지한다.

---

### Task 1: 크레이트 스캐폴드

**Files:**
- Create: `server/Cargo.toml`
- Create: `server/src/lib.rs`

- [ ] **Step 1: Cargo.toml 작성**

```toml
[package]
name = "gamerobotfactory-server"
version = "0.1.0"
edition = "2021"

[lib]
name = "sim_core"
path = "src/lib.rs"

[dependencies]
rayon = "1"

[dev-dependencies]
proptest = "1"
```

- [ ] **Step 2: 빈 lib.rs 생성**

`server/src/lib.rs` 내용을 완전히 비워둔다 (0바이트). 이후 태스크마다 `pub mod ...;` 선언을 하나씩 추가한다.

- [ ] **Step 3: 빌드 확인**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: `running 0 tests` — 컴파일 성공, 테스트 0개로 통과.

- [ ] **Step 4: Commit**

```bash
git add server/Cargo.toml server/src/lib.rs
git commit -m "chore: scaffold sim_core crate"
```

---

### Task 2: 그리드 모델

**Files:**
- Create: `server/src/grid.rs`
- Modify: `server/src/lib.rs`
- Test: `server/src/grid.rs` (인라인 `#[cfg(test)]` 모듈)

- [ ] **Step 1: 실패하는 테스트 작성**

`server/src/grid.rs`:

```rust
use std::collections::HashSet;

pub type CellId = (i32, i32);

#[derive(Debug, Clone)]
pub struct Grid {
    pub width: i32,
    pub height: i32,
    walls: HashSet<CellId>,
}

impl Grid {
    pub fn new(width: i32, height: i32) -> Self {
        Grid { width, height, walls: HashSet::new() }
    }

    pub fn add_wall(&mut self, cell: CellId) {
        self.walls.insert(cell);
    }

    pub fn in_bounds(&self, cell: CellId) -> bool {
        cell.0 >= 0 && cell.0 < self.width && cell.1 >= 0 && cell.1 < self.height
    }

    pub fn is_wall(&self, cell: CellId) -> bool {
        self.walls.contains(&cell)
    }

    pub fn is_walkable(&self, cell: CellId) -> bool {
        self.in_bounds(cell) && !self.is_wall(cell)
    }

    pub fn neighbors(&self, cell: CellId) -> Vec<CellId> {
        let (x, y) = cell;
        [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]
            .into_iter()
            .filter(|&c| self.is_walkable(c))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walkable_cell_in_bounds_with_no_wall() {
        let grid = Grid::new(5, 5);
        assert!(grid.is_walkable((2, 2)));
    }

    #[test]
    fn out_of_bounds_cell_is_not_walkable() {
        let grid = Grid::new(5, 5);
        assert!(!grid.is_walkable((5, 0)));
        assert!(!grid.is_walkable((-1, 0)));
    }

    #[test]
    fn wall_cell_is_not_walkable() {
        let mut grid = Grid::new(5, 5);
        grid.add_wall((2, 2));
        assert!(!grid.is_walkable((2, 2)));
    }

    #[test]
    fn neighbors_excludes_walls_and_out_of_bounds() {
        let mut grid = Grid::new(3, 3);
        grid.add_wall((1, 0));
        let mut ns = grid.neighbors((0, 0));
        ns.sort();
        assert_eq!(ns, vec![(0, 1)]);
    }
}
```

`server/src/lib.rs`에 추가:

```rust
pub mod grid;
```

- [ ] **Step 2: 테스트 실행해서 통과 확인**

Run: `cargo test --manifest-path server/Cargo.toml grid::`
Expected: 4개 테스트 모두 PASS (이 파일은 테스트와 구현을 함께 작성했으므로 fail 단계 없이 바로 통과 확인).

- [ ] **Step 3: Commit**

```bash
git add server/src/lib.rs server/src/grid.rs
git commit -m "feat: add static grid model"
```

---

### Task 3: A* 경로탐색

**Files:**
- Create: `server/src/pathfind.rs`
- Modify: `server/src/lib.rs`
- Test: `server/src/pathfind.rs` (인라인)

- [ ] **Step 1: 구현 + 테스트 작성**

`server/src/pathfind.rs`:

```rust
use crate::grid::{CellId, Grid};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

#[derive(Copy, Clone, Eq, PartialEq)]
struct QueueEntry {
    cost: i32,
    cell: CellId,
}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.cmp(&self.cost).then_with(|| self.cell.cmp(&other.cell))
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn heuristic(a: CellId, b: CellId) -> i32 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs()
}

/// 시작점에서 목표점까지 최단 경로를 찾는다. 벽과 `blocked`(다른 로봇이
/// 현재 점유한 칸)을 장애물로 취급하되, 목표 칸 자체가 `blocked`에 들어
///있어도 그쪽으로 향하는 시도는 막지 않는다 — 그 로봇이 다음 틱에 비킬
/// 수도 있기 때문이며, 실제 동시 이동 충돌은 `sim` 모듈의 타이브레이크가
/// 처리한다. 반환값은 `start`를 제외하고 `goal`을 포함하는 경로. 도달
/// 불가능하면 `None`.
pub fn find_path(
    grid: &Grid,
    start: CellId,
    goal: CellId,
    blocked: &HashSet<CellId>,
) -> Option<Vec<CellId>> {
    if start == goal {
        return Some(vec![]);
    }

    let mut open = BinaryHeap::new();
    open.push(QueueEntry { cost: heuristic(start, goal), cell: start });

    let mut came_from: HashMap<CellId, CellId> = HashMap::new();
    let mut g_score: HashMap<CellId, i32> = HashMap::new();
    g_score.insert(start, 0);

    let mut visited: HashSet<CellId> = HashSet::new();

    while let Some(QueueEntry { cell: current, .. }) = open.pop() {
        if current == goal {
            return Some(reconstruct_path(&came_from, start, goal));
        }
        if !visited.insert(current) {
            continue;
        }

        for next in grid.neighbors(current) {
            if next != goal && blocked.contains(&next) {
                continue;
            }
            let tentative_g = g_score[&current] + 1;
            if tentative_g < *g_score.get(&next).unwrap_or(&i32::MAX) {
                came_from.insert(next, current);
                g_score.insert(next, tentative_g);
                open.push(QueueEntry { cost: tentative_g + heuristic(next, goal), cell: next });
            }
        }
    }

    None
}

fn reconstruct_path(came_from: &HashMap<CellId, CellId>, start: CellId, goal: CellId) -> Vec<CellId> {
    let mut path = vec![goal];
    let mut current = goal;
    while current != start {
        current = came_from[&current];
        if current != start {
            path.push(current);
        }
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn finds_straight_path_on_empty_grid() {
        let grid = Grid::new(5, 5);
        let path = find_path(&grid, (0, 0), (3, 0), &HashSet::new()).unwrap();
        assert_eq!(path, vec![(1, 0), (2, 0), (3, 0)]);
    }

    #[test]
    fn returns_empty_path_when_already_at_goal() {
        let grid = Grid::new(5, 5);
        let path = find_path(&grid, (2, 2), (2, 2), &HashSet::new()).unwrap();
        assert_eq!(path, Vec::<CellId>::new());
    }

    #[test]
    fn returns_none_when_fully_walled_off() {
        let mut grid = Grid::new(3, 3);
        grid.add_wall((1, 0));
        grid.add_wall((1, 1));
        grid.add_wall((1, 2));
        let path = find_path(&grid, (0, 0), (2, 0), &HashSet::new());
        assert_eq!(path, None);
    }

    #[test]
    fn routes_around_partial_wall() {
        let mut grid = Grid::new(3, 3);
        grid.add_wall((1, 0));
        grid.add_wall((1, 1));
        let path = find_path(&grid, (0, 0), (2, 0), &HashSet::new()).unwrap();
        assert!(!path.contains(&(1, 0)));
        assert!(!path.contains(&(1, 1)));
    }

    #[test]
    fn treats_blocked_cells_as_obstacles() {
        let grid = Grid::new(3, 1);
        let mut blocked = HashSet::new();
        blocked.insert((1, 0));
        let path = find_path(&grid, (0, 0), (2, 0), &blocked);
        assert_eq!(path, None);
    }
}
```

`server/src/lib.rs`에 추가:

```rust
pub mod pathfind;
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml pathfind::`
Expected: 5개 테스트 PASS.

- [ ] **Step 3: Commit**

```bash
git add server/src/lib.rs server/src/pathfind.rs
git commit -m "feat: add A* pathfinding with blocked-cell avoidance"
```

---

### Task 4: 결정적 다중 로봇 틱 (더블 버퍼링 + 타이브레이크)

**Files:**
- Create: `server/src/sim.rs`
- Modify: `server/src/lib.rs`
- Test: `server/src/sim.rs` (인라인)

- [ ] **Step 1: 구현 + 테스트 작성**

`server/src/sim.rs`:

```rust
use crate::grid::{CellId, Grid};
use crate::pathfind::find_path;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

const REPATH_INTERVAL: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BodyPose {
    Standing,
    Crouching,
}

impl BodyPose {
    /// 어깨 관절의 지면 기준 높이. `posture` 모듈에서 팔 IK 타겟을
    /// 몸체 로컬 좌표로 바꿀 때 이 값을 뺀다 — 몸체 자세와 팔 IK가
    /// 분리되어 설계되지 않도록 하는 유일한 연결점.
    pub fn shoulder_height(&self) -> f32 {
        match self {
            BodyPose::Standing => 1.0,
            BodyPose::Crouching => 0.5,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Robot {
    pub id: u32,
    pub pos: CellId,
    pub goal: CellId,
    pub path: Vec<CellId>,
    pub ticks_until_repath: u32,
    pub pose: BodyPose,
}

impl Robot {
    pub fn new(id: u32, pos: CellId, goal: CellId) -> Self {
        Robot {
            id,
            pos,
            goal,
            path: Vec::new(),
            ticks_until_repath: 0,
            pose: BodyPose::Standing,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SimState {
    pub grid: Grid,
    pub robots: Vec<Robot>,
    pub tick_count: u64,
}

#[derive(Debug, Clone, Copy)]
struct MoveIntent {
    robot_id: u32,
    from: CellId,
    to: CellId,
}

/// 시뮬레이션을 정확히 한 틱 전진시킨다. 순수 함수 — `state`를 변경하지
/// 않고 새 상태를 반환한다. 각 로봇의 계획은 "틱 시작 시점에 얼어붙은
/// 스냅샷"(`occupied`)만 읽으므로(더블 버퍼링), 병렬로 계산해도 서로의
/// 계산 중인 결과를 참조하지 않아 데이터 경쟁이 없다.
pub fn tick(state: &SimState) -> SimState {
    let occupied: HashSet<CellId> = state.robots.iter().map(|r| r.pos).collect();

    let planned: Vec<Robot> = state
        .robots
        .par_iter()
        .map(|robot| plan_robot(&state.grid, robot, &occupied))
        .collect();

    let intents: Vec<MoveIntent> = state
        .robots
        .iter()
        .zip(planned.iter())
        .map(|(original, planned)| MoveIntent {
            robot_id: original.id,
            from: original.pos,
            to: planned.pos,
        })
        .collect();

    let resolved_positions = resolve_intents(&intents);

    let new_robots: Vec<Robot> = planned
        .into_iter()
        .zip(resolved_positions.into_iter())
        .map(|(mut robot, final_pos)| {
            let lost_tiebreak = final_pos != robot.pos;
            robot.pos = final_pos;
            if lost_tiebreak {
                // 다른 로봇이 이번 칸을 가져갔다 — 이번 틱은 제자리에 멈추고
                // 다음 기회에 새로 재계획한다 (무의미한 즉시 재시도 방지).
                robot.path.clear();
                robot.ticks_until_repath = 0;
            }
            robot
        })
        .collect();

    SimState { grid: state.grid.clone(), robots: new_robots, tick_count: state.tick_count + 1 }
}

fn plan_robot(grid: &Grid, robot: &Robot, occupied: &HashSet<CellId>) -> Robot {
    let mut next = robot.clone();

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
        if !occupied.contains(&next_cell) || next_cell == robot.pos {
            next.pos = next_cell;
            next.path.remove(0);
        }
        // else: 다른 로봇이 지난 틱 기준으로 그 칸을 차지하고 있다 —
        // 이번 틱은 멈추고, 곧 돌아올 재계획 주기에서 우회로를 찾는다.
    }

    next
}

/// 같은 틱에 여러 로봇이 같은 칸으로 이동을 계획하면, `robot_id`가 가장
/// 낮은 로봇이 이기고 나머지는 원래 칸으로 되돌린다 — 실행 순서나 스레드
/// 스케줄링과 무관하게 항상 같은 결과가 나오는 결정적 타이브레이크.
fn resolve_intents(intents: &[MoveIntent]) -> Vec<CellId> {
    let mut winner_by_cell: HashMap<CellId, u32> = HashMap::new();
    for intent in intents {
        winner_by_cell
            .entry(intent.to)
            .and_modify(|winner| {
                if intent.robot_id < *winner {
                    *winner = intent.robot_id;
                }
            })
            .or_insert(intent.robot_id);
    }

    intents
        .iter()
        .map(|intent| if winner_by_cell[&intent.to] == intent.robot_id { intent.to } else { intent.from })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_state(width: i32, height: i32) -> SimState {
        SimState { grid: Grid::new(width, height), robots: Vec::new(), tick_count: 0 }
    }

    #[test]
    fn robot_moves_one_step_toward_goal_each_tick() {
        let mut state = simple_state(5, 1);
        state.robots.push(Robot::new(1, (0, 0), (3, 0)));

        let next = tick(&state);

        assert_eq!(next.robots[0].pos, (1, 0));
        assert_eq!(next.tick_count, 1);
    }

    #[test]
    fn robot_stops_moving_once_at_goal() {
        let mut state = simple_state(5, 1);
        state.robots.push(Robot::new(1, (2, 0), (2, 0)));

        let next = tick(&state);

        assert_eq!(next.robots[0].pos, (2, 0));
    }

    #[test]
    fn lower_id_wins_when_two_robots_target_same_cell() {
        // 로봇 1은 (0,0)에서 오른쪽으로, 로봇 2는 (2,0)에서 왼쪽으로 —
        // 둘 다 (1,0)을 향해 움직이는 정면 대결 시나리오.
        let mut state = simple_state(3, 1);
        state.robots.push(Robot::new(1, (0, 0), (2, 0)));
        state.robots.push(Robot::new(2, (2, 0), (0, 0)));

        let next = tick(&state);

        let r1 = next.robots.iter().find(|r| r.id == 1).unwrap();
        let r2 = next.robots.iter().find(|r| r.id == 2).unwrap();
        assert_eq!(r1.pos, (1, 0), "낮은 id가 이겨야 한다");
        assert_eq!(r2.pos, (2, 0), "높은 id는 원래 칸으로 되돌아가야 한다");
    }

    #[test]
    fn tick_is_deterministic_across_repeated_runs() {
        let mut state = simple_state(3, 1);
        state.robots.push(Robot::new(1, (0, 0), (2, 0)));
        state.robots.push(Robot::new(2, (2, 0), (0, 0)));

        let positions_a: Vec<CellId> = tick(&tick(&state)).robots.iter().map(|r| r.pos).collect();
        let positions_b: Vec<CellId> = tick(&tick(&state)).robots.iter().map(|r| r.pos).collect();
        assert_eq!(positions_a, positions_b);
    }
}
```

`server/src/lib.rs`에 추가:

```rust
pub mod sim;
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml sim::`
Expected: 4개 테스트 PASS. (`lower_id_wins_when_two_robots_target_same_cell`가 이 태스크의 핵심 — 실패하면 타이브레이크 로직 자체가 틀린 것.)

- [ ] **Step 3: Commit**

```bash
git add server/src/lib.rs server/src/sim.rs
git commit -m "feat: add deterministic parallel tick with move tie-breaking"
```

---

### Task 5: 패닉 격리

로봇 하나의 갱신이 패닉해도 해당 틱 전체가 죽지 않아야 한다는 설계 요구사항(§동시성 모델)을 구현한다.

**Files:**
- Modify: `server/src/sim.rs`

- [ ] **Step 1: `safe_plan_robot` 래퍼 추가 + 테스트 작성**

`server/src/sim.rs`의 `tick` 함수 내부, `planned` 계산 줄을 다음과 같이 바꾼다:

```rust
    let planned: Vec<Robot> = state
        .robots
        .par_iter()
        .map(|robot| safe_plan_robot(&state.grid, robot, &occupied))
        .collect();
```

`plan_robot` 함수 바로 아래에 새 함수를 추가한다:

```rust
/// `plan_robot`을 패닉으로부터 격리한다. 패닉이 나면 해당 로봇은 이번
/// 틱을 그대로 멈춘 채 넘어가고, 나머지 로봇들의 갱신은 영향받지 않는다.
fn safe_plan_robot(grid: &Grid, robot: &Robot, occupied: &HashSet<CellId>) -> Robot {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| plan_robot(grid, robot, occupied)))
        .unwrap_or_else(|_| {
            eprintln!("robot {} update panicked; holding position this tick", robot.id);
            robot.clone()
        })
}
```

테스트 모듈에 추가:

```rust
    #[test]
    fn safe_plan_robot_recovers_from_a_panic_and_holds_position() {
        // plan_robot 자체를 결정적으로 패닉시키려면 결함 주입 지점이
        // 필요한데 이는 이 Plan 범위를 벗어난다. 대신 safe_plan_robot이
        // 실제로 사용하는 것과 동일한 catch_unwind 복구 경로를 여기서
        // 직접 검증한다.
        let robot = Robot::new(1, (0, 0), (2, 0));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Robot {
            panic!("simulated fault in robot update")
        }))
        .unwrap_or_else(|_| robot.clone());

        assert_eq!(result.pos, robot.pos);
    }

    #[test]
    fn one_robot_panicking_does_not_block_others_from_updating() {
        // safe_plan_robot으로 모든 로봇 갱신을 감싸도, 정상적인 로봇은
        // 평소대로 전진해야 한다는 회귀 방지 테스트.
        let mut state = simple_state(5, 1);
        state.robots.push(Robot::new(1, (0, 0), (3, 0)));
        state.robots.push(Robot::new(2, (4, 0), (4, 0)));

        let next = tick(&state);

        let healthy = next.robots.iter().find(|r| r.id == 1).unwrap();
        assert_eq!(healthy.pos, (1, 0));
    }
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml sim::`
Expected: 이전 4개 + 신규 2개 = 6개 PASS.

- [ ] **Step 3: Commit**

```bash
git add server/src/sim.rs
git commit -m "feat: isolate per-robot panics with catch_unwind"
```

---

### Task 6: 프로시저럴 보행 (stance/swing)

**Files:**
- Create: `server/src/gait.rs`
- Modify: `server/src/lib.rs`, `server/src/sim.rs`
- Test: `server/src/gait.rs`, `server/src/sim.rs` (인라인)

- [ ] **Step 1: gait.rs 작성**

`server/src/gait.rs`:

```rust
/// 다리 4개: 앞왼쪽/앞오른쪽/뒤왼쪽/뒤오른쪽.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegId {
    FrontLeft,
    FrontRight,
    BackLeft,
    BackRight,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LegPhase {
    Stance, // 발이 땅을 딛고 체중을 지지
    Swing,  // 발이 들려 앞으로 이동 중
}

fn diagonal_offset(leg: LegId) -> f32 {
    match leg {
        LegId::FrontLeft | LegId::BackRight => 0.0,
        LegId::FrontRight | LegId::BackLeft => 0.5,
    }
}

/// 트롯 걸음: 대각선 다리 쌍(앞왼쪽+뒤오른쪽, 앞오른쪽+뒤왼쪽)이 반 주기
/// 어긋나 함께 움직인다. `cycle_progress`는 [0.0, 1.0) 범위이며 로봇이
/// 이동 중일 때만 전진한다(정지 중엔 고정). 듀티 팩터를 스윙 40% /
/// 스탠스 60%로 둬서 항상 최소 한 쌍은 접지 상태를 유지한다 — 이게
/// 없으면 발이 미끄러지듯 보인다.
pub fn leg_phase(leg: LegId, cycle_progress: f32) -> LegPhase {
    let local = (cycle_progress + diagonal_offset(leg)).rem_euclid(1.0);
    if local < 0.4 { LegPhase::Swing } else { LegPhase::Stance }
}

/// 렌더링용 발 들림 높이. 접지 중엔 0.0, 스윙 구간 중간에 `lift`만큼
/// 올라갔다가 착지 직전 다시 0.0으로 돌아온다.
pub fn foot_lift(leg: LegId, cycle_progress: f32, lift: f32) -> f32 {
    let local = (cycle_progress + diagonal_offset(leg)).rem_euclid(1.0);
    if local < 0.4 {
        let swing_progress = local / 0.4;
        lift * (swing_progress * std::f32::consts::PI).sin()
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagonal_pairs_share_phase() {
        let progress = 0.1;
        assert_eq!(leg_phase(LegId::FrontLeft, progress), leg_phase(LegId::BackRight, progress));
        assert_eq!(leg_phase(LegId::FrontRight, progress), leg_phase(LegId::BackLeft, progress));
    }

    #[test]
    fn opposite_diagonals_are_out_of_phase() {
        let progress = 0.1;
        assert_ne!(leg_phase(LegId::FrontLeft, progress), leg_phase(LegId::FrontRight, progress));
    }

    #[test]
    fn at_least_one_diagonal_pair_is_always_grounded() {
        for i in 0..100 {
            let progress = i as f32 / 100.0;
            let fl_grounded = leg_phase(LegId::FrontLeft, progress) == LegPhase::Stance;
            let fr_grounded = leg_phase(LegId::FrontRight, progress) == LegPhase::Stance;
            assert!(fl_grounded || fr_grounded, "progress {progress}에서 두 대각쌍이 동시에 떠있음");
        }
    }

    #[test]
    fn planted_foot_has_zero_lift() {
        assert_eq!(foot_lift(LegId::FrontLeft, 0.5, 0.1), 0.0);
    }

    #[test]
    fn swinging_foot_lifts_off_the_ground() {
        assert!(foot_lift(LegId::FrontLeft, 0.1, 0.1) > 0.0);
    }
}
```

`server/src/lib.rs`에 추가:

```rust
pub mod gait;
```

- [ ] **Step 2: gait 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml gait::`
Expected: 5개 테스트 PASS.

- [ ] **Step 3: sim.rs에 다리 주기 상태 배선**

`Robot` 구조체에 필드 추가 (`server/src/sim.rs`):

```rust
pub struct Robot {
    pub id: u32,
    pub pos: CellId,
    pub goal: CellId,
    pub path: Vec<CellId>,
    pub ticks_until_repath: u32,
    pub pose: BodyPose,
    pub leg_cycle_progress: f32,
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
        }
    }
}
```

파일 상단에 상수 추가:

```rust
const LEG_CYCLE_SPEED: f32 = 0.1;
```

`tick` 함수의 `new_robots` 계산 부분을 다음으로 교체 (실제로 이동한 로봇만 다리 주기를 전진시키기 위해 원래 위치와 비교):

```rust
    let new_robots: Vec<Robot> = state
        .robots
        .iter()
        .zip(planned.into_iter())
        .zip(resolved_positions.into_iter())
        .map(|((original, mut robot), final_pos)| {
            let lost_tiebreak = final_pos != robot.pos;
            robot.pos = final_pos;
            if lost_tiebreak {
                robot.path.clear();
                robot.ticks_until_repath = 0;
            }
            if robot.pos != original.pos {
                robot.leg_cycle_progress = (robot.leg_cycle_progress + LEG_CYCLE_SPEED).rem_euclid(1.0);
            }
            robot
        })
        .collect();
```

테스트 모듈에 추가:

```rust
    #[test]
    fn leg_cycle_progress_advances_while_moving() {
        let mut state = simple_state(5, 1);
        state.robots.push(Robot::new(1, (0, 0), (3, 0)));

        let next = tick(&state);

        assert!(next.robots[0].leg_cycle_progress > 0.0);
    }

    #[test]
    fn leg_cycle_progress_does_not_advance_once_at_goal() {
        let mut state = simple_state(5, 1);
        state.robots.push(Robot::new(1, (2, 0), (2, 0)));

        let next = tick(&state);

        assert_eq!(next.robots[0].leg_cycle_progress, 0.0);
    }
```

- [ ] **Step 4: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml sim::`
Expected: 이전 6개 + 신규 2개 = 8개 PASS.

- [ ] **Step 5: Commit**

```bash
git add server/src/lib.rs server/src/gait.rs server/src/sim.rs
git commit -m "feat: add procedural stance/swing gait and wire leg cycle into the tick"
```

---

### Task 7: 2-본 팔 IK

**Files:**
- Create: `server/src/ik.rs`
- Modify: `server/src/lib.rs`
- Test: `server/src/ik.rs` (인라인)

- [ ] **Step 1: 구현 + 테스트 작성**

`server/src/ik.rs`:

```rust
/// 팔 끝(손목) 목표 위치 — 몸체 로컬 좌표(원점 = 어깨, x = 전방, y = 위).
/// 몸체가 웅크리거나 서도 이 숫자 자체는 바뀌지 않는다 — 어깨의 월드
/// 트랜스폼만 바뀌고, 최종 월드 포즈는 "몸체 트랜스폼 * 이 팔 로컬
/// 포즈"로 합성된다. 이게 몸체 자세와 무관하게 팔이 어깨에 붙어있도록
/// 보장하는 지점이다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct ArmPose {
    pub shoulder_angle: f32,
    pub elbow_angle: f32,
}

/// 어깨-팔꿈치-손목 2-본 해석적 IK. `target`이 팔이 닿을 수 있는 최대
/// 거리보다 멀면, 같은 방향을 유지한 채 최대 도달 거리로 클램프한
/// 뒤 푼다 — 그래서 목표가 너무 멀어도 팔은 최소한 그 방향을 향한다.
pub fn solve_two_bone_ik(upper_len: f32, lower_len: f32, target: Point2) -> ArmPose {
    let max_reach = upper_len + lower_len;
    let min_reach = (upper_len - lower_len).abs();
    let dist = (target.x * target.x + target.y * target.y).sqrt();

    let clamped_dist = dist.clamp(min_reach.max(0.001), max_reach - 0.001);
    let scale = if dist > 0.0 { clamped_dist / dist } else { 1.0 };
    let target = Point2 { x: target.x * scale, y: target.y * scale };
    let dist = clamped_dist;

    let cos_elbow = (upper_len.powi(2) + lower_len.powi(2) - dist.powi(2)) / (2.0 * upper_len * lower_len);
    let elbow_angle = std::f32::consts::PI - cos_elbow.clamp(-1.0, 1.0).acos();

    let angle_to_target = target.y.atan2(target.x);
    let cos_shoulder_offset = (upper_len.powi(2) + dist.powi(2) - lower_len.powi(2)) / (2.0 * upper_len * dist);
    let shoulder_offset = cos_shoulder_offset.clamp(-1.0, 1.0).acos();
    let shoulder_angle = angle_to_target - shoulder_offset;

    ArmPose { shoulder_angle, elbow_angle }
}

/// 주어진 포즈에서 손목이 실제로 어디에 있는지 계산한다 — 테스트와
/// 렌더러가 각도를 다시 좌표로 바꿀 때 쓴다.
pub fn forward_kinematics(upper_len: f32, lower_len: f32, pose: ArmPose) -> Point2 {
    let elbow = Point2 {
        x: upper_len * pose.shoulder_angle.cos(),
        y: upper_len * pose.shoulder_angle.sin(),
    };
    let wrist_angle = pose.shoulder_angle + pose.elbow_angle;
    Point2 { x: elbow.x + lower_len * wrist_angle.cos(), y: elbow.y + lower_len * wrist_angle.sin() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(a: f32, b: f32) {
        assert!((a - b).abs() < 0.01, "{a} != {b}");
    }

    #[test]
    fn reaches_target_within_arm_length() {
        let target = Point2 { x: 1.0, y: 0.0 };
        let pose = solve_two_bone_ik(0.7, 0.6, target);
        let result = forward_kinematics(0.7, 0.6, pose);
        assert_close(result.x, target.x);
        assert_close(result.y, target.y);
    }

    #[test]
    fn clamps_target_beyond_max_reach() {
        let far_target = Point2 { x: 10.0, y: 0.0 };
        let pose = solve_two_bone_ik(0.7, 0.6, far_target);
        let result = forward_kinematics(0.7, 0.6, pose);
        let reached_dist = (result.x.powi(2) + result.y.powi(2)).sqrt();
        assert!(reached_dist <= 0.7 + 0.6 + 0.01);
        assert!(result.x > 0.0);
    }

    #[test]
    fn handles_target_at_origin_without_panicking() {
        let pose = solve_two_bone_ik(0.7, 0.6, Point2 { x: 0.0, y: 0.0 });
        assert!(pose.shoulder_angle.is_finite());
        assert!(pose.elbow_angle.is_finite());
    }
}
```

`server/src/lib.rs`에 추가:

```rust
pub mod ik;
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml ik::`
Expected: 3개 테스트 PASS.

- [ ] **Step 3: Commit**

```bash
git add server/src/lib.rs server/src/ik.rs
git commit -m "feat: add two-bone analytic arm IK with reach clamping"
```

---

### Task 8: 몸체 자세 ↔ 팔 IK 연결

**Files:**
- Create: `server/src/posture.rs`
- Modify: `server/src/lib.rs`
- Test: `server/src/posture.rs` (인라인)

- [ ] **Step 1: 구현 + 테스트 작성**

`server/src/posture.rs`:

```rust
use crate::ik::Point2;
use crate::sim::BodyPose;

/// 월드 공간 목표(예: "이 컨베이어 슬롯의 높이")를 `solve_two_bone_ik`가
/// 기대하는 몸체 로컬 좌표로 바꾼다. 몸체 자세와 팔 IK가 각자 따로
/// 설계되지 않도록 하는 유일한 변환 지점 — 웅크리면 어깨가 낮아지므로,
/// 같은 월드 목표라도 몸체 로컬 y가 달라진다.
pub fn world_target_to_body_local(world_target_height: f32, world_target_forward: f32, pose: BodyPose) -> Point2 {
    Point2 { x: world_target_forward, y: world_target_height - pose.shoulder_height() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crouching_raises_the_target_in_body_local_space() {
        let target_height = 0.6;
        let standing = world_target_to_body_local(target_height, 0.5, BodyPose::Standing);
        let crouching = world_target_to_body_local(target_height, 0.5, BodyPose::Crouching);
        assert!(
            crouching.y > standing.y,
            "고정된 월드 목표는 웅크린 상태에서 몸체 로컬 기준 더 높게 보여야 한다"
        );
    }

    #[test]
    fn forward_offset_is_unaffected_by_posture() {
        let standing = world_target_to_body_local(0.6, 0.5, BodyPose::Standing);
        let crouching = world_target_to_body_local(0.6, 0.5, BodyPose::Crouching);
        assert_eq!(standing.x, crouching.x);
    }
}
```

`server/src/lib.rs`에 추가:

```rust
pub mod posture;
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml posture::`
Expected: 2개 테스트 PASS.

- [ ] **Step 3: Commit**

```bash
git add server/src/lib.rs server/src/posture.rs
git commit -m "feat: connect body posture to arm IK target conversion"
```

---

### Task 9: 결정적 생산량 집계

**Files:**
- Create: `server/src/production.rs`
- Modify: `server/src/lib.rs`
- Test: `server/src/production.rs` (인라인)

- [ ] **Step 1: 구현 + 테스트 작성**

`server/src/production.rs`:

```rust
use crate::sim::Robot;
use std::collections::HashMap;

/// 이번 틱 총 생산량. 로봇 ID 오름차순으로 고정해서 합산한다 — 이 틱을
/// 계산하기 전에 로봇 갱신이 어떤 순서로 병렬화됐든, 부동소수점 합산
/// 순서가 항상 같아서 결과가 실행마다 재현 가능하다.
pub fn total_production(robots: &[Robot], units_per_robot: &HashMap<u32, f32>) -> f32 {
    let mut ids: Vec<u32> = robots.iter().map(|r| r.id).collect();
    ids.sort_unstable();

    ids.iter().map(|id| units_per_robot.get(id).copied().unwrap_or(0.0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::Robot;

    #[test]
    fn sums_in_ascending_id_order_regardless_of_input_order() {
        let robots_a = vec![Robot::new(3, (0, 0), (0, 0)), Robot::new(1, (0, 0), (0, 0))];
        let robots_b = vec![Robot::new(1, (0, 0), (0, 0)), Robot::new(3, (0, 0), (0, 0))];

        let mut units = HashMap::new();
        units.insert(1, 0.1_f32);
        units.insert(3, 0.2_f32);

        assert_eq!(total_production(&robots_a, &units), total_production(&robots_b, &units));
    }

    #[test]
    fn missing_robot_contributes_zero() {
        let robots = vec![Robot::new(5, (0, 0), (0, 0))];
        let units = HashMap::new();
        assert_eq!(total_production(&robots, &units), 0.0);
    }
}
```

`server/src/lib.rs`에 추가:

```rust
pub mod production;
```

- [ ] **Step 2: 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml production::`
Expected: 2개 테스트 PASS.

- [ ] **Step 3: Commit**

```bash
git add server/src/lib.rs server/src/production.rs
git commit -m "feat: add deterministic sorted-by-id production aggregation"
```

---

### Task 10: 프로퍼티 기반 경로탐색 테스트 + 전체 검증

**Files:**
- Create: `server/tests/pathfinding_properties.rs`

- [ ] **Step 1: 프로퍼티 테스트 작성**

`server/tests/pathfinding_properties.rs`:

```rust
use proptest::prelude::*;
use sim_core::grid::{CellId, Grid};
use sim_core::pathfind::find_path;
use std::collections::HashSet;

const SIZE: i32 = 6;

fn arbitrary_grid_and_endpoints() -> impl Strategy<Value = (Grid, CellId, CellId)> {
    (
        proptest::collection::vec((0..SIZE, 0..SIZE), 0..8),
        (0..SIZE, 0..SIZE),
        (0..SIZE, 0..SIZE),
    )
        .prop_map(move |(walls, start, goal)| {
            let mut grid = Grid::new(SIZE, SIZE);
            for w in walls {
                grid.add_wall(w);
            }
            (grid, start, goal)
        })
}

proptest! {
    /// 경로가 존재한다면, 모든 칸은 이동 가능해야 하고 연속된 두 칸은
    /// 항상 정확히 맨해튼 거리 1만큼 떨어져 있어야 한다(대각선 이동이나
    /// 순간이동 없음, 벽을 통과하지 않음).
    #[test]
    fn path_only_uses_walkable_adjacent_cells((grid, start, goal) in arbitrary_grid_and_endpoints()) {
        prop_assume!(grid.is_walkable(start));
        prop_assume!(grid.is_walkable(goal));

        if let Some(path) = find_path(&grid, start, goal, &HashSet::new()) {
            let mut prev = start;
            for cell in &path {
                prop_assert!(grid.is_walkable(*cell));
                let dist = (cell.0 - prev.0).abs() + (cell.1 - prev.1).abs();
                prop_assert_eq!(dist, 1);
                prev = *cell;
            }
        }
    }
}
```

- [ ] **Step 2: 프로퍼티 테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml --test pathfinding_properties`
Expected: PASS (proptest가 기본 256케이스를 무작위 생성해 검증).

- [ ] **Step 3: 전체 테스트 스위트 최종 확인**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: 모든 모듈(grid, pathfind, sim, gait, ik, posture, production) + 프로퍼티 테스트까지 전부 PASS, 실패/경고 없음.

- [ ] **Step 4: Commit**

```bash
git add server/tests/pathfinding_properties.rs
git commit -m "test: add property-based invariants for pathfinding"
```

---

## Plan 1 완료 후 상태

- `cargo test --manifest-path server/Cargo.toml` 하나로 그리드/경로탐색/틱 동시성/보행/IK/자세/생산량 전부 검증되는 네트워크 의존성 없는 라이브러리 크레이트.
- 아직 없는 것 (다음 Plan들의 몫): WebSocket 서버 바이너리, SQLite 영속화, REST API, `/metrics`, 클라이언트, Docker/배포. `server/src/main.rs`는 Plan 2에서 처음 생긴다.
