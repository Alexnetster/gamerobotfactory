# 조립 라인 (일자형 벨트 + 스테이션 + 헬퍼 로봇) 구현 계획

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 자유이동 픽업→운반→배치 사이클을 완전히 대체하는 일자형 조립 라인(제품이 스테이션을 거치며 단계별로 조립되고, 헬퍼 로봇이 부품/프레임을 보충하는 구조)을 구현한다.

**Architecture:** `sim_core`(`server/src/sim.rs`)에 `Product`/`Station`/`RobotRole` 엔티티와 결정적 틱 로직을 추가하고, 기존 `work_points`/U자 `belt_cells`/자유이동 픽업-운반-배치 로직을 완전히 제거한다. 프로토콜(`protocol.rs`/`delta.rs`)에 새 뷰 타입을 얹고, 클라이언트(`canvas.ts`/`protocol.ts`/`sidebar.ts`)가 일자 벨트와 제품 진행을 그리도록 바꾼다.

**Tech Stack:** Rust(axum, rayon, proptest) 서버 + TypeScript/Canvas 클라이언트, 기존 코드베이스 그대로.

**설계 문서:** [`docs/superpowers/specs/2026-07-24-assembly-line-design.md`](../specs/2026-07-24-assembly-line-design.md) — 이 계획의 각 태스크는 그 문서의 섹션 번호를 인용한다.

**주의 — 기존 DB 파일**: 이 기능을 배포하기 전에 로컬 `gamerobotfactory.sqlite3`(있다면)를 지운다. 옛 스키마의 로봇 상태 가정과 새 구조가 맞지 않기 때문(설계문서 §9). 코드 변경은 아니고 배포 시 수동으로 한 번 처리한다 — Task 10에서 다시 상기시킨다.

---

### Task 1: `sim_core` 데이터 모델 — `Product`/`Station`/`RobotRole`

**Files:**
- Modify: `server/src/sim.rs` (구조체/상수 추가, `SimState`에 필드 추가)
- Modify: `server/src/game_state.rs:155` (`empty_state()` 헬퍼)
- Modify: `server/src/main.rs:86,378` (`initial_state()`, `safe_tick_passes_through_normal_ticks_unchanged` 테스트)
- Modify: `server/src/protocol.rs:405` (`to_snapshot_reflects_current_game_state` 테스트)
- Modify: `server/tests/tick_properties.rs:35,66` (두 `arbitrary_sim_state*` 헬퍼)

이 태스크는 데이터 모델만 추가한다 — 틱 로직(제품 이동/조립)은 Task 2, 헬퍼 로직은 Task 3에서 다룬다.

- [ ] **Step 1: `sim.rs`에 레이아웃 상수 + `Product`/`Station`/`RobotRole` 추가**

`server/src/sim.rs`의 기존 상수들(`WEAR_LIMIT_TICKS` 등) 바로 아래, `PICKUP_SEED`/`PLACE_SEED` 다음 줄에 추가:

```rust
// 조립 라인 레이아웃(설계문서 §1) — 그리드(9x7, main.rs::initial_state와 일치)
// 가운데 가로줄이 벨트, 그 위 칸이 창고 구역. 값 자체는 이 레이아웃
// 하나로 고정이라 튜닝 대상이 아니다(그리드 크기가 바뀌면 같이 재검토).
pub const STATION_COUNT: usize = 3;
pub const BELT_ROW: i32 = 3;
pub const BELT_START_X: i32 = 1;
pub const BELT_END_X: i32 = 7; // 이 칸에 도달한 제품은 반출(완성)되어 다음 틱에 사라진다
pub const STATION_XS: [i32; STATION_COUNT] = [2, 4, 6];
pub const STATION_ROBOT_ROW: i32 = 2; // 벨트(y=3) 바로 위, 벨트 칸이 아님
pub const WAREHOUSE_CELL: CellId = (4, 0); // 헬퍼 로봇의 대표 출발/도착 칸(창고 구역 y=0..=1 중 하나)
pub const STATION_MAX_INVENTORY: u32 = 5;
pub const ASSEMBLY_TICKS: u32 = 20; // 조립 로봇의 스테이션당 작업 시간 — 튜닝 대상
pub const HELPER_PICKUP_TICKS: u32 = 20; // 헬퍼가 창고에서 집어드는 시간 — 튜닝 대상
pub const HELPER_DROP_TICKS: u32 = 20; // 헬퍼가 목적지에 내려놓는 시간 — 튜닝 대상
```

같은 파일, `Robot` 구조체(현재 99번 줄 근처) **바로 위**에 추가:

```rust
/// 로봇의 역할(설계문서 §4) — `Assembly`는 `station_index`가 가리키는
/// 스테이션 옆에 고정되어 절대 이동하지 않는다. `Helper`는 창고와
/// 스테이션/라인 시작점 사이를 오간다. 기본값은 `Helper`(아래
/// `Robot::new`) — 조립 로봇 3대는 `game_state.rs`(Task 4)가 스폰 직후
/// 명시적으로 `role`을 덮어써서 만든다. `Robot::new`의 시그니처를 바꾸지
/// 않는 이유: 이 필드를 생성자 파라미터로 추가하면 기존 호출부
/// 수십 곳(모든 테스트 포함)이 전부 깨지는데, 그 호출부 대부분은 role과
/// 무관한 걸(마모/고장/이동 충돌 등) 검증하는 테스트라 다 고칠 가치가
/// 없다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotRole {
    Assembly { station_index: u8 },
    Helper,
}
```

`Robot` 구조체에 필드 추가(기존 필드들 다음):

```rust
pub struct Robot {
    // ...기존 필드 그대로...
    pub carrying: bool,
    pub work_ticks_remaining: u32,
    pub role: RobotRole,
}
```

`Robot::new`(생성자 본문)에 기본값 추가:

```rust
impl Robot {
    pub fn new(id: u32, pos: CellId, goal: CellId) -> Self {
        Robot {
            // ...기존 필드 그대로...
            carrying: false,
            work_ticks_remaining: 0,
            role: RobotRole::Helper,
        }
    }
    // ...wear_ratio 그대로...
}
```

`Product`/`Station`은 `Robot`/`RobotStatus` 정의 다음, `SimState` 정의 **바로 위**에 추가:

```rust
/// 벨트 위를 흐르는 제품(드론) — 설계문서 §2. `stage`는 지금까지 통과한
/// 스테이션 수(0=빈 프레임, 3=완성). `work_ticks_remaining > 0`이면
/// 지금 스테이션에 정지해 조립 카운트다운 중 — 로봇의 같은 이름 필드와
/// 똑같은 의미(0이면 이동 가능, 0보다 크면 제자리).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Product {
    pub id: u32,
    pub stage: u8,
    pub pos: CellId,
    pub work_ticks_remaining: u32,
}

impl Product {
    pub fn new(id: u32, pos: CellId) -> Self {
        Product { id, stage: 0, pos, work_ticks_remaining: 0 }
    }
}

/// 조립 스테이션(설계문서 §3) — `belt_cell`이 제품이 실제로 멈추는 칸,
/// `robot_cell`이 그 옆에 고정된 조립 로봇의 자리. `index`가 `STATION_XS`의
/// 인덱스이자, 제품의 `stage`와 대응한다(스테이션 N은 stage==N인 제품만
/// 처리하고 그 결과 stage를 N+1로 올린다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Station {
    pub index: u8,
    pub robot_cell: CellId,
    pub belt_cell: CellId,
    pub part_inventory: u32,
}

impl Station {
    pub fn new(index: u8) -> Self {
        let x = STATION_XS[index as usize];
        Station {
            index,
            robot_cell: (x, STATION_ROBOT_ROW),
            belt_cell: (x, BELT_ROW),
            part_inventory: STATION_MAX_INVENTORY,
        }
    }
}
```

- [ ] **Step 2: `SimState`에 `products`/`stations` 필드 + 편의 생성자 추가**

`SimState` 정의(현재 188~193번 줄)를 다음으로 교체:

```rust
#[derive(Debug, Clone)]
pub struct SimState {
    pub grid: Arc<Grid>,
    pub robots: Vec<Robot>,
    pub products: Vec<Product>,
    pub stations: Vec<Station>,
    pub tick_count: u64,
}

impl SimState {
    /// 스테이션 3개(항상 `STATION_COUNT`개, 전부 재고 가득 참)로
    /// 초기화된 새 상태를 만든다 — 대부분의 생성 코드는 제품 없이
    /// `tick_count: 0`으로 시작하므로, 매번 이 보일러플레이트를 반복하는
    /// 대신 이 생성자 하나로 통일한다.
    pub fn new(grid: Arc<Grid>, robots: Vec<Robot>) -> Self {
        SimState {
            grid,
            robots,
            products: Vec::new(),
            stations: (0..STATION_COUNT as u8).map(Station::new).collect(),
            tick_count: 0,
        }
    }
}
```

`tick()` 함수 마지막 줄(현재 252번 줄, `SimState { grid: state.grid.clone(), robots: new_robots, tick_count: state.tick_count + 1 }`)은 이 태스크에서는 그대로 두되 **컴파일이 깨지지 않도록만** 고친다(제품/스테이션 실제 전진 로직은 Task 2):

```rust
    SimState {
        grid: state.grid.clone(),
        robots: new_robots,
        products: state.products.clone(),
        stations: state.stations.clone(),
        tick_count: state.tick_count + 1,
    }
```

- [ ] **Step 3: 기존 `SimState { .. }` 리터럴 호출부를 전부 고쳐 컴파일되게 한다**

아래 5개 파일의 리터럴을 각각 고친다(전부 `products`/`stations` 필드가 빠져서 컴파일 에러가 나는 곳들 — `cargo build --all-targets`로 전체 목록을 재확인하고 빠짐없이 고칠 것):

`server/src/sim.rs:463-464`(`simple_state` 헬퍼)를 다음으로 교체:

```rust
    fn simple_state(width: i32, height: i32) -> SimState {
        SimState::new(Arc::new(Grid::new(width, height)), Vec::new())
    }
```

`server/src/sim.rs:867`(`full_work_cycle_...` 테스트, 이 테스트는 Task 2에서 완전히 삭제될 예정이지만 이 태스크 시점엔 아직 컴파일은 되어야 하므로):

```rust
        let mut state = SimState::new(grid.clone(), vec![Robot::new(7, pickup, pickup)]);
```

`server/src/game_state.rs:155`(`empty_state()`):

```rust
    fn empty_state() -> GameState {
        GameState::new(SimState::new(Arc::new(Grid::new(5, 5)), Vec::new()))
    }
```

(이 파일 상단에 `use std::sync::Arc;`가 이미 있는지 확인 — 없으면 추가.)

`server/src/main.rs:86`(`initial_state()`):

```rust
    let sim = SimState::new(Arc::new(Grid::new(9, 7)), Vec::new());
```

`server/src/main.rs:378`(`safe_tick_passes_through_normal_ticks_unchanged`):

```rust
    fn safe_tick_passes_through_normal_ticks_unchanged() {
        let mut sim = SimState::new(Arc::new(Grid::new(3, 3)), Vec::new());
        sim.tick_count = 5;
        let result = safe_tick(&sim, false);
        assert!(result.is_some());
        assert_eq!(result.unwrap().tick_count, 6);
    }
```

`server/src/protocol.rs:405`(`to_snapshot_reflects_current_game_state`):

```rust
        let mut sim = SimState::new(Arc::new(Grid::new(3, 3)), Vec::new());
        sim.tick_count = 5;
        let mut state = GameState::new(sim);
```

`server/tests/tick_properties.rs:35,66`(두 곳 동일하게):

```rust
        SimState::new(Arc::new(Grid::new(SIZE, SIZE)), robots)
```

(두 함수 다 `tick_count: 0`으로 시작하던 것과 동작이 같다 — `SimState::new`의 기본값.)

- [ ] **Step 4: 빌드 확인**

Run: `cargo build --all-targets` (저장소 루트 또는 `server/` 디렉터리, `Cargo.toml` 위치 기준)
Expected: 에러 없이 컴파일 성공. 에러가 나면 위에서 놓친 `SimState { .. }` 리터럴이 더 있다는 뜻이니 그 위치를 찾아 같은 패턴으로 고친다.

- [ ] **Step 5: 새 타입 기본 테스트**

`server/src/sim.rs`의 `#[cfg(test)] mod tests` 블록 끝에 추가:

```rust
    #[test]
    fn station_new_derives_correct_cells_from_index() {
        let s0 = Station::new(0);
        assert_eq!(s0.belt_cell, (STATION_XS[0], BELT_ROW));
        assert_eq!(s0.robot_cell, (STATION_XS[0], STATION_ROBOT_ROW));
        assert_eq!(s0.part_inventory, STATION_MAX_INVENTORY);
    }

    #[test]
    fn sim_state_new_seeds_exactly_station_count_stations_with_no_products() {
        let state = SimState::new(Arc::new(Grid::new(9, 7)), Vec::new());
        assert_eq!(state.stations.len(), STATION_COUNT);
        assert!(state.products.is_empty());
        for (i, station) in state.stations.iter().enumerate() {
            assert_eq!(station.index, i as u8);
        }
    }

    #[test]
    fn new_robot_defaults_to_helper_role() {
        let robot = Robot::new(1, (0, 0), (0, 0));
        assert_eq!(robot.role, RobotRole::Helper);
    }
```

- [ ] **Step 6: 테스트 실행**

Run: `cargo test --all` (server 디렉터리 기준)
Expected: 전부 통과(기존 테스트 포함 — 이 태스크는 아직 동작을 바꾸지 않고 데이터 모델만 얹었다).

- [ ] **Step 7: 커밋**

```bash
git add server/src/sim.rs server/src/game_state.rs server/src/main.rs server/src/protocol.rs server/tests/tick_properties.rs
git commit -m "feat: add Product/Station/RobotRole data model for the assembly line"
```

---

### Task 2: 제품 이동 + 스테이션 조립 틱 로직 (기존 자유이동 사이클 제거)

**Files:**
- Modify: `server/src/sim.rs` (`plan_robot` 전면 교체, `work_points`/U자 `belt_cells`/관련 테스트 삭제, 제품 틱 로직 추가)

이 태스크가 "완전 대체"의 핵심이다 — 컨베이어가 켜졌을 때의 기존 픽업/운반/배치 로직을 통째로 걷어내고, `Assembly` 로봇은 항상 고정, `Helper` 로봇은 아직 아무 일도 하지 않는(Task 3에서 채움) 상태로 만든 뒤, 제품이 스스로 벨트를 따라 흐르며 스테이션에서 조립되는 로직을 추가한다. 이 태스크 시점에는 스테이션 재고가 바닥나도 채워주는 주체가 없다(Task 3) — 그래서 재고 소진 시나리오는 이 태스크에서 "제자리에서 대기"까지만 검증한다.

- [ ] **Step 1: `MoveIntent`을 로봇/제품 공용으로 리네이밍**

`server/src/sim.rs`의 `MoveIntent` 구조체(현재 195~200번 줄)를 교체:

```rust
/// 한 틱 안에서 무언가(로봇 또는 제품)가 `from`에서 `to`로 이동하려는
/// 의도. 로봇과 제품은 서로 다른 배열에 살지만 "같은 칸을 여러이 동시에
/// 노리면 id가 작은 쪽이 이긴다"는 타이브레이크 규칙은 완전히 같으므로
/// (설계문서 §7), 이 구조체와 `resolve_intents`를 그대로 공유한다 —
/// 제품 전용으로 거의 같은 함수를 새로 만드는 건 이 프로젝트의 중복
/// 방지 원칙에 어긋난다.
#[derive(Debug, Clone, Copy)]
struct MoveIntent {
    mover_id: u32,
    from: CellId,
    to: CellId,
}
```

`tick()` 안의 intents 생성 부분(현재 215~224번 줄)에서 필드명을 갱신:

```rust
    let intents: Vec<MoveIntent> = state
        .robots
        .iter()
        .zip(planned.iter())
        .map(|(original, planned)| MoveIntent {
            mover_id: original.id,
            from: original.pos,
            to: planned.pos,
        })
        .collect();
```

`resolve_intents` 함수 본문(현재 440~457번 줄)의 `intent.robot_id`를 전부 `intent.mover_id`로 바꾼다:

```rust
fn resolve_intents(intents: &[MoveIntent]) -> Vec<CellId> {
    let mut winner_by_cell: HashMap<CellId, u32> = HashMap::new();
    for intent in intents {
        winner_by_cell
            .entry(intent.to)
            .and_modify(|winner| {
                if intent.mover_id < *winner {
                    *winner = intent.mover_id;
                }
            })
            .or_insert(intent.mover_id);
    }

    intents
        .iter()
        .map(|intent| if winner_by_cell[&intent.to] == intent.mover_id { intent.to } else { intent.from })
        .collect()
}
```

- [ ] **Step 2: `plan_robot`의 컨베이어-켜짐 분기를 역할 기반으로 전면 교체**

`plan_robot` 함수(현재 313~366번 줄)를 통째로 교체:

```rust
fn plan_robot(
    grid: &Grid,
    robot: &Robot,
    occupied: &HashSet<CellId>,
    tick_count: u64,
    conveyor_running: bool,
    active_stations: &HashSet<u8>,
) -> Robot {
    let mut next = update_status(robot.clone(), tick_count);

    if next.status != RobotStatus::Operational {
        return next;
    }

    match next.role {
        // 조립 로봇은 절대 이동하지 않는다(설계문서 §1, §4) — 실제 조립
        // 작업(재고 소모/제품 stage 증가)은 로봇이 아니라 제품 쪽 틱
        // 로직(plan_products, 아래)이 스테이션 상태를 직접 갱신한다.
        // 여기서는 `task`만 "지금 그 스테이션에 제품이 있고 조립
        // 카운트다운 중인가"를 반영해서 채운다 — `update_status`의 마모
        // 축적 조건(`task == Picking`)이 조립 로봇에도 그대로 적용되게
        // 하기 위함(설계문서 §9 "로봇 내구도/고장/수리는 그대로 재사용").
        // `active_stations`는 `tick()`이 *이전 틱* 제품/스테이션 스냅샷에서
        // 미리 계산해 넘겨준 값이라(이중버퍼 패턴, 로봇 이동의 `occupied`와
        // 같은 이유), 위 `update_status`가 방금 소비한 `next.task`(이전
        // 틱에 이 함수가 설정해 둔 값)와 자연스럽게 한 틱 지연이 있다 —
        // 기존 마모 축적도 원래 이런 한 틱 지연 패턴이었으므로(§ worn_ticks
        // 관련 기존 테스트 참고) 새로 생긴 문제가 아니다.
        RobotRole::Assembly { station_index } => {
            next.task = if active_stations.contains(&station_index) { Task::Picking } else { Task::Idle };
            next
        }
        RobotRole::Helper => {
            // Task 3에서 창고<->스테이션/라인시작 이동 로직을 채운다.
            // 이 태스크 시점에는 헬퍼가 아무 일도 하지 않는다(제자리 대기)
            // — conveyor_running과 무관하게 항상 정지.
            next
        }
    }
}
```

`advance_along_path` 함수(현재 371~403번 줄)는 이제 아무도 호출하지 않게 된다(순찰도 자유이동 작업 사이클도 없어졌으므로) — **완전히 삭제한다.**

- [ ] **Step 3: 순찰(`patrol_points`)과 자유이동 작업 사이클(`work_points`/U자 `belt_cells`) 완전히 삭제**

다음 함수들을 `server/src/sim.rs`에서 전부 삭제한다:
- `patrol_points` (255~267번 줄)
- `next_patrol_goal` (269~274번 줄)
- `work_points` (276~301번 줄, 그 위 doc 주석 포함)
- `belt_cells` (303~311번 줄, 그 위 doc 주석 포함)

**근거(계획 시점 판단, 설계문서 §9 범위 확장)**: 설계문서 §9는 `work_points`/U자 `belt_cells`만 명시적으로 나열했지만, `patrol_points`/`next_patrol_goal`은 오직 "컨베이어가 꺼졌을 때 로봇이 순찰한다"는, 이제는 존재하지 않는 옛 분기에서만 쓰였다. 조립 로봇은 절대 이동하지 않고(위 Step 2), 헬퍼 로봇은 컨베이어가 꺼지면(아래) 그냥 멈추는 게 "라인 전체가 꺼짐"이라는 새 의미와 더 일관되므로, 순찰을 부활시키지 않고 죽은 코드로 남기는 대신 같이 삭제한다.

`plan_robot`의 컨베이어-꺼짐 처리도 Step 2의 교체본에 포함됐다 — 위 §5(설계문서)에는 명시 안 됐지만, "컨베이어 꺼짐 = 라인 전체 일시정지"라는 기존 의미를 로봇에도 그대로 확장한 것(제품/스테이션이 멈추는 것과 대칭): `conveyor_running == false`일 때도 Assembly/Helper 둘 다 그냥 제자리(Step 2 코드가 이미 그렇게 되어 있음 — 별도 분기 불필요, 이동 로직 자체가 없어졌으므로).

다음 테스트들도 이 삭제와 함께 `server/src/sim.rs`에서 제거한다(전부 삭제된 함수를 검증하던 것들):
- `patrol_points_are_always_distinct_for_a_reasonably_sized_grid`
- `next_patrol_goal_alternates_between_the_two_patrol_points`
- `work_points_are_always_distinct_for_a_reasonably_sized_grid`
- `work_points_always_land_on_a_conveyor_belt_cell`
- `belt_cells_form_a_u_shape_open_on_the_right`
- `full_work_cycle_moves_to_pickup_picks_up_carries_and_places`
- `turning_conveyor_off_mid_work_resets_task_and_carrying_immediately`
- `manual_trigger_arm_action_cannot_skip_the_work_cycle_wait`
- `robot_picks_a_new_patrol_goal_and_moves_on_the_same_tick_it_arrives`
- `leg_cycle_progress_advances_when_patrol_reassignment_causes_movement`
- `facing_holds_last_direction_while_stationary` — 이 테스트의 "서쪽으로 목표를 바꿔 이동" 부분은 `advance_along_path`(삭제됨)를 직접 검증하던 것이라 함께 삭제. 나머지 로봇 이동/타이브레이크/고장 테스트(`robot_moves_one_step_toward_goal_each_tick` 등)는 로봇이 더 이상 스스로 목표를 향해 걷지 않으므로(조립 로봇은 고정, 헬퍼는 Task 3 전까지 정지) 사실 이 태스크 이후로는 항상 제자리에 머무는 상태만 검증하게 된다 — 이들 중 "특정 지점까지 걸어간다"류 테스트(`robot_moves_one_step_toward_goal_each_tick`, `robot_does_not_move_on_a_tick_that_is_not_a_patrol_interval_multiple`, `lower_id_wins_when_two_robots_target_same_cell`, `tick_is_deterministic_across_repeated_runs`, `facing_updates_to_match_actual_movement_direction`, `facing_does_not_change_when_a_robot_loses_its_tiebreak`)도 함께 삭제한다 — 이동/타이브레이크/facing 로직 자체(`tick()`의 intent/resolve_intents 부분, `Direction::from_move`)는 계속 존재하고 Step 5에서 제품 전진 경로로 다시 검증되므로 로직 커버리지 손실은 아니다. 다만 `PATROL_MOVE_INTERVAL_TICKS`/`REPATH_INTERVAL` 상수와 `Grid::neighbors`/`find_path` 자체는 이제 아무도 안 쓰게 된다 — 이 두 상수와 `find_path` import(`use crate::pathfind::find_path;`, 1번째 줄)를 삭제한다. `LEG_CYCLE_SPEED`는 여전히 쓰인다(로봇이 실제로 이동할 때 다리 애니메이션 진행 — 헬퍼가 Task 3에서 다시 걷게 되므로 유지).

- [ ] **Step 4: 제품 전진 + 스테이션 조립 틱 로직 추가**

`resolve_intents` 함수 바로 다음에 추가:

```rust
/// 제품 한 틱 전진 + 스테이션 조립 진행(설계문서 §5, §7). 순수 함수 —
/// `products`/`stations`를 값으로 받아 새 값을 반환한다. 로봇과 마찬가지로
/// "틱 시작 시점 스냅샷만 읽고 이동 여부를 결정"하는 이중버퍼 패턴을
/// 따른다(설계문서 §7) — 두 제품이 같은 칸을 노리면 `resolve_intents`가
/// (로봇과 똑같이) id가 작은 쪽을 이긴다.
fn plan_products(products: &[Product], stations: &[Station]) -> (Vec<Product>, Vec<Station>) {
    let occupied: HashSet<CellId> = products.iter().map(|p| p.pos).collect();
    let mut stations = stations.to_vec();

    // 1단계: 이미 스테이션에 서 있는 제품의 조립 진행/시작.
    let mut updated: Vec<Product> = products
        .iter()
        .cloned()
        .map(|mut p| {
            let station = stations
                .iter_mut()
                .find(|s| s.belt_cell == p.pos && s.index as usize == p.stage as usize);
            if let Some(station) = station {
                if p.work_ticks_remaining > 0 {
                    p.work_ticks_remaining -= 1;
                    if p.work_ticks_remaining == 0 {
                        p.stage += 1;
                    }
                } else if station.part_inventory > 0 {
                    station.part_inventory -= 1;
                    p.work_ticks_remaining = ASSEMBLY_TICKS;
                }
                // else: 재고 0 — 제품은 그 자리에서 그냥 대기(설계문서 §5-2).
            }
            p
        })
        .collect();

    // 2단계: 전진. "이번 틱에 움직이지 않는" 제품(조립 카운트다운 중이거나,
    // 재고가 없어 대기 중인 제품)이 서 있는 칸은 다른 제품이 들어갈 수
    // 없다 — 로봇의 `occupied` 검사와 같은 이유로, 틱 시작 시점 스냅샷
    // (`occupied`)을 기준으로 판단해 한 틱 안에서 여러 칸이 도미노처럼
    // 한꺼번에 밀리는 걸 막는다(로봇 이동과 동일한 보수적 규칙).
    let blocked: HashSet<CellId> = updated
        .iter()
        .filter(|p| {
            p.work_ticks_remaining > 0
                || stations.iter().any(|s| s.belt_cell == p.pos && s.index as usize == p.stage as usize)
        })
        .map(|p| p.pos)
        .collect();

    let intents: Vec<MoveIntent> = updated
        .iter()
        .filter(|p| !blocked.contains(&p.pos))
        .filter_map(|p| {
            let target = (p.pos.0 + 1, BELT_ROW);
            if occupied.contains(&target) {
                None
            } else {
                Some(MoveIntent { mover_id: p.id, from: p.pos, to: target })
            }
        })
        .collect();

    let resolved = resolve_intents(&intents);
    let resolved_by_id: HashMap<u32, CellId> =
        intents.iter().zip(resolved).map(|(intent, pos)| (intent.mover_id, pos)).collect();
    for p in updated.iter_mut() {
        if let Some(&new_pos) = resolved_by_id.get(&p.id) {
            p.pos = new_pos;
        }
    }

    // 3단계: 반출 — `BELT_END_X`는 순수 종료 마커라 제품이 실제로 그
    // 칸에 머무는 모습은 렌더링되지 않는다(설계문서 §5-3) — 도착하는
    // 순간 제거된다. 완료 감지(생산량 집계)는 sim_core 밖(main.rs)에서
    // "이전 틱엔 있었는데 이번 틱엔 없어진 제품 id"로 한다(기존
    // `detect_completed_placements`와 같은 패턴).
    let remaining: Vec<Product> = updated.into_iter().filter(|p| p.pos.0 < BELT_END_X).collect();

    (remaining, stations)
}
```

`safe_plan_robot`(현재 407~409번 줄)도 같은 파라미터를 추가해 그대로 전달하도록 교체:

```rust
fn safe_plan_robot(
    grid: &Grid,
    robot: &Robot,
    occupied: &HashSet<CellId>,
    tick_count: u64,
    conveyor_running: bool,
    active_stations: &HashSet<u8>,
) -> Robot {
    safe_call(robot, || plan_robot(grid, robot, occupied, tick_count, conveyor_running, active_stations))
}
```

`tick()` 함수(현재 206~253번 줄)를 교체해 제품 로직을 끼워넣는다. **`active_stations`는 이번 틱이 시작되기 전(직전 틱) 제품/스테이션 스냅샷에서 계산**한다 — 로봇 계획이 `occupied`(직전 틱 로봇 위치)만 읽는 것과 같은 이중버퍼 원칙:

```rust
pub fn tick(state: &SimState, conveyor_running: bool) -> SimState {
    let occupied: HashSet<CellId> = state.robots.iter().map(|r| r.pos).collect();
    let active_stations: HashSet<u8> = state
        .stations
        .iter()
        .filter(|s| state.products.iter().any(|p| p.pos == s.belt_cell && p.work_ticks_remaining > 0))
        .map(|s| s.index)
        .collect();

    let planned: Vec<Robot> = state
        .robots
        .par_iter()
        .map(|robot| safe_plan_robot(&state.grid, robot, &occupied, state.tick_count, conveyor_running, &active_stations))
        .collect();

    let intents: Vec<MoveIntent> = state
        .robots
        .iter()
        .zip(planned.iter())
        .map(|(original, planned)| MoveIntent {
            mover_id: original.id,
            from: original.pos,
            to: planned.pos,
        })
        .collect();

    let resolved_positions = resolve_intents(&intents);

    let new_robots: Vec<Robot> = state
        .robots
        .iter()
        .zip(planned)
        .zip(resolved_positions)
        .map(|((original, mut robot), final_pos)| {
            let lost_tiebreak = final_pos != robot.pos;
            robot.pos = final_pos;
            if lost_tiebreak {
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

    let (new_products, new_stations) = if conveyor_running {
        plan_products(&state.products, &state.stations)
    } else {
        (state.products.clone(), state.stations.clone())
    };

    SimState {
        grid: state.grid.clone(),
        robots: new_robots,
        products: new_products,
        stations: new_stations,
        tick_count: state.tick_count + 1,
    }
}
```

- [ ] **Step 5: 새 테스트 — 제품 전진/조립/반출**

`server/src/sim.rs` 테스트 모듈 끝에 추가(먼저 Step 3에서 지시한 테스트들을 삭제한 뒤 추가할 것):

```rust
    fn state_with_products(products: Vec<Product>) -> SimState {
        let mut state = SimState::new(Arc::new(Grid::new(9, 7)), Vec::new());
        state.products = products;
        state
    }

    #[test]
    fn product_advances_one_cell_per_tick_when_not_blocked_by_a_station() {
        let mut state = state_with_products(vec![Product::new(1, (5, BELT_ROW))]);
        // (5, BELT_ROW)는 스테이션 칸이 아니다(STATION_XS = [2, 4, 6]).
        state = tick(&state, true);
        assert_eq!(state.products[0].pos, (6, BELT_ROW));
    }

    #[test]
    fn product_stops_at_its_matching_station_and_assembles_over_assembly_ticks() {
        let station_x = STATION_XS[0];
        let mut state = state_with_products(vec![Product::new(1, (station_x, BELT_ROW))]);

        state = tick(&state, true);
        assert_eq!(state.products[0].pos, (station_x, BELT_ROW), "조립 중엔 이동하지 않아야 한다");
        assert_eq!(state.products[0].stage, 0, "아직 조립이 끝나지 않았다");
        assert_eq!(state.stations[0].part_inventory, STATION_MAX_INVENTORY - 1, "재고가 정확히 1 소모돼야 한다");

        for _ in 0..ASSEMBLY_TICKS - 1 {
            state = tick(&state, true);
        }
        assert_eq!(state.products[0].stage, 0, "ASSEMBLY_TICKS - 1번째 틱까지는 아직 stage가 오르면 안 된다");

        state = tick(&state, true);
        assert_eq!(state.products[0].stage, 1, "정확히 ASSEMBLY_TICKS번째 틱에 stage가 올라야 한다");
    }

    #[test]
    fn product_waits_in_place_when_its_station_has_no_inventory() {
        let station_x = STATION_XS[0];
        let mut state = state_with_products(vec![Product::new(1, (station_x, BELT_ROW))]);
        state.stations[0].part_inventory = 0;

        for _ in 0..10 {
            state = tick(&state, true);
        }

        assert_eq!(state.products[0].pos, (station_x, BELT_ROW), "재고가 없으면 계속 그 자리에서 대기해야 한다");
        assert_eq!(state.products[0].stage, 0);
        assert_eq!(state.products[0].work_ticks_remaining, 0, "재고가 없으면 조립 카운트다운이 시작되면 안 된다");
    }

    #[test]
    fn product_resumes_automatically_once_inventory_is_replenished() {
        let station_x = STATION_XS[0];
        let mut state = state_with_products(vec![Product::new(1, (station_x, BELT_ROW))]);
        state.stations[0].part_inventory = 0;
        state = tick(&state, true);
        assert_eq!(state.products[0].work_ticks_remaining, 0);

        state.stations[0].part_inventory = STATION_MAX_INVENTORY; // 헬퍼가 보충했다고 가정(Task 3에서 실제 배선)
        state = tick(&state, true);
        assert!(state.products[0].work_ticks_remaining > 0, "재고가 채워지면 같은 틱에 바로 조립이 재개돼야 한다");
    }

    #[test]
    fn a_stalled_product_blocks_the_one_behind_it() {
        let station_x = STATION_XS[0];
        let mut state = state_with_products(vec![
            Product::new(1, (station_x, BELT_ROW)),
            Product::new(2, (station_x - 1, BELT_ROW)),
        ]);
        state.stations[0].part_inventory = 0;

        state = tick(&state, true);

        assert_eq!(state.products.iter().find(|p| p.id == 1).unwrap().pos, (station_x, BELT_ROW));
        assert_eq!(
            state.products.iter().find(|p| p.id == 2).unwrap().pos,
            (station_x - 1, BELT_ROW),
            "앞이 막혀 있으면 뒤 제품도 전진하면 안 된다"
        );
    }

    #[test]
    fn product_completing_the_final_station_and_reaching_the_belt_end_is_removed() {
        let mut state = state_with_products(vec![{
            let mut p = Product::new(1, (BELT_END_X - 1, BELT_ROW));
            p.stage = STATION_COUNT as u8; // 이미 세 스테이션을 다 거쳤다
            p
        }]);

        state = tick(&state, true);

        assert!(state.products.is_empty(), "벨트 끝에 도달한 완성품은 사라져야 한다(반출)");
    }

    #[test]
    fn products_do_not_move_or_assemble_while_conveyor_is_off() {
        let station_x = STATION_XS[0];
        let state = state_with_products(vec![Product::new(1, (station_x, BELT_ROW))]);

        let next = tick(&state, false);

        assert_eq!(next.products[0].pos, (station_x, BELT_ROW));
        assert_eq!(next.products[0].work_ticks_remaining, 0);
        assert_eq!(next.stations[0].part_inventory, STATION_MAX_INVENTORY, "컨베이어가 꺼져 있으면 재고도 소모되면 안 된다");
    }

    #[test]
    fn assembly_role_robot_never_moves_even_with_conveyor_running() {
        let mut robot = Robot::new(1, (STATION_XS[0], STATION_ROBOT_ROW), (0, 0));
        robot.role = RobotRole::Assembly { station_index: 0 };
        let mut state = SimState::new(Arc::new(Grid::new(9, 7)), vec![robot]);

        for _ in 0..20 {
            state = tick(&state, true);
        }

        assert_eq!(state.robots[0].pos, (STATION_XS[0], STATION_ROBOT_ROW), "조립 로봇은 절대 이동하면 안 된다");
    }
```

- [ ] **Step 6: 테스트 실행**

Run: `cargo test --all` (server 디렉터리)
Expected: 전부 통과. 삭제 대상으로 지목된 테스트 이름이 남아있으면(Step 3을 빠뜨림) 컴파일 에러(`work_points`/`belt_cells`/`patrol_points`/`advance_along_path` 등을 참조) 또는 사용하지 않는 함수 경고가 뜬다 — 전부 정리될 때까지 반복.

- [ ] **Step 7: 커밋**

```bash
git add server/src/sim.rs
git commit -m "feat: replace free-roam pick/carry/place cycle with product/station belt logic"
```

---

### Task 3: 헬퍼 로봇 — 작업 큐, 배정, 픽업/드롭 상태 기계

**Files:**
- Modify: `server/src/sim.rs` (`HelperTask` enum, `SimState.pending_helper_tasks`, 헬퍼 `plan_robot` 분기, 라인 시작점 프레임 보충, `products_never_occupy_the_same_cell` proptest)
- Modify: `server/tests/tick_properties.rs`

- [ ] **Step 1: `HelperTask` + 큐 필드 추가**

`server/src/sim.rs`의 `RobotRole` 정의 다음에 추가:

```rust
/// 헬퍼 로봇에게 배정 가능한 작업(설계문서 §6). `RestockStation`은
/// 창고→그 스테이션의 `robot_cell`로, `DeliverFrame`은 창고→라인
/// 시작점(`(BELT_START_X, BELT_ROW)`)으로 화물을 나른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperTask {
    RestockStation { station_index: u8 },
    DeliverFrame,
}

/// 헬퍼 한 대가 지금 어느 단계에 있는지 — 배정만 되고 아직 창고에
/// 도착 전인지, 픽업 카운트다운 중인지, 목적지로 이동 중인지, 드롭
/// 카운트다운 중인지. `Robot.carrying`(이동/드롭 단계 여부)과
/// `Robot.work_ticks_remaining`(픽업/드롭 카운트다운)을 그대로
/// 재사용하고(설계문서 §4), 이 필드는 "지금 무슨 작업을 배정받았는지"만
/// 담는다 — 배정 자체가 없으면 `None`(=Idle, 큐에서 다음 일을 기다림).
pub type HelperAssignment = Option<HelperTask>;
```

`Robot` 구조체에 필드 추가(마지막, `role` 다음):

```rust
    pub role: RobotRole,
    pub helper_assignment: HelperAssignment,
```

`Robot::new`에 기본값 추가(Task 1에서 이미 추가한 `role: RobotRole::Helper,` 다음 줄에):

```rust
            helper_assignment: None,
```

`SimState` 구조체에 큐 필드 추가:

```rust
pub struct SimState {
    pub grid: Arc<Grid>,
    pub robots: Vec<Robot>,
    pub products: Vec<Product>,
    pub stations: Vec<Station>,
    pub pending_helper_tasks: Vec<HelperTask>,
    pub tick_count: u64,
}
```

`SimState::new`에 빈 큐로 초기화 추가:

```rust
            pending_helper_tasks: Vec::new(),
```

`tick()`의 마지막 `SimState { .. }` 리터럴에 필드 추가:

```rust
        pending_helper_tasks: new_pending_helper_tasks,
```

(`new_pending_helper_tasks`는 Step 3에서 계산.)

- [ ] **Step 2: 헬퍼 로봇 이동/작업 로직**

`plan_robot`의 `RobotRole::Helper => { next }` 분기(Task 2 Step 2에서 만든 자리)를 교체:

```rust
            RobotRole::Helper => {
                if !conveyor_running {
                    return next;
                }
                plan_helper(grid, next, occupied, tick_count)
            }
```

`plan_robot` 함수 바로 다음에 새 함수 추가:

```rust
/// 헬퍼 로봇의 창고<->목적지 왕복(설계문서 §6). `helper_assignment`가
/// `None`이면 아무 것도 하지 않는다(작업 배정은 `tick()`이 로봇 목록
/// 전체를 보고 매 틱 결정하므로 이 함수 진입 전에 이미 채워져 있다고
/// 가정) — 배정된 작업이 있을 때의 이동/카운트다운만 여기서 처리한다.
fn plan_helper(grid: &Grid, mut next: Robot, occupied: &HashSet<CellId>, tick_count: u64) -> Robot {
    let Some(task) = next.helper_assignment else {
        return next;
    };

    let destination = match task {
        HelperTask::RestockStation { station_index } => {
            grid.robot_cell_for_station(station_index)
        }
        HelperTask::DeliverFrame => (BELT_START_X, BELT_ROW),
    };

    if next.work_ticks_remaining > 0 {
        next.work_ticks_remaining -= 1;
        return next;
    }

    let target = if next.carrying { destination } else { WAREHOUSE_CELL };

    if next.pos != target {
        if next.goal != target {
            next.goal = target;
            next.path.clear();
            next.ticks_until_repath = 0;
        }
        return advance_along_path(grid, next, occupied, tick_count);
    }

    next.work_ticks_remaining = if next.carrying { HELPER_DROP_TICKS } else { HELPER_PICKUP_TICKS };
    if !next.carrying {
        next.carrying = true; // 창고 도착 -> 픽업 카운트다운 시작(들었다고 가정, 드롭 시 실제 효과 적용은 tick()에서)
    }
    next
}
```

**주의**: 위 코드는 `grid.robot_cell_for_station`과 `advance_along_path`를 참조하는데, `advance_along_path`는 Task 2 Step 2에서 삭제했다 — **이 태스크에서 되살린다**(헬퍼가 다시 걷기 시작하므로). `plan_helper` 함수 바로 다음에 Task 2 이전 원본 그대로 추가:

```rust
fn advance_along_path(grid: &Grid, mut next: Robot, occupied: &HashSet<CellId>, tick_count: u64) -> Robot {
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

(이번엔 `PATROL_MOVE_INTERVAL_TICKS` 지연을 넣지 않는다 — 그건 순찰 전용 튜닝이었고 헬퍼는 매 틱 이동해도 무방하다. `tick_count` 파라미터는 인터페이스 일관성을 위해 남겨두되 이 함수 안에서는 안 쓰인다.) 삭제했던 `use crate::pathfind::find_path;`(1번째 줄)와 `const REPATH_INTERVAL: u32 = 10;`(Task 2에서 지웠다면)도 되살린다 — 파일 맨 위 확인.

`grid.robot_cell_for_station`은 `Grid`가 아니라 `sim.rs`의 자유 함수로 두는 게 더 자연스럽다(스테이션은 `sim.rs`의 개념이지 `grid.rs`의 개념이 아니다) — 위 `plan_helper`의 해당 줄을 아래처럼 바꾼다:

```rust
        HelperTask::RestockStation { station_index } => station_robot_cell(station_index),
```

그리고 `Station::new` 바로 다음에 헬퍼 함수 추가:

```rust
fn station_robot_cell(station_index: u8) -> CellId {
    (STATION_XS[station_index as usize], STATION_ROBOT_ROW)
}
```

- [ ] **Step 3: 작업 큐 채우기 + 배정 + 실제 재고/프레임 반영을 `tick()`에 배선**

`tick()` 함수에서 제품 로직 다음(Task 2 Step 4에서 추가한 `let (new_products, new_stations) = ...` 다음), 최종 `SimState { .. }` 리턴 전에 추가:

```rust
    let (new_robots, new_stations, new_products, new_pending_helper_tasks) =
        run_helper_logistics(new_robots, new_stations, new_products, state.pending_helper_tasks.clone(), conveyor_running);
```

(`new_robots`/`new_stations`/`new_products`는 각각 위에서 이미 계산된 값 — 이 호출로 재바인딩한다. Rust의 shadowing으로 자연스럽게 처리된다.)

`plan_products` 함수 다음에 새 함수 추가:

```rust
/// 헬퍼 로직 한 틱 분: (1) 재고/프레임 부족을 감지해 큐에 새 요청을
/// 추가(중복 방지, 설계문서 §6), (2) 노는 헬퍼에게 큐 맨 앞 요청을 배정,
/// (3) 드롭 카운트다운이 막 끝난 헬퍼의 화물을 실제 목적지에 반영
/// (재고 채우기 또는 새 프레임 생성)한다. 세 가지를 한 함수로 묶은 이유:
/// 셋 다 "이번 틱에 로봇/스테이션/제품 상태를 서로 참조하며 갱신"하는
/// 같은 트랜잭션의 부분들이라 나누면 오히려 상태를 두 번씩 넘겨야 한다.
fn run_helper_logistics(
    mut robots: Vec<Robot>,
    mut stations: Vec<Station>,
    mut products: Vec<Product>,
    mut pending: Vec<HelperTask>,
    conveyor_running: bool,
) -> (Vec<Robot>, Vec<Station>, Vec<Product>, Vec<HelperTask>) {
    if !conveyor_running {
        return (robots, stations, products, pending);
    }

    // (1) 새 요청 발생 — 이미 큐에 있거나 배정된 요청은 다시 만들지 않는다.
    let already_wanted = |task: HelperTask, pending: &[HelperTask], robots: &[Robot]| {
        pending.contains(&task) || robots.iter().any(|r| r.helper_assignment == Some(task))
    };

    for station in &stations {
        let task = HelperTask::RestockStation { station_index: station.index };
        if station.part_inventory == 0 && !already_wanted(task, &pending, &robots) {
            pending.push(task);
        }
    }

    let line_start = (BELT_START_X, BELT_ROW);
    let line_start_empty = !products.iter().any(|p| p.pos == line_start);
    if line_start_empty && !already_wanted(HelperTask::DeliverFrame, &pending, &robots) {
        pending.push(HelperTask::DeliverFrame);
    }

    // (2) 노는 헬퍼에게 배정 — 먼저 발생한 요청(큐 맨 앞)부터.
    for robot in robots.iter_mut() {
        if robot.role != RobotRole::Helper || robot.helper_assignment.is_some() {
            continue;
        }
        if pending.is_empty() {
            break;
        }
        robot.helper_assignment = Some(pending.remove(0));
    }

    // (3) 드롭 완료 반영 — work_ticks_remaining이 막 0이 된(carrying=true였던)
    // 헬퍼의 화물을 실제로 목적지에 적용하고 배정을 해제한다.
    for robot in robots.iter_mut() {
        if robot.role != RobotRole::Helper || !robot.carrying || robot.work_ticks_remaining != 0 {
            continue;
        }
        let Some(task) = robot.helper_assignment else { continue };
        let at_destination = match task {
            HelperTask::RestockStation { station_index } => robot.pos == station_robot_cell(station_index),
            HelperTask::DeliverFrame => robot.pos == line_start,
        };
        if !at_destination {
            continue;
        }
        match task {
            HelperTask::RestockStation { station_index } => {
                if let Some(station) = stations.iter_mut().find(|s| s.index == station_index) {
                    station.part_inventory = STATION_MAX_INVENTORY;
                }
            }
            HelperTask::DeliverFrame => {
                let new_id = products.iter().map(|p| p.id).max().map_or(0, |max| max + 1);
                products.push(Product::new(new_id, line_start));
            }
        }
        robot.carrying = false;
        robot.helper_assignment = None;
    }

    (robots, stations, products, pending)
}
```

**드롭 완료 판정 수정**: 위 `plan_helper`에서 `work_ticks_remaining`이 0이 되는 순간은 카운트다운이 다 끝난 "직후"인데, `plan_helper`가 실행되는 시점엔 아직 `carrying`을 false로 바꾸지 않는다(그건 위 `run_helper_logistics`의 (3)이 한다). 이 순서(로봇 개별 계획 → 헬퍼 로지스틱 후처리)가 지켜져야 하므로, `tick()` 안에서 `run_helper_logistics` 호출은 반드시 로봇 위치/카운트다운이 이미 확정된 뒤(`new_robots` 계산 이후)여야 한다 — 위 Step 3 서두의 배선이 이미 그렇게 되어 있다.

**픽업 완료 후 실제 이동 재개**: `plan_helper`에서 `next.carrying = true`로 세팅한 시점부터, 다음 틱부터는 `target = destination`(창고가 아니라 실제 목적지)이 되어 자연히 그쪽으로 걸어간다 — 별도 처리 불필요.

- [ ] **Step 4: 헬퍼 로직 테스트**

`server/src/sim.rs` 테스트 모듈에 추가:

```rust
    #[test]
    fn a_depleted_station_gets_exactly_one_restock_request_queued() {
        let mut state = SimState::new(Arc::new(Grid::new(9, 7)), Vec::new());
        state.stations[0].part_inventory = 0;

        let next = tick(&state, true);

        let count = next
            .pending_helper_tasks
            .iter()
            .filter(|t| **t == HelperTask::RestockStation { station_index: 0 })
            .count()
            + next
                .robots
                .iter()
                .filter(|r| r.helper_assignment == Some(HelperTask::RestockStation { station_index: 0 }))
                .count();
        assert_eq!(count, 0, "헬퍼가 한 대도 없으면 요청만 큐에 쌓이고 아무도 배정받지 않는다");
        assert_eq!(next.pending_helper_tasks.len(), 1, "요청은 정확히 한 번만 큐에 들어가야 한다(중복 방지)");

        let after_another_tick = tick(&next, true);
        assert_eq!(
            after_another_tick.pending_helper_tasks.len(),
            1,
            "재고가 여전히 0이어도 이미 큐에 있는 요청을 또 추가하면 안 된다"
        );
    }

    #[test]
    fn an_idle_helper_gets_assigned_the_oldest_pending_request() {
        let mut state = SimState::new(Arc::new(Grid::new(9, 7)), vec![Robot::new(1, WAREHOUSE_CELL, WAREHOUSE_CELL)]);
        state.stations[0].part_inventory = 0;

        let next = tick(&state, true);

        assert_eq!(next.robots[0].helper_assignment, Some(HelperTask::RestockStation { station_index: 0 }));
    }

    #[test]
    fn helper_restocks_a_station_end_to_end() {
        let mut state = SimState::new(Arc::new(Grid::new(9, 7)), vec![Robot::new(1, WAREHOUSE_CELL, WAREHOUSE_CELL)]);
        state.stations[0].part_inventory = 0;

        let mut restocked = false;
        for _ in 0..500 {
            state = tick(&state, true);
            if state.stations[0].part_inventory == STATION_MAX_INVENTORY {
                restocked = true;
                break;
            }
        }
        assert!(restocked, "헬퍼가 결국 스테이션 재고를 채워야 한다");
        assert_eq!(state.robots[0].helper_assignment, None, "임무를 마치면 배정이 풀려야 한다");
        assert!(!state.robots[0].carrying);
    }

    #[test]
    fn helper_delivers_a_fresh_frame_when_the_line_start_is_empty() {
        let state = SimState::new(Arc::new(Grid::new(9, 7)), vec![Robot::new(1, WAREHOUSE_CELL, WAREHOUSE_CELL)]);
        assert!(state.products.is_empty());

        let mut state = state;
        let mut delivered = false;
        for _ in 0..500 {
            state = tick(&state, true);
            if state.products.iter().any(|p| p.pos == (BELT_START_X, BELT_ROW) && p.stage == 0) {
                delivered = true;
                break;
            }
        }
        assert!(delivered, "라인 시작점이 비어있으면 헬퍼가 결국 새 프레임을 가져다 놓아야 한다");
    }

    #[test]
    fn assembly_robots_are_never_assigned_helper_tasks() {
        let mut robot = Robot::new(1, station_robot_cell(0), station_robot_cell(0));
        robot.role = RobotRole::Assembly { station_index: 0 };
        let mut state = SimState::new(Arc::new(Grid::new(9, 7)), vec![robot]);
        state.stations[0].part_inventory = 0;

        let next = tick(&state, true);

        assert_eq!(next.robots[0].helper_assignment, None);
    }
```

- [ ] **Step 5: 제품 무충돌 결정성 proptest**

`server/tests/tick_properties.rs`에 추가(파일 끝, 기존 `proptest! { ... }` 블록 안에):

```rust
    /// 여러 제품이 벨트 위에 있어도, 한 틱 뒤에 서로 다른 칸에 있어야
    /// 한다(설계문서 §7) — 로봇의 `tick_never_produces_collisions`와
    /// 같은 불변식을 제품에도 적용.
    #[test]
    fn products_never_occupy_the_same_cell(start_xs in proptest::sample::subsequence((sim_core::sim::BELT_START_X..sim_core::sim::BELT_END_X).collect::<Vec<i32>>(), 4), conveyor_running: bool) {
        use sim_core::sim::{Product, SimState, BELT_ROW};
        let products: Vec<Product> = start_xs.into_iter().enumerate().map(|(i, x)| Product::new(i as u32, (x, BELT_ROW))).collect();
        let mut state = SimState::new(Arc::new(Grid::new(SIZE, SIZE)), Vec::new());
        state.products = products;

        let next = tick(&state, conveyor_running);

        let mut seen = HashSet::new();
        for product in &next.products {
            prop_assert!(seen.insert(product.pos), "duplicate product position after tick: {:?}", product.pos);
        }
    }
```

파일 상단 `use` 목록에 `sim_core::sim::tick`은 이미 있으니 추가 import는 함수 안에서 지역적으로 처리했다(위 코드처럼 함수 내부 `use`).

- [ ] **Step 6: 테스트 실행**

Run: `cargo test --all` (server 디렉터리)
Expected: 전부 통과.

- [ ] **Step 7: 커밋**

```bash
git add server/src/sim.rs server/tests/tick_properties.rs
git commit -m "feat: helper robot task queue, assignment, and pickup/dropoff logistics"
```

---

### Task 4: `game_state.rs` — 로봇 스폰/카운트 의미 변경

**Files:**
- Modify: `server/src/game_state.rs`

- [ ] **Step 1: 스폰 로직 교체 — 조립 로봇 3대 자동 생성 + 헬퍼만 조절**

`game_state.rs`의 `GameState::new`를 교체:

```rust
impl GameState {
    pub fn new(sim: SimState) -> Self {
        let next_robot_id = sim.robots.iter().map(|r| r.id).max().map_or(0, |max| max + 1);
        let mut state = GameState { sim, conveyor: Conveyor::new(), selected_robot: None, next_robot_id };
        state.ensure_assembly_robots_exist();
        state
    }

    /// 조립 로봇이 하나도 없으면(=새 게임 시작) 스테이션 수만큼(3대) 자동
    /// 생성한다. 이미 있으면(예: 향후 영속화된 상태를 복원하는 경우) 다시
    /// 만들지 않는다 — 멱등. 사용자가 조절할 수 없다(설계문서 §4) —
    /// `set_robot_count`는 헬퍼만 다룬다.
    fn ensure_assembly_robots_exist(&mut self) {
        let has_assembly = self.sim.robots.iter().any(|r| matches!(r.role, RobotRole::Assembly { .. }));
        if has_assembly {
            return;
        }
        for station in self.sim.stations.clone() {
            let id = self.next_robot_id;
            self.next_robot_id += 1;
            let mut robot = Robot::new(id, station.robot_cell, station.robot_cell);
            robot.role = RobotRole::Assembly { station_index: station.index };
            self.sim.robots.push(robot);
        }
    }
}
```

파일 상단 `use` 구문에 `RobotRole` 추가:

```rust
use sim_core::sim::{Robot, RobotRole, RobotStatus, SimState, Task, REPAIR_TICKS};
```

- [ ] **Step 2: `set_robot_count`를 헬퍼 전용 + 하한 1로 교체**

`set_robot_count` 함수를 교체:

```rust
    /// 헬퍼 로봇 대수를 정확히 `target`대로 맞춘다(조립 로봇 3대는
    /// 여기서 건드리지 않는다 — 설계문서 §4). 하한 1을 강제한다(설계문서
    /// §6) — 0명이 되면 재고가 바닥난 스테이션을 영영 못 채워 라인
    /// 전체가 회복 불가능하게 멈추기 때문. 상한은 기존 `MAX_ROBOT_COUNT`
    /// 그대로(조립 로봇 3대를 더한 총 로봇 수가 아니라 헬퍼 수 자체에
    /// 적용). 몇 대를 추가/제거할지 시작 시점에 한 번만 계산해 두므로
    /// 반복마다 `filter().count()`를 다시 돌지 않는다.
    pub fn set_robot_count(&mut self, target: usize) {
        let target = target.clamp(1, MAX_ROBOT_COUNT);
        let current = self.sim.robots.iter().filter(|r| r.role == RobotRole::Helper).count();

        if current < target {
            for _ in 0..(target - current) {
                let id = self.next_robot_id;
                self.next_robot_id += 1;
                self.sim.robots.push(Robot::new(id, sim_core::sim::WAREHOUSE_CELL, sim_core::sim::WAREHOUSE_CELL));
            }
        } else {
            for _ in 0..(current - target) {
                if let Some((index, _)) = self
                    .sim
                    .robots
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.role == RobotRole::Helper)
                    .max_by_key(|(_, r)| r.id)
                {
                    self.sim.robots.remove(index);
                }
            }
        }

        if let Some(selected) = self.selected_robot {
            if !self.sim.robots.iter().any(|r| r.id == selected) {
                self.selected_robot = None;
            }
        }
    }
```

- [ ] **Step 3: 기존 테스트 갱신 — 헬퍼 전용 의미 + 조립 로봇 3대 반영**

`game_state.rs`의 `#[cfg(test)] mod tests` 블록에서 다음을 고친다:

`set_robot_count_grows_and_shrinks`, `set_robot_count_clamps_to_max`, `set_robot_count_assigns_unique_growing_ids`, `set_robot_count_removes_highest_id_even_out_of_vec_order`, `select_robot_rejects_unknown_id`, `select_then_release_clears_selection`, `removing_selected_robot_clears_selection`, `trigger_arm_action_sets_task_on_the_right_robot`, `trigger_arm_action_rejects_non_operational_robot`, `repair_robot_transitions_a_failed_robot_to_repairing`, `repair_robot_rejects_a_non_failed_robot`, `repair_all_failed_robots_repairs_only_the_failed_ones_and_counts_them`, `repair_all_failed_robots_is_a_harmless_no_op_when_nothing_is_failed`, `select_robot_works_on_a_failed_robot`, `set_robot_count_shrink_removes_highest_id_even_if_it_is_repairing` — 이 테스트들은 전부 `state.sim.robots[0]`처럼 **인덱스로** 특정 로봇을 가리키는데, 이제 `empty_state()`에서 `GameState::new`가 조립 로봇 3대를 자동으로 추가하므로 인덱스가 다 밀린다. 각 테스트에서 `state.sim.robots[N]`을 헬퍼만 골라내는 방식으로 바꾼다. 공통 헬퍼를 테스트 모듈 상단에 추가:

```rust
    fn helper_robots(state: &GameState) -> Vec<&Robot> {
        state.sim.robots.iter().filter(|r| r.role == RobotRole::Helper).collect()
    }
```

그리고 각 테스트에서 `state.sim.robots.len()`을 쓰던 곳은 `helper_robots(&state).len()`으로, `state.sim.robots[0].id` 같은 인덱스 접근은 `helper_robots(&state)[0].id`로 바꾼다. 예시(`set_robot_count_grows_and_shrinks`):

```rust
    #[test]
    fn set_robot_count_grows_and_shrinks() {
        let mut state = empty_state();
        state.set_robot_count(3);
        assert_eq!(helper_robots(&state).len(), 3);
        state.set_robot_count(1);
        assert_eq!(helper_robots(&state).len(), 1);
    }
```

`set_robot_count_clamps_to_max`:

```rust
    #[test]
    fn set_robot_count_clamps_to_max() {
        let mut state = empty_state();
        state.set_robot_count(usize::MAX);
        assert_eq!(helper_robots(&state).len(), MAX_ROBOT_COUNT);
    }
```

새 테스트 추가(하한 1 검증, 설계문서 §6):

```rust
    #[test]
    fn set_robot_count_never_goes_below_one_helper() {
        let mut state = empty_state();
        state.set_robot_count(5);
        state.set_robot_count(0);
        assert_eq!(helper_robots(&state).len(), 1, "헬퍼는 최소 1명이어야 한다");
    }

    #[test]
    fn game_state_new_always_creates_exactly_station_count_assembly_robots() {
        let state = empty_state();
        let assembly_count = state
            .sim
            .robots
            .iter()
            .filter(|r| matches!(r.role, RobotRole::Assembly { .. }))
            .count();
        assert_eq!(assembly_count, sim_core::sim::STATION_COUNT);
    }

    #[test]
    fn set_robot_count_never_removes_an_assembly_robot() {
        let mut state = empty_state();
        state.set_robot_count(0);
        let assembly_count = state
            .sim
            .robots
            .iter()
            .filter(|r| matches!(r.role, RobotRole::Assembly { .. }))
            .count();
        assert_eq!(assembly_count, sim_core::sim::STATION_COUNT, "set_robot_count(0)이어도 조립 로봇은 그대로 남아야 한다");
    }
```

나머지 인덱스 기반 테스트(`trigger_arm_action_sets_task_on_the_right_robot` 등)도 같은 패턴(`helper_robots(&state)[N].id`)으로 고친다 — 구체적으로 각 테스트에서 `state.sim.robots[k]`를 `helper_robots(&state)[k]`로 치환.

- [ ] **Step 4: 빌드 + 테스트**

Run: `cargo build --all-targets && cargo test --all` (server 디렉터리)
Expected: 통과. 다른 크레이트(`protocol.rs`, `main.rs`)에서 `state.sim.robots.len()`을 가정하던 테스트가 있으면 이 시점에 컴파일은 되지만 값이 달라져 실패할 수 있다 — Task 5/6에서 처리(그 파일들을 아직 안 건드렸으므로 지금은 실패해도 정상, 다음 태스크에서 고친다). 만약 지금 이 태스크 범위(`game_state.rs`) 안에서만 실패가 난다면 지금 고친다.

- [ ] **Step 5: 커밋**

```bash
git add server/src/game_state.rs
git commit -m "feat: auto-spawn fixed assembly robots, SetRobotCount now governs helper count only"
```

---

### Task 5: 프로토콜 — `RobotRole`/`StationView`/`ProductView`

**Files:**
- Modify: `server/src/protocol.rs`
- Modify: `server/src/delta.rs`

- [ ] **Step 1: 와이어 타입 추가**

`protocol.rs`의 `WireArmPose` 다음에 추가:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind")]
pub enum WireRobotRole {
    Assembly { station_index: u8 },
    Helper,
}

impl From<sim_core::sim::RobotRole> for WireRobotRole {
    fn from(r: sim_core::sim::RobotRole) -> WireRobotRole {
        match r {
            sim_core::sim::RobotRole::Assembly { station_index } => WireRobotRole::Assembly { station_index },
            sim_core::sim::RobotRole::Helper => WireRobotRole::Helper,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct StationView {
    pub index: u8,
    pub robot_cell: WireCellId,
    pub part_inventory: u32,
}

impl From<&sim_core::sim::Station> for StationView {
    fn from(s: &sim_core::sim::Station) -> StationView {
        StationView { index: s.index, robot_cell: s.robot_cell.into(), part_inventory: s.part_inventory }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductView {
    pub id: u32,
    pub stage: u8,
    pub pos: WireCellId,
}

impl From<&sim_core::sim::Product> for ProductView {
    fn from(p: &sim_core::sim::Product) -> ProductView {
        ProductView { id: p.id, stage: p.stage, pos: p.pos.into() }
    }
}
```

`RobotView` 구조체에 필드 추가(`carrying` 다음):

```rust
    pub carrying: bool,
    pub role: WireRobotRole,
```

`impl From<&Robot> for RobotView`의 본문에 필드 추가:

```rust
            carrying: r.carrying,
            role: r.role.into(),
```

- [ ] **Step 2: `Snapshot`/`Delta`에 스테이션/제품 실어보내기**

`ServerMessage` enum(현재 218~224번 줄)을 교체:

```rust
pub enum ServerMessage {
    Snapshot {
        v: u8,
        tick: u64,
        session_id: uuid::Uuid,
        conveyor: ConveyorView,
        robots: Vec<RobotView>,
        stations: Vec<StationView>,
        products: Vec<ProductView>,
    },
    Delta {
        v: u8,
        tick: u64,
        conveyor: Option<ConveyorView>,
        changed_robots: Vec<RobotView>,
        removed_robot_ids: Vec<u32>,
        stations: Vec<StationView>,
        changed_products: Vec<ProductView>,
        removed_product_ids: Vec<u32>,
    },
    ResumeAck { v: u8, session_id: uuid::Uuid, resumed: bool },
}
```

(`stations`는 항상 3개뿐이라 매번 풀 목록을 싣는다 — 설계문서 §8. `products`/`changed_products`/`removed_product_ids`는 로봇과 같은 델타 패턴.)

`to_snapshot` 함수를 교체:

```rust
pub fn to_snapshot(state: &GameState, session_id: uuid::Uuid) -> ServerMessage {
    ServerMessage::Snapshot {
        v: PROTOCOL_VERSION,
        tick: state.sim.tick_count,
        session_id,
        conveyor: state.conveyor.into(),
        robots: state.sim.robots.iter().map(RobotView::from).collect(),
        stations: state.sim.stations.iter().map(StationView::from).collect(),
        products: state.sim.products.iter().map(ProductView::from).collect(),
    }
}
```

- [ ] **Step 3: `delta.rs`의 `compute_delta` 확장**

`delta.rs`의 `compute_delta` 함수 시그니처와 본문을 교체:

```rust
pub fn compute_delta(
    previous_conveyor: ConveyorView,
    previous_robots: &[RobotView],
    previous_products: &[crate::protocol::ProductView],
    current_tick: u64,
    current_conveyor: ConveyorView,
    current_robots: &[RobotView],
    current_stations: &[crate::protocol::StationView],
    current_products: &[crate::protocol::ProductView],
) -> ServerMessage {
    let conveyor = if previous_conveyor == current_conveyor { None } else { Some(current_conveyor) };

    let changed_robots: Vec<RobotView> = current_robots
        .iter()
        .filter(|current| !previous_robots.iter().any(|prev| prev == *current))
        .cloned()
        .collect();

    let removed_robot_ids: Vec<u32> = previous_robots
        .iter()
        .filter(|prev| !current_robots.iter().any(|current| current.id == prev.id))
        .map(|prev| prev.id)
        .collect();

    let changed_products: Vec<crate::protocol::ProductView> = current_products
        .iter()
        .filter(|current| !previous_products.iter().any(|prev| prev == *current))
        .cloned()
        .collect();

    let removed_product_ids: Vec<u32> = previous_products
        .iter()
        .filter(|prev| !current_products.iter().any(|current| current.id == prev.id))
        .map(|prev| prev.id)
        .collect();

    ServerMessage::Delta {
        v: PROTOCOL_VERSION,
        tick: current_tick,
        conveyor,
        changed_robots,
        removed_robot_ids,
        stations: current_stations.to_vec(),
        changed_products,
        removed_product_ids,
    }
}
```

파일 상단 import에 `ProductView`/`StationView` 추가:

```rust
use crate::protocol::{ConveyorView, ProductView, RobotView, ServerMessage, StationView, PROTOCOL_VERSION};
```

- [ ] **Step 4: 기존 테스트 갱신**

`protocol.rs`의 `server_message_round_trips_through_json`, `to_snapshot_reflects_current_game_state` 테스트에 새 필드(빈 `Vec`/실제 값)를 채운다:

```rust
    #[test]
    fn server_message_round_trips_through_json() {
        let msg = ServerMessage::Snapshot {
            v: 1,
            tick: 42,
            session_id: uuid::Uuid::nil(),
            conveyor: ConveyorView { running: true },
            robots: vec![],
            stations: vec![],
            products: vec![],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }
```

```rust
    #[test]
    fn to_snapshot_reflects_current_game_state() {
        use crate::game_state::GameState;
        use sim_core::grid::Grid;
        use sim_core::sim::SimState;
        use std::sync::Arc;

        let mut sim = SimState::new(Arc::new(Grid::new(3, 3)), Vec::new());
        sim.tick_count = 5;
        let mut state = GameState::new(sim);
        state.set_robot_count(2);
        state.toggle_conveyor();

        let snapshot = to_snapshot(&state, uuid::Uuid::nil());
        match snapshot {
            ServerMessage::Snapshot { v, tick, conveyor, robots, stations, .. } => {
                assert_eq!(v, PROTOCOL_VERSION);
                assert_eq!(tick, 5);
                assert!(!conveyor.running);
                assert_eq!(robots.len(), 2 + sim_core::sim::STATION_COUNT, "헬퍼 2대 + 조립 로봇 3대");
                assert_eq!(stations.len(), sim_core::sim::STATION_COUNT);
            }
            _ => panic!("expected Snapshot"),
        }
    }
```

`delta.rs`의 모든 `compute_delta(...)` 호출부(테스트 전부)에 새 파라미터(`&[]`, `&[]`)를 채운다. 예시(`unchanged_robots_are_omitted_from_delta`):

```rust
    #[test]
    fn unchanged_robots_are_omitted_from_delta() {
        let prev = vec![robot_view(1, 0)];
        let curr = vec![robot_view(1, 0)];

        let msg = compute_delta(ConveyorView { running: true }, &prev, &[], 1, ConveyorView { running: true }, &curr, &[], &[]);

        match msg {
            ServerMessage::Delta { conveyor, changed_robots, removed_robot_ids, .. } => {
                assert!(conveyor.is_none());
                assert!(changed_robots.is_empty());
                assert!(removed_robot_ids.is_empty());
            }
            _ => panic!("expected Delta"),
        }
    }
```

(나머지 `compute_delta` 호출부도 동일하게 `&[]` 두 개씩 끼워넣는다 — `moved_robot_is_included_in_delta`, `removed_robot_id_is_reported`, `new_robot_is_included_in_delta`, `conveyor_change_is_reported_only_when_it_changed`.)

새 테스트 추가(제품 델타 검증):

```rust
    fn product_view(id: u32, x: i32, stage: u8) -> crate::protocol::ProductView {
        crate::protocol::ProductView { id, stage, pos: WireCellId { x, y: 0 } }
    }

    #[test]
    fn changed_product_is_included_and_unchanged_is_omitted() {
        let prev_products = vec![product_view(1, 0, 0), product_view(2, 5, 1)];
        let curr_products = vec![product_view(1, 1, 0), product_view(2, 5, 1)];

        let msg = compute_delta(
            ConveyorView { running: true }, &[], &prev_products, 1,
            ConveyorView { running: true }, &[], &[], &curr_products,
        );

        match msg {
            ServerMessage::Delta { changed_products, removed_product_ids, .. } => {
                assert_eq!(changed_products, vec![product_view(1, 1, 0)]);
                assert!(removed_product_ids.is_empty());
            }
            _ => panic!("expected Delta"),
        }
    }

    #[test]
    fn removed_product_id_is_reported() {
        let prev_products = vec![product_view(1, 0, 0)];

        let msg = compute_delta(
            ConveyorView { running: true }, &[], &prev_products, 1,
            ConveyorView { running: true }, &[], &[], &[],
        );

        match msg {
            ServerMessage::Delta { removed_product_ids, .. } => assert_eq!(removed_product_ids, vec![1]),
            _ => panic!("expected Delta"),
        }
    }
```

- [ ] **Step 5: 빌드 + 테스트**

Run: `cargo build --all-targets && cargo test --all` (server 디렉터리)
Expected: `protocol.rs`/`delta.rs` 관련 전부 통과. `main.rs`가 아직 `compute_delta`를 옛 시그니처로 부르는 곳에서 컴파일 에러가 날 것 — Task 6에서 고친다(지금은 이 태스크 범위 밖).

- [ ] **Step 6: 커밋**

```bash
git add server/src/protocol.rs server/src/delta.rs
git commit -m "feat: add RobotRole/StationView/ProductView to the wire protocol"
```

---

### Task 6: `main.rs` 배선 — 생산량 집계, 초기 그리드, 틱 루프

**Files:**
- Modify: `server/src/main.rs`
- Delete: `server/src/production.rs`
- Modify: `server/src/lib.rs` 또는 해당 크레이트 루트(모듈 선언에서 `production` 제거) — `production.rs`가 실제로 어느 크레이트에 있는지 Step 1에서 확인.

- [ ] **Step 1: `production.rs`가 죽은 코드가 되는 이유 확인 후 제거**

`production.rs::total_production(robots, units_per_robot)`은 "로봇 ID별로 생산량을 매핑해 결정적 순서로 합산"하는 함수였다 — 부동소수점 합산 순서가 실행마다 달라지지 않게 하기 위한 장치였다. 새 설계에서 생산량은 "이번 틱에 완성된 제품 수 × `UNIT_PER_CYCLE`"이라는 단순 곱셈이라 애초에 합산 순서 문제가 생기지 않는다 — 이 함수를 계속 쓰는 건 이제 안 쓰는 추상화를 억지로 통과시키는 것뿐이다(중복/불필요 코드 방지 원칙). `sim_core`(어느 크레이트인지 `grep -rn "mod production" .`으로 확인, 보통 `server/src/lib.rs` 또는 `sim_core`의 루트) 선언에서 `pub mod production;`을 제거하고 `server/src/production.rs` 파일을 삭제한다.

Run: `grep -rn "mod production\|production::" server/src` 로 전체 참조를 확인하고, `main.rs`의 `use sim_core::production::total_production;`(19번 줄)도 이 태스크 Step 3에서 제거한다.

- [ ] **Step 2: `initial_state()` 갱신**

`main.rs`의 `initial_state()`(현재 77~88번 줄)를 교체:

```rust
fn initial_state() -> SharedState {
    // 9x7 — client/src/main.ts::GRID_SIZE와 정확히 같은 크기(변경 없음).
    // 로봇은 더 이상 여기서 스폰하지 않는다 — GameState::new가 조립
    // 로봇 3대를 자동으로 만들고, 헬퍼는 SetRobotCount로 조절한다.
    let sim = SimState::new(Arc::new(Grid::new(9, 7)), Vec::new());
    Arc::new(Mutex::new(GameState::new(sim)))
}
```

- [ ] **Step 3: `detect_completed_placements` → `detect_completed_assemblies`, 생산량 계산 단순화**

`detect_completed_placements` 함수(현재 146~159번 줄)를 삭제하고 그 자리에 추가:

```rust
/// 이전 틱과 이번 틱의 제품 id 목록을 비교해, 이번 틱에 반출(완성)된
/// 제품 수를 센다 — 벨트 끝에 도달한 제품은 `sim_core::sim::plan_products`가
/// 조용히 목록에서 제거하므로(설계문서 §5-3), "이전엔 있었는데 이번엔
/// 없어진 id"가 곧 완성 이벤트다. `detect_status_transitions`와 같은
/// 이유(실제 틱 타이밍 없이 결정적으로 단위테스트하기 위함)로 순수 함수로
/// 분리했다.
fn detect_completed_assemblies(previous_products: &[protocol::ProductView], current_products: &[protocol::ProductView]) -> usize {
    previous_products
        .iter()
        .filter(|prev| !current_products.iter().any(|current| current.id == prev.id))
        .count()
}
```

`spawn_tick_loop` 함수 안의 델타/생산량 계산 블록(현재 211~225번 줄)을 교체:

```rust
                let current_snapshot = to_snapshot(&guard, uuid::Uuid::nil());
                let (delta, failure_events, total_production_value) = match (&last_snapshot, &current_snapshot) {
                    (
                        protocol::ServerMessage::Snapshot { conveyor: prev_conveyor, robots: prev_robots, products: prev_products, .. },
                        protocol::ServerMessage::Snapshot {
                            tick: cur_tick, conveyor: cur_conveyor, robots: cur_robots, stations: cur_stations, products: cur_products, ..
                        },
                    ) => {
                        let delta = compute_delta(
                            *prev_conveyor, prev_robots, prev_products, *cur_tick, *cur_conveyor, cur_robots, cur_stations, cur_products,
                        );
                        let events = detect_status_transitions(prev_robots, cur_robots, *cur_tick);
                        let completed = detect_completed_assemblies(prev_products, cur_products);
                        let production = completed as f32 * sim_core::sim::UNIT_PER_CYCLE;
                        (delta, events, production)
                    }
                    _ => (current_snapshot.clone(), Vec::new(), 0.0_f32),
                };
```

파일 상단 `use` 목록에서 `use sim_core::production::total_production;`(19번 줄)을 삭제하고, `use std::collections::HashMap;`(21번 줄)도 더 이상 이 파일에서 안 쓰면 삭제(다른 곳에서 `HashMap`을 쓰는지 `grep -n "HashMap" server/src/main.rs`로 확인 후 결정).

- [ ] **Step 4: 기존 테스트 갱신**

`main.rs`의 `sample_robot_view` 헬퍼(현재 396~410번 줄)에 새 필드 추가:

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
            path: Vec::new(),
            facing: protocol::WireDirection::East,
            arm_pose: protocol::WireArmPose { shoulder_angle: 0.0, elbow_angle: 0.0 },
            carrying: false,
            role: protocol::WireRobotRole::Helper,
        }
    }
```

`detect_a_new_failure` 등 `detect_status_transitions` 테스트들은 변경 없이 그대로 통과해야 한다(그 함수 자체는 안 바뀜).

`detect_completed_placements`를 검증하던 테스트가 있다면(이 함수 이름으로 `grep -n "detect_completed_placements" server/src/main.rs` 확인) 지우고 아래로 교체:

```rust
    fn sample_product_view(id: u32) -> protocol::ProductView {
        protocol::ProductView { id, stage: 3, pos: protocol::WireCellId { x: 7, y: 3 } }
    }

    #[test]
    fn detect_completed_assemblies_counts_products_that_disappeared() {
        let previous = vec![sample_product_view(1), sample_product_view(2)];
        let current = vec![sample_product_view(1)];

        assert_eq!(detect_completed_assemblies(&previous, &current), 1);
    }

    #[test]
    fn detect_completed_assemblies_is_zero_when_nothing_disappeared() {
        let previous = vec![sample_product_view(1)];
        let current = vec![sample_product_view(1)];

        assert_eq!(detect_completed_assemblies(&previous, &current), 0);
    }

    #[test]
    fn detect_completed_assemblies_ignores_brand_new_products() {
        let previous: Vec<protocol::ProductView> = vec![];
        let current = vec![sample_product_view(5)];

        assert_eq!(detect_completed_assemblies(&previous, &current), 0);
    }
```

- [ ] **Step 5: 빌드 + 테스트**

Run: `cargo build --all-targets && cargo test --all` (server 디렉터리)
Expected: 통과.

- [ ] **Step 6: 커밋**

```bash
git rm server/src/production.rs
git add server/src/main.rs
git commit -m "feat: wire product completion into production accounting, drop the now-dead production module"
```

---

### Task 7: 클라이언트 프로토콜/상태 — `RobotRole`/`StationView`/`ProductView`

**Files:**
- Modify: `client/src/net/protocol.ts`
- Modify: `client/src/state/mirror.ts`
- Modify: `client/src/state/interpolation.ts`

- [ ] **Step 1: 와이어 타입**

`client/src/net/protocol.ts`에 추가(파일 끝, `ClientCommand` 위):

```ts
export type WireRobotRole = { kind: 'Assembly'; station_index: number } | { kind: 'Helper' }

export interface StationView {
  index: number
  robot_cell: WireCellId
  part_inventory: number
}

export interface ProductView {
  id: number
  stage: number
  pos: WireCellId
}
```

`RobotView` 인터페이스에 필드 추가:

```ts
export interface RobotView {
  // ...기존 필드 그대로...
  carrying: boolean
  role: WireRobotRole
}
```

`ServerMessage` 타입을 교체:

```ts
export type ServerMessage =
  | {
      kind: 'Snapshot'
      v: number
      tick: number
      session_id: string
      conveyor: ConveyorView
      robots: RobotView[]
      stations: StationView[]
      products: ProductView[]
    }
  | {
      kind: 'Delta'
      v: number
      tick: number
      conveyor: ConveyorView | null
      changed_robots: RobotView[]
      removed_robot_ids: number[]
      stations: StationView[]
      changed_products: ProductView[]
      removed_product_ids: number[]
    }
  | { kind: 'ResumeAck'; v: number; session_id: string; resumed: boolean }
```

- [ ] **Step 2: `MirrorState` 확장**

`client/src/state/mirror.ts`를 교체:

```ts
import type { ConveyorView, ProductView, RobotView, ServerMessage, StationView } from '../net/protocol'

export interface MirrorState {
  conveyor: ConveyorView
  robots: Map<number, RobotView>
  stations: StationView[]
  products: Map<number, ProductView>
}

export function createEmptyMirror(): MirrorState {
  return { conveyor: { running: false }, robots: new Map(), stations: [], products: new Map() }
}

export function applyServerMessage(mirror: MirrorState, message: ServerMessage): MirrorState {
  switch (message.kind) {
    case 'Snapshot':
      return {
        conveyor: message.conveyor,
        robots: new Map(message.robots.map((r) => [r.id, r])),
        stations: message.stations,
        products: new Map(message.products.map((p) => [p.id, p])),
      }
    case 'Delta': {
      const robots = new Map(mirror.robots)
      for (const robot of message.changed_robots) {
        robots.set(robot.id, robot)
      }
      for (const id of message.removed_robot_ids) {
        robots.delete(id)
      }
      const products = new Map(mirror.products)
      for (const product of message.changed_products) {
        products.set(product.id, product)
      }
      for (const id of message.removed_product_ids) {
        products.delete(id)
      }
      return {
        conveyor: message.conveyor ?? mirror.conveyor,
        robots,
        stations: message.stations.length > 0 ? message.stations : mirror.stations,
        products,
      }
    }
    case 'ResumeAck':
      return mirror
  }
}
```

(`stations`는 서버가 매 델타마다 항상 풀 목록을 보내므로(`Task 5`) 보통 `message.stations.length > 0`이 항상 참이지만, 빈 그리드 등 극단적 케이스를 위한 방어적 폴백.)

- [ ] **Step 3: 제품 보간(interpolation) 추가**

`client/src/state/interpolation.ts`에 추가(파일 끝):

```ts
export interface InterpolatedProduct extends ProductView {
  renderPos: { x: number; y: number }
}

/** `computeRenderRobots`와 완전히 같은 보간 규칙 — 제품도 서버 틱 사이를
 * 매끄럽게 이동하는 것처럼 보이려면 같은 처리가 필요하다. */
export function computeRenderProducts(prev: TickSnapshot | null, curr: TickSnapshot, nowMs: number): InterpolatedProduct[] {
  const factor = computeRenderFactor(nowMs - curr.receivedAtMs)
  const result: InterpolatedProduct[] = []

  for (const product of curr.mirror.products.values()) {
    const prevProduct = prev?.mirror.products.get(product.id)
    if (!prevProduct) {
      result.push({ ...product, renderPos: { x: product.pos.x, y: product.pos.y } })
      continue
    }
    result.push({
      ...product,
      renderPos: {
        x: lerp(prevProduct.pos.x, product.pos.x, factor),
        y: lerp(prevProduct.pos.y, product.pos.y, factor),
      },
    })
  }
  return result
}
```

파일 상단 import에 `ProductView` 추가:

```ts
import type { ProductView, RobotView } from '../net/protocol'
```

- [ ] **Step 4: 기존 테스트 갱신**

`client/tests/unit/protocol.test.ts`, `mirror.test.ts`, `canvas.test.ts`, `sidebar.test.ts`(각 파일에서 `RobotView` 픽스처를 만드는 곳)에 `carrying: false, role: { kind: 'Helper' }` 필드를 추가한다 — 정확한 파일 위치는 `grep -rln "carrying: false" client/tests` 로 확인해 전부 갱신.

`mirror.test.ts`에 새 테스트 추가:

```ts
import { createEmptyMirror, applyServerMessage } from '../../src/state/mirror'

test('snapshot populates stations and products', () => {
  const mirror = applyServerMessage(createEmptyMirror(), {
    kind: 'Snapshot',
    v: 1,
    tick: 0,
    session_id: 'x',
    conveyor: { running: true },
    robots: [],
    stations: [{ index: 0, robot_cell: { x: 2, y: 2 }, part_inventory: 5 }],
    products: [{ id: 1, stage: 0, pos: { x: 1, y: 3 } }],
  })

  expect(mirror.stations).toHaveLength(1)
  expect(mirror.products.get(1)?.stage).toBe(0)
})

test('delta removes a product by id', () => {
  let mirror = applyServerMessage(createEmptyMirror(), {
    kind: 'Snapshot',
    v: 1,
    tick: 0,
    session_id: 'x',
    conveyor: { running: true },
    robots: [],
    stations: [],
    products: [{ id: 1, stage: 0, pos: { x: 1, y: 3 } }],
  })

  mirror = applyServerMessage(mirror, {
    kind: 'Delta',
    v: 1,
    tick: 1,
    conveyor: null,
    changed_robots: [],
    removed_robot_ids: [],
    stations: [],
    changed_products: [],
    removed_product_ids: [1],
  })

  expect(mirror.products.has(1)).toBe(false)
})
```

(테스트 러너/assertion 스타일은 기존 `mirror.test.ts` 파일을 열어 실제 쓰는 프레임워크(vitest `test`/`expect` 등)를 그대로 따를 것 — 위 코드는 그 스타일을 가정.)

- [ ] **Step 5: 테스트 실행**

Run: `npm test` (client 디렉터리, 또는 `package.json`의 실제 테스트 스크립트 이름 확인 후 실행)
Expected: 통과.

- [ ] **Step 6: 커밋**

```bash
git add client/src/net/protocol.ts client/src/state/mirror.ts client/src/state/interpolation.ts client/tests
git commit -m "feat: client protocol/state support for RobotRole, stations, and products"
```

---

### Task 8: 클라이언트 렌더링 — 일자 벨트, 제품 스프라이트, 재고 경고

**Files:**
- Modify: `client/src/render/canvas.ts`

- [ ] **Step 1: U자 벨트 판정을 일자 벨트로 교체**

`isConveyorCell`/`conveyorFlowDirection`(현재 11~35번 줄)을 교체:

```ts
import { BELT_ROW, BELT_START_X, BELT_END_X, STATION_XS, STATION_ROBOT_ROW, WAREHOUSE_ROWS } from './layout'

/** 일자형 벨트가 차지하는 칸(설계문서 §1) — 서버 `sim_core::sim`의
 * `BELT_ROW`/`BELT_START_X`/`BELT_END_X`와 정확히 같은 정의를 여기 다시
 * 써서 유지한다(서버는 자기 좌표만 보내지 이 상수들을 와이어로 보내지
 * 않는다 — 안 바뀌는 레이아웃 상수라 매 메시지에 실을 이유가 없다는
 * 판단, 기존 U자 벨트 때와 같은 트레이드오프). 지난번 U자 벨트에서 이
 * 정의가 서버/클라이언트 사이에 어긋나(그리드 크기 불일치) 실제 버그가
 * 났으므로, 이 값이 서버 쪽과 일치하는지 검증하는 테스트를 반드시 둔다
 * (Task 10). */
export function isConveyorCell(_grid: GridSize, x: number, y: number): boolean {
  return y === BELT_ROW && x >= BELT_START_X && x <= BELT_END_X
}

export function isWarehouseCell(_grid: GridSize, x: number, y: number): boolean {
  return WAREHOUSE_ROWS.includes(y)
}

/** 벨트는 이제 항상 오른쪽(+x)으로만 흐른다 — U자 순환이 없어졌으니
 * 방향 계산도 훨씬 단순해졌다. */
export function conveyorFlowDirection(grid: GridSize, x: number, y: number): { dx: number; dy: number } | null {
  if (!isConveyorCell(grid, x, y)) {
    return null
  }
  return { dx: 1, dy: 0 }
}
```

`client/src/render/layout.ts`(새 파일, 서버 `sim.rs`의 레이아웃 상수를 그대로 미러링):

```ts
// server/src/sim.rs의 레이아웃 상수를 그대로 미러링한다(BELT_ROW 등).
// 값 자체가 와이어로 오지 않는 이유는 canvas.ts의 isConveyorCell 주석 참고.
export const BELT_ROW = 3
export const BELT_START_X = 1
export const BELT_END_X = 7
export const STATION_XS = [2, 4, 6]
export const STATION_ROBOT_ROW = 2
export const WAREHOUSE_ROWS = [0, 1]
```

- [ ] **Step 2: 창고 구역 타일 색 추가**

`drawTile` 함수(현재 88~123번 줄)의 시그니처와 `drawFloor` 호출부를 교체 — 벨트/창고/일반 바닥 세 가지 색을 구분해야 한다:

```ts
function drawFloor(ctx: CanvasRenderingContext2D, grid: GridSize, conveyor: ConveyorView, animationTimeMs: number): void {
  for (let y = 0; y < grid.height; y++) {
    for (let x = 0; x < grid.width; x++) {
      const screen = gridToScreen(x, y)
      const direction = conveyorFlowDirection(grid, x, y)
      const warehouse = isWarehouseCell(grid, x, y)
      drawTile(ctx, screen.x, screen.y, direction, warehouse, conveyor.running, animationTimeMs)
    }
  }
}

function drawTile(
  ctx: CanvasRenderingContext2D,
  sx: number,
  sy: number,
  direction: { dx: number; dy: number } | null,
  warehouse: boolean,
  running: boolean,
  animationTimeMs: number,
): void {
  const isBelt = direction !== null
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
  } else if (warehouse) {
    gradient.addColorStop(0, '#8f5fc9')
    gradient.addColorStop(1, '#5f3d8f')
  } else {
    gradient.addColorStop(0, '#4a9d6f')
    gradient.addColorStop(1, '#2c6b47')
  }
  ctx.fillStyle = gradient
  ctx.fill()
  ctx.strokeStyle = 'rgba(0,0,0,0.3)'
  ctx.stroke()

  if (direction) {
    drawConveyorChevrons(ctx, direction, running, animationTimeMs)
  }
  ctx.restore()
}
```

`drawConveyorChevrons`는 변경 없음(이미 임의 `direction`을 받으므로 오른쪽 고정 방향에도 그대로 동작).

- [ ] **Step 3: 제품 스프라이트 + 스테이션 재고 경고**

`DrawSceneInput` 인터페이스(현재 51~58번 줄)에 필드 추가:

```ts
export interface DrawSceneInput {
  grid: GridSize
  conveyor: ConveyorView
  robots: InterpolatedRobot[]
  products: InterpolatedProduct[]
  stations: StationView[]
  showPaths: boolean
  animationTimeMs: number
  selectedRobotId: number | null
}
```

import 목록에 추가:

```ts
import type { InterpolatedProduct } from '../state/interpolation'
import type { StationView } from '../net/protocol'
```

`drawScene` 함수(현재 60~76번 줄)에 제품/스테이션 그리기 호출 추가:

```ts
export function drawScene(ctx: CanvasRenderingContext2D, canvasWidth: number, canvasHeight: number, input: DrawSceneInput): void {
  ctx.clearRect(0, 0, canvasWidth, canvasHeight)
  ctx.save()
  ctx.translate(canvasWidth / 2, 40)
  ctx.scale(RENDER_SCALE, RENDER_SCALE)

  drawFloor(ctx, input.grid, input.conveyor, input.animationTimeMs)

  for (const product of input.products) {
    drawProduct(ctx, product)
  }

  for (const station of input.stations) {
    drawStationInventoryWarning(ctx, station)
  }

  for (const robot of sortRobotsForDrawing(input.robots)) {
    if (input.showPaths) {
      drawPath(ctx, robot)
    }
    drawRobot(ctx, robot, robot.id === input.selectedRobotId)
  }

  ctx.restore()
}
```

새 함수 추가(`drawRobot` 함수 앞에):

```ts
/** 제품(드론) 단계별 스프라이트 — 브레인스토밍 목업(product-progression.html)의
 * SVG 모양을 캔버스 그리기로 그대로 옮긴 것. stage가 오를수록 프레임 위에
 * 부품이 하나씩 늘어난다. */
function drawProduct(ctx: CanvasRenderingContext2D, product: InterpolatedProduct): void {
  const screen = gridToScreen(product.renderPos.x, product.renderPos.y)
  ctx.save()
  ctx.translate(screen.x, screen.y - 6) // 바닥보다 살짝 띄워서 로봇 팔 높이와 겹치지 않게

  // 프레임(X자 팔) — 0단계부터 항상 보인다.
  ctx.strokeStyle = '#565e68'
  ctx.lineWidth = 3
  ctx.lineCap = 'round'
  ctx.beginPath()
  ctx.moveTo(-7, -4)
  ctx.lineTo(7, 4)
  ctx.moveTo(7, -4)
  ctx.lineTo(-7, 4)
  ctx.stroke()

  // 배터리(stage >= 1)
  if (product.stage >= 1) {
    ctx.fillStyle = '#c9762f'
    ctx.strokeStyle = '#1c2024'
    ctx.lineWidth = 1
    ctx.fillRect(-4, -3, 8, 6)
    ctx.strokeRect(-4, -3, 8, 6)
  }

  // 프로펠러 4개(stage >= 2)
  if (product.stage >= 2) {
    ctx.strokeStyle = '#8b95a0'
    ctx.lineWidth = 1.5
    for (const [cx, cy] of [
      [-7, -4],
      [7, -4],
      [-7, 4],
      [7, 4],
    ] as const) {
      ctx.beginPath()
      ctx.arc(cx, cy, 3, 0, Math.PI * 2)
      ctx.stroke()
    }
  }

  // 완성 체크(stage >= 3)
  if (product.stage >= 3) {
    ctx.strokeStyle = '#ffd23a'
    ctx.lineWidth = 1.5
    ctx.beginPath()
    ctx.arc(0, 0, 6, 0, Math.PI * 2)
    ctx.stroke()
  }

  ctx.restore()
}

/** 재고 0인 스테이션은 조립 로봇 위에 경고색 표시(기존 고장 표시와 같은
 * 관례 — 빨강 경고 삼각형). 재고가 있으면 아무것도 그리지 않는다. */
function drawStationInventoryWarning(ctx: CanvasRenderingContext2D, station: StationView): void {
  if (station.part_inventory > 0) {
    return
  }
  const screen = gridToScreen(station.robot_cell.x, station.robot_cell.y)
  ctx.save()
  ctx.translate(screen.x, screen.y - 34)
  ctx.fillStyle = '#e04b3f'
  ctx.beginPath()
  ctx.moveTo(0, -6)
  ctx.lineTo(5, 4)
  ctx.lineTo(-5, 4)
  ctx.closePath()
  ctx.fill()
  ctx.strokeStyle = '#1c2024'
  ctx.lineWidth = 1
  ctx.stroke()
  ctx.restore()
}
```

- [ ] **Step 4: 테스트 갱신**

`client/tests/unit/canvas.test.ts`에서 `isConveyorCell`/`conveyorFlowDirection`을 U자 기준으로 검증하던 테스트를 찾아(`grep -n "isConveyorCell\|conveyorFlowDirection" client/tests/unit/canvas.test.ts`) 일자 벨트 기준으로 교체:

```ts
test('isConveyorCell is true only along the straight belt row within its span', () => {
  const grid = { width: 9, height: 7 }
  expect(isConveyorCell(grid, 1, 3)).toBe(true)
  expect(isConveyorCell(grid, 7, 3)).toBe(true)
  expect(isConveyorCell(grid, 0, 3)).toBe(false)
  expect(isConveyorCell(grid, 8, 3)).toBe(false)
  expect(isConveyorCell(grid, 4, 2)).toBe(false)
})

test('conveyorFlowDirection always points right on the belt', () => {
  const grid = { width: 9, height: 7 }
  expect(conveyorFlowDirection(grid, 4, 3)).toEqual({ dx: 1, dy: 0 })
  expect(conveyorFlowDirection(grid, 4, 2)).toBeNull()
})

test('isWarehouseCell is true for the top two rows', () => {
  const grid = { width: 9, height: 7 }
  expect(isWarehouseCell(grid, 3, 0)).toBe(true)
  expect(isWarehouseCell(grid, 3, 1)).toBe(true)
  expect(isWarehouseCell(grid, 3, 2)).toBe(false)
})
```

기존 `drawScene`을 직접 호출하는 테스트가 있으면 `products`/`stations` 필드를 `DrawSceneInput`에 채운다(`grep -n "drawScene(" client/tests/unit/canvas.test.ts`로 위치 확인).

- [ ] **Step 5: 테스트 실행**

Run: `npm test` (client 디렉터리)
Expected: 통과.

- [ ] **Step 6: 커밋**

```bash
git add client/src/render/canvas.ts client/src/render/layout.ts client/tests
git commit -m "feat: render the straight belt, staged product sprites, and station inventory warnings"
```

---

### Task 9: 사이드바 라벨 + `main.ts` 배선

**Files:**
- Modify: `client/src/ui/sidebar.ts`
- Modify: `client/src/main.ts`

- [ ] **Step 1: 사이드바 라벨/하한 변경**

`sidebar.ts`에서 로봇 수 조절 버튼 근처(현재 54~64번 줄)에 라벨 추가:

```ts
    const countLabel = document.createElement('span')
    countLabel.textContent = '헬퍼 로봇 수 '
    globalSection.appendChild(countLabel)

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
```

`SidebarState.robotCount`의 의미는 이제 "헬퍼 로봇 수"다 — 타입 자체는 안 바뀌지만 `main.ts`가 채우는 값이 바뀐다(Step 2).

- [ ] **Step 2: `main.ts` — 헬퍼 카운트 하한 1, 제품/스테이션 배선**

`onChangeRobotCount` 콜백(현재 71~74번 줄)을 교체 — 이제 "몇 대의 헬퍼가 있는지"를 `mirror.robots`에서 role로 걸러 세야 한다:

```ts
    onChangeRobotCount: (delta) => {
      const currentHelperCount = [...mirror.robots.values()].filter((r) => r.role.kind === 'Helper').length
      const nextCount = Math.max(1, currentHelperCount + delta)
      connection.send({ type: 'SetRobotCount', count: nextCount })
    },
```

`renderSidebar` 함수(현재 99~107번 줄)에서 `robotCount` 계산을 같은 방식으로 교체:

```ts
  function renderSidebar(): void {
    const helperCount = [...mirror.robots.values()].filter((r) => r.role.kind === 'Helper').length
    sidebar.update({
      connection: connectionStatus,
      conveyor: mirror.conveyor,
      robotCount: helperCount,
      selectedRobot: selectedRobotId !== null ? (mirror.robots.get(selectedRobotId) ?? null) : null,
      pathDebugEnabled,
    })
  }
```

`frame()` 함수(현재 157~170번 줄)에서 `drawScene` 호출에 제품/스테이션 전달:

```ts
  function frame(): void {
    const now = performance.now()
    const rendered = currSnapshot ? computeRenderRobots(prevSnapshot, currSnapshot, now) : []
    const renderedProducts = currSnapshot ? computeRenderProducts(prevSnapshot, currSnapshot, now) : []
    drawScene(ctx, canvas.width, canvas.height, {
      grid: GRID_SIZE,
      conveyor: mirror.conveyor,
      robots: rendered,
      products: renderedProducts,
      stations: mirror.stations,
      showPaths: pathDebugEnabled,
      animationTimeMs: now,
      selectedRobotId,
    })
    requestAnimationFrame(frame)
  }
```

import 목록(현재 5~6번 줄)에 `computeRenderProducts` 추가:

```ts
import { computeRenderRobots, computeRenderProducts } from './state/interpolation'
```

- [ ] **Step 2b: `GRID_SIZE` 주석 갱신**

`main.ts`의 `GRID_SIZE` 위 주석(현재 14~15번 줄, "U자 컨베이어가...")을 현재 사실에 맞게 고친다:

```ts
// 그리드 크기 — server/src/main.rs::initial_state()와 정확히 일치해야 한다.
const GRID_SIZE = { width: 9, height: 7 }
```

- [ ] **Step 3: 사이드바 테스트 갱신**

`client/tests/unit/sidebar.test.ts`에서 "로봇 수" 라벨/버튼을 검증하는 테스트가 있으면 "헬퍼 로봇 수" 텍스트를 확인하도록 갱신(`grep -n "로봇 수\|robotCount" client/tests/unit/sidebar.test.ts`로 위치 확인 후 갱신).

- [ ] **Step 4: 테스트 실행**

Run: `npm test` (client 디렉터리)
Expected: 통과.

- [ ] **Step 5: 커밋**

```bash
git add client/src/ui/sidebar.ts client/src/main.ts client/tests
git commit -m "feat: relabel sidebar count control to helpers, wire products/stations into the render loop"
```

---

### Task 10: 통합테스트 갱신 + 문서/DB 초기화 안내

**Files:**
- Modify: `server/tests/ws_integration.rs`
- Modify: `server/tests/rest_integration.rs`
- Modify: `README.md`
- Modify: `docs/KANBAN.md`

- [ ] **Step 1: `ws_integration.rs`의 초기 로봇 수 가정 수정**

`connects_and_receives_initial_snapshot_then_reacts_to_commands` 테스트(현재 46~90번 줄 근처)의 첫 단언을 고친다 — 이제 서버는 항상 조립 로봇 3대로 시작한다:

```rust
    let json: Value = serde_json::from_str(&text).expect("initial message should be valid JSON");
    assert_eq!(json["kind"], "Snapshot");
    assert_eq!(
        json["robots"].as_array().expect("robots should be an array").len(),
        3,
        "서버는 항상 조립 로봇 3대로 시작해야 한다"
    );
```

`SetRobotCount{count:2}`를 보낸 뒤 "로봇 2대가 보인다"를 확인하던 부분을 "총 5대(조립 3 + 헬퍼 2)"로 고친다:

```rust
    let saw_five_robots = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let Some(Ok(Message::Text(text))) = read.next().await else { return false };
            let Ok(json) = serde_json::from_str::<Value>(&text) else { continue };
            if json["kind"] == "Delta" {
                if let Some(changed) = json["changed_robots"].as_array() {
                    // 델타는 "바뀐" 로봇만 담으므로 누적 카운트가 아니라, 이번
                    // 메시지에 헬퍼 2대가 새로 나타났는지를 직접 확인한다.
                    let helper_count = changed.iter().filter(|r| r["role"]["kind"] == "Helper").count();
                    if helper_count == 2 {
                        return true;
                    }
                }
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(saw_five_robots, "expected a delta message reflecting 2 new helper robots after SetRobotCount");
```

**주의**: 이 파일의 나머지 부분(219, 252, 289번 줄 근처의 다른 `SetRobotCount` 사용)도 실제로 열어서(`Read` 도구로) 어떤 시나리오를 검증하는지 확인하고, "총 로봇 수가 N이어야 한다"는 가정이 있으면 "헬퍼 N + 조립 3"으로 고친다 — 정확한 문맥은 구현 시점에 파일을 직접 읽고 판단할 것(이 계획 작성 시점엔 해당 세 지점의 전체 테스트 본문을 다 확인하지 못했다).

- [ ] **Step 2: `rest_integration.rs`의 생산량 테스트를 새 모델로 재작성**

`production_only_increases_after_a_robot_completes_a_full_work_cycle` 테스트(184번 줄 근처, `ROBOT_COUNT`/`PICK_TICKS`/`PLACE_TICKS` 기반 상한 계산 포함)는 옛 자유이동 사이클 전제라 그대로 못 쓴다. 이 테스트가 실제로 무엇을 검증했는지(REST `/api/stats/history`로 생산량이 실제 작업 완료에 연동되는지) 같은 취지를 새 모델로 재작성한다 — 조립 로봇 3대(항상 존재)와 헬퍼 2~3대가 실제로 드론 한 대를 완성시키는 데 몇 틱 정도 걸리는지 상한을 계산해서 검증:

```rust
#[tokio::test]
async fn production_increases_once_a_full_drone_completes_the_line() {
    let server = spawn_server();
    let url = format!("ws://127.0.0.1:{}/ws", server.port);
    let (ws_stream, _) = tokio_tungstenite::connect_async(url).await.expect("failed to connect");
    let (mut write, mut read) = ws_stream.split();
    let _first = read.next().await.expect("stream ended early").expect("ws error");

    write
        .send(Message::Text(serde_json::json!({ "type": "SetRobotCount", "count": 3 }).to_string()))
        .await
        .expect("failed to send SetRobotCount");

    // 헬퍼가 첫 프레임을 배달하고, 세 스테이션을 각각 조립 카운트다운
    // 만큼 거쳐 완성되기까지 넉넉히 기다린다 — 정확한 틱 수 대신
    // "이 정도면 충분히 여러 사이클이 돌고도 남는다"는 넉넉한 벽시계
    // 타임아웃을 쓴다(기존 REST 통합테스트들과 같은 패턴).
    tokio::time::sleep(Duration::from_secs(8)).await;

    let http_url = format!("http://127.0.0.1:{}/api/stats/history", server.port);
    let body = reqwest::get(&http_url).await.expect("failed to GET stats history").text().await.expect("failed to read body");
    let rows: serde_json::Value = serde_json::from_str(&body).expect("stats history should be valid JSON");
    let rows = rows.as_array().expect("expected an array of stats rows");

    let max_production = rows
        .iter()
        .filter_map(|row| row["total_production"].as_f64())
        .fold(0.0_f64, f64::max);

    assert!(max_production > 0.0, "적어도 한 대의 드론은 완성되어 생산량이 0보다 커야 한다");
}
```

(`reqwest`가 이미 `Cargo.toml`의 dev-dependencies에 있는지 `grep -n "reqwest" server/Cargo.toml`로 확인 — 기존 `rest_integration.rs`가 REST 엔드포인트를 이미 호출하고 있었으므로 이미 있을 가능성이 높다. 없으면 기존 파일이 실제로 어떤 HTTP 클라이언트를 쓰는지 확인해 그것을 그대로 쓴다 — 이 계획 작성 시점엔 그 파일의 HTTP 클라이언트 선택을 직접 확인하지 못했다.)

기존 `ROBOT_COUNT`/`max_cycles_per_robot` 관련 죽은 코드(184~189번 줄 근처)는 위 재작성과 함께 삭제한다.

- [ ] **Step 3: 전체 서버 테스트 실행**

Run: `cargo test --all` (server 디렉터리, 통합테스트 포함)
Expected: 전부 통과. 실패하면 Task 1~9에서 놓친 컴파일/의미 변경이 있다는 뜻이니 실패 메시지를 따라 해당 태스크로 돌아가 고친다.

- [ ] **Step 4: 클라이언트 전체 테스트 + E2E**

Run: `npm test` 그리고 (있다면) `npm run test:e2e` (client 디렉터리, 정확한 스크립트 이름은 `client/package.json` 확인)
Expected: 전부 통과.

- [ ] **Step 5: README/KANBAN 갱신**

`docs/KANBAN.md`에 이 기능 완료를 Done 섹션에 추가(기존 컨벤션대로 커밋 SHA 포함 — 마지막 커밋 후 SHA를 채운다). `README.md`의 게임 설명 부분(로봇이 뭘 하는지 설명하는 절)을 조립 라인 구조로 갱신 — 정확한 위치는 현재 README를 열어 확인.

**DB 초기화 리마인더**: 이 기능을 실제로 배포/로컬 실행하기 전에 기존 `gamerobotfactory.sqlite3` 파일을 지운다(설계문서 §9) — 옛 스키마 가정(예: 로봇이 자유 이동하며 쌓은 통계)과 새 구조가 맞지 않는다. `docker-compose.yml`을 쓴다면 볼륨(`gamerobotfactory-data`)도 함께 지워야 한다: `docker compose down -v` 후 `docker compose up --build -d`.

- [ ] **Step 6: 최종 커밋**

```bash
git add server/tests/ws_integration.rs server/tests/rest_integration.rs README.md docs/KANBAN.md
git commit -m "test: update integration tests for the assembly-line model; docs: update README/KANBAN"
```
