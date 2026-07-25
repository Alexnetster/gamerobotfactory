use crate::grid::{CellId, Grid};
use crate::pathfind::find_path;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const LEG_CYCLE_SPEED: f32 = 0.1;
// 헬퍼가 경로를 다시 계산하기 전 기존 경로를 유지하는 최소 틱 수 —
// 매 틱 재계산하면 낭비이므로(길찾기가 A*라 상대적으로 비쌈), 어느 정도
// 오래된 점유 스냅샷 기반 경로도 허용한다(헬퍼는 순찰보다 목적지가
// 명확해 짧은 지연은 문제되지 않음).
const REPATH_INTERVAL: u32 = 10;
// 1000초(약 17분) 분량의 작업(20Hz 기준) — 튜닝 대상. 2000(100초) →
// 6000(5분)으로 한 번 완화했으나, 실사용 피드백("수리 후에도 금방 다시
// 고장난다")으로 다시 낮춤: wear_ratio가 0에서 다시 쌓여야 하는 건
// 맞지만(수리하면 worn_ticks가 0으로 리셋됨), 그 램프업 자체가 몇 분
// 안에 다시 위험 수위에 닿을 만큼 가팔랐다. 이제 로봇 한 대가 완전히
// 마모되려면 픽업+배치 사이클(40틱)을 500번 반복해야 한다 — 짧은
// 데모 세션 안에서는 "가끔 일어나는 특별한 사건"에 가깝게, 수리
// 직후에는 사실상 안전하게 느껴지도록.
pub const WEAR_LIMIT_TICKS: u64 = 20000;
// 완전 마모 상태에서의 틱당 최대 고장 확률 — 튜닝 대상. 위와 같은 이유로
// 0.05(5%) → 0.02(2%)를 거쳐 다시 낮춤.
const MAX_FAILURE_PROB: f64 = 0.01;
pub const REPAIR_TICKS: u32 = 100; // 20Hz 기준 5초 — 튜닝 대상. 나중 태스크의 game_state.rs::repair_robot이 RepairRobot 처리 시 이 값을 참조할 예정이라 pub.
pub const PICK_TICKS: u32 = 20; // 20Hz 기준 약 1초 — 튜닝 대상
pub const PLACE_TICKS: u32 = 20; // 20Hz 기준 약 1초 — 튜닝 대상
pub const UNIT_PER_CYCLE: f32 = 1.0; // 배치 1회 완료당 생산량 — main.rs가 참조

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

/// 새로 스폰되는 헬퍼가 유휴 상태로 서 있을 수 있는 칸들(창고 구역
/// y=0..=1 안, `WAREHOUSE_CELL` 자체와는 다른 칸). 실제 픽업/드롭 거래는
/// 여전히 `WAREHOUSE_CELL` 하나만 쓴다 — 이 배열은 오직 "새로 만들어진
/// 헬퍼가 처음에 어디 서 있는가"만 바꾼다.
///
/// 왜 필요한가: `plan_helper`는 `helper_assignment == None`인(아직 아무
/// 작업도 배정 못 받은) 헬퍼를 완전히 가만히 둔다(절대 이동하지 않음,
/// 아래 참고). `game_state.rs::set_robot_count`로 헬퍼가 여러 대 추가돼도
/// 동시에 대기 중인 작업은 보통 하나뿐이라(스테이션 `STATION_COUNT`개 +
/// `DeliverFrame` 하나), 헬퍼 수가 대기 작업 수보다 많아지면 나머지는
/// 영구히 유휴 상태로 남는다. 예전엔 이 유휴 헬퍼들을 전부
/// `WAREHOUSE_CELL` 자체에 스폰했는데, 그 칸은 활성 헬퍼가 실제 픽업을
/// 하러 반드시 도달해야 하는 칸이다 — `find_path`는 목표 칸이 점유돼
/// 있어도 그쪽으로 향하는 경로 자체는 허용하지만(pathfind.rs 참고, "그
/// 로봇이 다음 틱에 비킬 수도 있으므로"), `advance_along_path`의 마지막
/// 한 칸 진입은 여전히 `!occupied.contains(&next_cell)`로 막힌다 — 유휴
/// 헬퍼가 `WAREHOUSE_CELL`을 영구 점거하면 그 마지막 한 칸이 영원히
/// 열리지 않아 활성 헬퍼가 도착 직전에서 영원히 멈추고, 그 헬퍼가 물고
/// 있던 작업이 끝나지 않으니 라인 전체가 멈춘다(실측된 배포 정지 버그,
/// `6576653` 커밋을 bisect해서 확인). 스폰 위치만 이 배열로 분산시키면
/// 유휴 헬퍼끼리는 서로 몇 명이 겹쳐도(트랜잭션 칸이 아니므로) 문제가
/// 없다.
///
/// `set_robot_count`가 새로 배정하는 로봇 id를 이 배열 길이로 나눈
/// 나머지로 인덱싱해서 스폰 칸을 고른다 — id는 항상 증가만 하므로(재사용
/// 없음, `set_robot_count_assigns_unique_growing_ids` 테스트가 이미
/// 보장) 한 번의 `set_robot_count` 호출로 여러 대가 한꺼번에 스폰돼도
/// 이 배열 길이만큼은 서로 겹치지 않는다. 그 이상(예: 배열 길이보다 많은
/// 헬퍼가 한 번에 스폰되거나, `MAX_ROBOT_COUNT`에 가까운 극단적으로 많은
/// 헬퍼 수) 겹치는 건 감수한다 — 겹쳐도 문제되는 칸은 여전히
/// `WAREHOUSE_CELL` 자체뿐이고, 그 칸은 이 배열에 없으므로 안전하다.
pub const HELPER_SPAWN_STAGING_CELLS: [CellId; 6] = [(0, 0), (2, 0), (6, 0), (8, 0), (1, 1), (7, 1)];
pub const STATION_MAX_INVENTORY: u32 = 5;
pub const ASSEMBLY_TICKS: u32 = 20; // 조립 로봇의 스테이션당 작업 시간 — 튜닝 대상
pub const HELPER_PICKUP_TICKS: u32 = 20; // 헬퍼가 창고에서 집어드는 시간 — 튜닝 대상
pub const HELPER_DROP_TICKS: u32 = 20; // 헬퍼가 목적지에 내려놓는 시간 — 튜닝 대상

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

/// 로봇이 지금 수행 중인 팔 작업. `TriggerArmAction` 커맨드가 이 값을
/// 바꾼다 — 실제 IK/애니메이션 계산은 클라이언트/렌더러(Plan 4)의 몫이고,
/// 여기서는 "지금 무슨 작업 중인가"라는 사실만 기록한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    Idle,
    Picking,
    Placing,
}

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

#[derive(Debug, Clone)]
pub struct Robot {
    pub id: u32,
    pub pos: CellId,
    pub goal: CellId,
    pub path: Vec<CellId>,
    pub ticks_until_repath: u32,
    pub leg_cycle_progress: f32,
    pub task: Task,
    pub worn_ticks: u64,
    pub status: RobotStatus,
    pub facing: Direction,
    pub carrying: bool,
    pub work_ticks_remaining: u32,
    pub role: RobotRole,
    pub helper_assignment: HelperAssignment,
}

impl Robot {
    pub fn new(id: u32, pos: CellId, goal: CellId) -> Self {
        Robot {
            id,
            pos,
            goal,
            path: Vec::new(),
            ticks_until_repath: 0,
            leg_cycle_progress: 0.0,
            task: Task::Idle,
            worn_ticks: 0,
            status: RobotStatus::Operational,
            facing: Direction::East,
            carrying: false,
            work_ticks_remaining: 0,
            role: RobotRole::Helper,
            helper_assignment: None,
        }
    }

    /// 0.0(방금 교체됨) ~ 1.0(완전 마모)의 마모 비율. 고장 확률 계산과
    /// (나중 태스크에서 배선될) 프로토콜의 `durability_remaining` 노출이
    /// 이 함수 하나만 쓸 예정이다 — 계산식을 두 곳에 복사해두면
    /// `WEAR_LIMIT_TICKS`를 나중에 튜닝할 때 한쪽만 고치고 잊어버리는
    /// 드리프트가 생기기 쉽다.
    pub fn wear_ratio(&self) -> f32 {
        (self.worn_ticks as f32 / WEAR_LIMIT_TICKS as f32).min(1.0)
    }
}

/// (robot_id, tick_count)를 섞어 대략 [0.0, 1.0] 구간의 결정적 의사난수를
/// 낸다(u64 -> f64 변환의 부동소수점 반올림으로 극히 드물게 정확히 1.0이
/// 나올 수 있음 — `failure_prob`가 최대 `MAX_FAILURE_PROB`를 넘지
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

fn station_robot_cell(station_index: u8) -> CellId {
    (STATION_XS[station_index as usize], STATION_ROBOT_ROW)
}

#[derive(Debug, Clone)]
pub struct SimState {
    pub grid: Arc<Grid>,
    pub robots: Vec<Robot>,
    pub products: Vec<Product>,
    pub stations: Vec<Station>,
    pub pending_helper_tasks: Vec<HelperTask>,
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
            pending_helper_tasks: Vec::new(),
            tick_count: 0,
        }
    }
}

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

/// 시뮬레이션을 정확히 한 틱 전진시킨다. 순수 함수 — `state`를 변경하지
/// 않고 새 상태를 반환한다. 각 로봇의 계획은 "틱 시작 시점에 얼어붙은
/// 스냅샷"(`occupied`)만 읽으므로(더블 버퍼링), 병렬로 계산해도 서로의
/// 계산 중인 결과를 참조하지 않아 데이터 경쟁이 없다.
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

    let (new_products, new_stations) = if conveyor_running {
        plan_products(&state.products, &state.stations)
    } else {
        (state.products.clone(), state.stations.clone())
    };

    let (new_robots, new_stations, new_products, new_pending_helper_tasks) =
        run_helper_logistics(new_robots, new_stations, new_products, state.pending_helper_tasks.clone(), conveyor_running);

    SimState {
        grid: state.grid.clone(),
        robots: new_robots,
        products: new_products,
        stations: new_stations,
        pending_helper_tasks: new_pending_helper_tasks,
        tick_count: state.tick_count + 1,
    }
}

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
            if !conveyor_running {
                return next;
            }
            plan_helper(grid, next, occupied, tick_count)
        }
    }
}

/// 헬퍼 로봇의 창고<->목적지 왕복(설계문서 §6). `helper_assignment`가
/// `None`이면 아무 것도 하지 않는다(작업 배정은 `tick()`이 로봇 목록
/// 전체를 보고 매 틱 결정하므로 이 함수 진입 전에 이미 채워져 있다고
/// 가정) — 배정된 작업이 있을 때의 이동/카운트다운만 여기서 처리한다.
fn plan_helper(grid: &Grid, mut next: Robot, occupied: &HashSet<CellId>, tick_count: u64) -> Robot {
    let Some(task) = next.helper_assignment else {
        return next;
    };

    let destination = match task {
        HelperTask::RestockStation { station_index } => station_robot_cell(station_index),
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
        next = advance_along_path(grid, next, occupied, tick_count);
        if next.pos != target {
            return next; // 아직 도착 전 — 다음 틱에 계속 이동
        }
        // 이번 틱에 막 도착했다 — 곧바로 아래에서 카운트다운을 시작한다.
        // "도착"과 "카운트다운 시작" 사이에 아무 일도 안 하는 틱을 끼우면
        // (즉 여기서 그냥 return next 했다면) work_ticks_remaining이 여전히
        // 0인 채로 pos == target인 상태가 한 틱 존재하게 되는데,
        // run_helper_logistics의 드롭 완료 판정(carrying &&
        // work_ticks_remaining == 0 && pos == destination)이 "막 도착함"과
        // "카운트다운이 다 끝남"을 구분하지 못해 그 틱에 곧바로 드롭을
        // 완료시켜 버린다 — HELPER_DROP_TICKS 카운트다운 전체가 건너뛰어짐
        // (실측: mutation test로 재현 — 도착까지 걸린 이동 틱 수만으로
        // 완료되고 HELPER_DROP_TICKS만큼 더 걸리지 않는 것을 확인함).
    }

    next.work_ticks_remaining = if next.carrying { HELPER_DROP_TICKS } else { HELPER_PICKUP_TICKS };
    if !next.carrying {
        next.carrying = true; // 창고 도착 -> 픽업 카운트다운 시작(들었다고 가정, 드롭 시 실제 효과 적용은 tick()에서)
    }
    next
}

/// 헬퍼가 목표(`next.goal`)를 향해 경로를 따라 한 칸 전진한다. 순찰
/// 전용이던 `PATROL_MOVE_INTERVAL_TICKS` 지연은 여기서 넣지 않는다 —
/// 헬퍼는 매 틱 이동해도 무방하다(의도적 차이, 오래된 순찰 로직의
/// 흔적이 아님). `tick_count` 파라미터는 인터페이스 일관성을 위해
/// 남겨두되 이 함수 안에서는 쓰이지 않는다.
fn advance_along_path(grid: &Grid, mut next: Robot, occupied: &HashSet<CellId>, _tick_count: u64) -> Robot {
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

/// `plan_robot`을 패닉으로부터 격리한다. 패닉이 나면 해당 로봇은 이번
/// 틱을 그대로 멈춘 채 넘어가고, 나머지 로봇들의 갱신은 영향받지 않는다.
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

/// Runs `f` (a robot's per-tick update) isolated from panics: if it
/// unwinds, the robot holds its last position instead of taking down
/// the whole tick. Depends on the crate never setting `panic = "abort"`
/// in a Cargo profile — under `panic = "abort"` this becomes a no-op
/// and a single robot's fault would abort the whole process instead of
/// being isolated, with no compile-time warning. `AssertUnwindSafe` is
/// currently a no-op assertion (nothing reachable here has interior
/// mutability yet) but must be revisited if that changes.
fn safe_call(robot: &Robot, f: impl FnOnce() -> Robot) -> Robot {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| {
            eprintln!("robot {} update panicked; holding position this tick", robot.id);
            robot.clone()
        })
}

/// 같은 틱에 여러 로봇이 같은 칸으로 이동을 계획하면, `robot_id`가 가장
/// 낮은 로봇이 이기고 나머지는 원래 칸으로 되돌린다 — 실행 순서나 스레드
/// 스케줄링과 무관하게 항상 같은 결과가 나오는 결정적 타이브레이크.
///
/// 참고: 이 함수 자체는 같은 칸(vertex) 충돌만 잡아낸다. 하지만 두 로봇이
/// 서로의 칸을 맞바꾸려는 경우(A: X→Y, B: Y→X)는 애초에 이 함수까지
/// 오지 않는다 — `plan_robot`이 이동 전 "틱 시작 시점 점유 스냅샷"
/// (`occupied`, 자기 자신 포함)을 기준으로 다음 칸이 비어 있는지 확인
/// 하므로, A는 B가 아직 X에 있는 Y로 이동을 시도하지 않고 그 자리에
/// 머문다(B도 마찬가지) — 결과적으로 서로 통과하지 않고 둘 다 제자리에
/// 멈춘다. 이는 설계 문서에 명시된 범위(1칸 예약만 처리, 시간축까지
/// 포함한 완전한 예약 탐색은 하지 않음)보다 오히려 더 안전한 결과이며,
/// `resolve_intents`가 별도로 edge/swap 충돌을 처리할 필요가 없는 이유다.
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

    // (2) 노는 헬퍼에게 배정 — 먼저 발생한 요청(큐 맨 앞)부터. 배정 순서는
    // `robots` Vec 순서가 아니라 robot_id 오름차순으로 명시적으로 고정한다
    // (`resolve_intents`가 `intent.mover_id`로 명시적 타이브레이크하는 것과
    // 같은 이유) — 오늘은 `game_state::set_robot_count`가 항상 커지는 id를
    // 끝에 append하고 줄어들 때도 최댓값 id를 제거해서 Vec 순서가 id
    // 오름차순과 우연히 일치하지만, 그 불변식은 이 함수 밖에 있고 여기서
    // 재확인할 방법이 없다 — Vec 순서에 그냥 기대면 나중에 그 불변식이
    // 깨지는 순간(혹은 이 파일의 테스트처럼 `robots.push`로 순서를 직접
    // 뒤섞는 호출부) 배정이 비결정적으로 보이게 된다.
    let mut idle_helper_indices: Vec<usize> = robots
        .iter()
        .enumerate()
        .filter(|(_, r)| r.role == RobotRole::Helper && r.helper_assignment.is_none())
        .map(|(index, _)| index)
        .collect();
    idle_helper_indices.sort_by_key(|&index| robots[index].id);
    for index in idle_helper_indices {
        if pending.is_empty() {
            break;
        }
        robots[index].helper_assignment = Some(pending.remove(0));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_state(width: i32, height: i32) -> SimState {
        SimState::new(Arc::new(Grid::new(width, height)), Vec::new())
    }

    #[test]
    fn safe_call_recovers_from_a_real_panic_and_holds_position() {
        let robot = Robot::new(1, (0, 0), (2, 0));

        let result = safe_call(&robot, || panic!("simulated fault in robot update"));

        assert_eq!(result.pos, robot.pos);
    }

    #[test]
    fn new_robot_starts_idle() {
        let robot = Robot::new(1, (0, 0), (0, 0));
        assert_eq!(robot.task, Task::Idle);
    }

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
        // failure_prob==MAX_FAILURE_PROB로 고정된다 — 여러 tick_count에
        // 대해 update_status를 반복 호출해 실제로 그 비율 근처로 고장이
        // 발생하는지 통계적으로 확인한다(정확히 일치할 필요는 없고
        // 자릿수만 맞으면 됨 — 결정적 해시라 매번 같은 결과). 기대 범위를
        // `MAX_FAILURE_PROB`에서 직접 계산해서, 이 상수를 나중에 다시
        // 튜닝해도(실제로 한 번 0.05->0.02로 바뀐 적 있음) 이 테스트가
        // 낡은 하드코딩 값 때문에 깨지지 않게 한다.
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
        let expected_range = (MAX_FAILURE_PROB * 0.5)..(MAX_FAILURE_PROB * 1.5);
        assert!(expected_range.contains(&rate), "expected a failure rate near {MAX_FAILURE_PROB}, got {rate}");
    }

    #[test]
    fn failed_robot_does_not_move_even_toward_an_unreached_goal() {
        let mut state = simple_state(5, 1);
        let mut robot = Robot::new(1, (0, 0), (3, 0));
        robot.status = RobotStatus::Failed;
        state.robots.push(robot);

        let next = tick(&state, false);

        assert_eq!(next.robots[0].pos, (0, 0), "a Failed robot must not move");
    }

    #[test]
    fn failed_robot_permanently_blocks_the_cell_for_other_robots() {
        // A single-tick version of this test can't distinguish "blocked
        // because Failed" from the pre-existing one-tick lookahead collision
        // rule (any stationary robot, Failed or not, blocks the cell for
        // exactly one tick). Running enough ticks that an Operational
        // blocker would provably have vacated by then (as
        // `robot_moves_one_step_toward_goal_each_tick` proves it would, on
        // tick 1) is what actually proves the Failed-freeze is in effect.
        let mut blocker = Robot::new(1, (1, 0), (2, 0)); // would eventually vacate toward (2,0) if operational
        blocker.status = RobotStatus::Failed;
        let mover = Robot::new(2, (0, 0), (2, 0));
        let mut state = simple_state(3, 1);
        state.robots.push(blocker);
        state.robots.push(mover);

        for _ in 0..10 {
            state = tick(&state, false);
            let blocker_after = state.robots.iter().find(|r| r.id == 1).unwrap();
            let mover_after = state.robots.iter().find(|r| r.id == 2).unwrap();
            assert_eq!(blocker_after.pos, (1, 0), "a Failed robot must never move, even toward its own unreached goal");
            assert_eq!(mover_after.pos, (0, 0), "the mover can never advance into a cell permanently occupied by a Failed robot");
        }
    }

    #[test]
    fn repairing_robot_counts_down_and_returns_to_operational() {
        let mut state = simple_state(3, 1);
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        robot.status = RobotStatus::Repairing { remaining_ticks: 2 };
        robot.worn_ticks = 500;
        state.robots.push(robot);

        let after_one = tick(&state, false);
        assert_eq!(after_one.robots[0].status, RobotStatus::Repairing { remaining_ticks: 1 });

        let after_two = tick(&after_one, false);
        assert_eq!(after_two.robots[0].status, RobotStatus::Operational);
        assert_eq!(after_two.robots[0].worn_ticks, 0, "worn_ticks should reset to 0 once repair completes");
    }

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
        assert_eq!(
            state.products[0].pos,
            (station_x + 1, BELT_ROW),
            "조립이 끝나 stage가 오른 바로 그 틱에 제품이 한 칸 전진해야 한다(같은 틱 내 즉시 이동)"
        );
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
    fn lower_id_wins_when_two_products_target_the_same_cell() {
        // 정상적인 벨트 흐름에서는 제품 위치가 항상 서로 달라(1차선이라
        // 서로 다른 위치의 제품은 절대 같은 칸을 노릴 수 없다) 이 상황이
        // 자연스럽게 발생하지 않는다 — 그래도 `plan_products`가 로봇과
        // 공유하는 `resolve_intents`의 id 타이브레이크를 실제로 올바르게
        // 호출/적용하는지는 직접 검증해야 한다(설계문서 §7). 뮤테이션
        // 테스트로 실측: 이 테스트를 추가하기 전에는 `resolve_intents`의
        // 타이브레이크 방향(`intent.mover_id < *winner`)을 반대로 뒤집어도
        // (높은 id가 이기게 바꿔도) 기존 테스트가 단 하나도 실패하지
        // 않았다 — 이 태스크 시점엔 로봇이 전혀 움직이지 않고(조립 로봇은
        // 고정, 헬퍼는 Task 3 전까지 정지) 벨트는 1차선이라 제품끼리도
        // 자연 충돌이 없기 때문이다. 그래서 의도적으로 두 제품을 같은
        // 칸에 겹쳐 두고(비정상 상태) `resolve_intents` 경로를 직접
        // 노출시킨다.
        let mut state = state_with_products(vec![Product::new(2, (5, BELT_ROW)), Product::new(1, (5, BELT_ROW))]);

        state = tick(&state, true);

        let winner = state.products.iter().find(|p| p.pos == (6, BELT_ROW)).expect("정확히 한 제품만 전진해야 한다");
        assert_eq!(winner.id, 1, "낮은 id가 이겨야 한다");
        let loser = state.products.iter().find(|p| p.id == 2).unwrap();
        assert_eq!(loser.pos, (5, BELT_ROW), "높은 id는 원래 칸에 남아야 한다");
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

    #[test]
    fn a_depleted_station_gets_exactly_one_restock_request_queued() {
        let mut state = SimState::new(Arc::new(Grid::new(9, 7)), Vec::new());
        state.stations[0].part_inventory = 0;

        let next = tick(&state, true);

        // 헬퍼가 한 대도 없으므로 배정될 로봇이 있을 수 없다 — 큐 길이와는
        // 별개로 확인한다(둘을 하나로 합쳐 세면, 큐에 정확히 1개가 쌓이는
        // 정상 동작 자체가 이 합계를 0이 아니게 만들어 항상 모순되므로 분리).
        let assigned_count = next
            .robots
            .iter()
            .filter(|r| r.helper_assignment == Some(HelperTask::RestockStation { station_index: 0 }))
            .count();
        assert_eq!(assigned_count, 0, "헬퍼가 한 대도 없으면 요청만 큐에 쌓이고 아무도 배정받지 않는다");
        // `pending_helper_tasks.len()` 전체가 아니라 RestockStation{0} 요청
        // 개수만 센다: `SimState::new`는 제품 없이 시작하므로 라인 시작점도
        // 비어 있어(설계문서 §5-4) 같은 틱에 `DeliverFrame` 요청도 정당하게
        // 함께 큐에 들어간다 — 그건 이 테스트가 검증하려는 대상이 아니다.
        let restock_request_count = |s: &SimState| {
            s.pending_helper_tasks.iter().filter(|t| **t == HelperTask::RestockStation { station_index: 0 }).count()
        };
        assert_eq!(restock_request_count(&next), 1, "요청은 정확히 한 번만 큐에 들어가야 한다(중복 방지)");

        let after_another_tick = tick(&next, true);
        assert_eq!(
            restock_request_count(&after_another_tick),
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
    fn helper_drop_countdown_actually_elapses_after_arriving_and_is_not_skipped() {
        // 계획서에는 없던 추가 테스트 — 리뷰 중 뮤테이션 테스트로 실제
        // 버그를 하나 발견해서 그 회귀를 막기 위해 추가했다. 헬퍼가 목적지
        // 칸에 도착하는 바로 그 틱엔 `work_ticks_remaining`이 (이동 전부터)
        // 계속 0인 채라(픽업 카운트다운이 이미 끝나 있었으므로), 만약
        // "도착"과 "카운트다운 시작"을 별개 틱으로 나누면
        // `run_helper_logistics`의 드롭 완료 판정(carrying &&
        // work_ticks_remaining == 0 && pos == destination)이 "막 도착함"과
        // "카운트다운이 다 끝남"을 구분 못 해 `HELPER_DROP_TICKS` 전체를
        // 건너뛰고 도착 즉시 드롭을 완료시켜 버렸다(실제로 재현: 이동
        // 거리만큼의 틱 수만에 완료됨). `plan_helper`가 도착 틱에 곧바로
        // 카운트다운을 시작하도록 고쳐서 고쳤다 — 이 테스트는 그 고정을
        // 검증한다(총 소요 틱 = 이동 거리 + HELPER_DROP_TICKS 이상이어야
        // 함).
        let mut robot = Robot::new(1, WAREHOUSE_CELL, WAREHOUSE_CELL);
        robot.carrying = true;
        robot.work_ticks_remaining = 0;
        robot.helper_assignment = Some(HelperTask::RestockStation { station_index: 0 });
        let mut state = SimState::new(Arc::new(Grid::new(9, 7)), vec![robot]);
        state.stations[0].part_inventory = 0;

        let manhattan_distance = (WAREHOUSE_CELL.0 - STATION_XS[0]).abs() + (WAREHOUSE_CELL.1 - STATION_ROBOT_ROW).abs();

        let mut ticks_elapsed = 0;
        loop {
            state = tick(&state, true);
            ticks_elapsed += 1;
            if state.robots[0].helper_assignment.is_none() {
                break;
            }
            if ticks_elapsed > 100 {
                panic!("never completed");
            }
        }
        assert!(
            ticks_elapsed >= manhattan_distance as u32 + HELPER_DROP_TICKS,
            "expected at least travel({manhattan_distance}) + HELPER_DROP_TICKS({HELPER_DROP_TICKS}) ticks, got {ticks_elapsed}"
        );
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

    #[test]
    fn two_simultaneously_depleted_stations_are_each_restocked_by_a_different_idle_helper() {
        // 기존 헬퍼 테스트 5개는 전부 스테이션 1개 + 헬퍼 최대 1대만
        // 다뤄서, 여러 스테이션이 같은 틱에 동시에 고갈되고 여러 헬퍼가
        // 동시에 놀고 있을 때 "각 스테이션이 서로 다른 헬퍼에게 배정돼
        // 결국 둘 다 채워지는지"는 실측된 적이 없었다(코드 리뷰에서 손으로
        // 추적한 결과 로직 자체는 맞다고 확인됨 — enqueue 루프가 스테이션
        // 마다 별개의 `RestockStation{0}`/`RestockStation{1}` 태스크를
        // 큐에 넣고, FIFO 배정이 서로 다른 헬퍼에게 순서대로 나눠주므로).
        let mut state = SimState::new(
            Arc::new(Grid::new(9, 7)),
            vec![Robot::new(1, WAREHOUSE_CELL, WAREHOUSE_CELL), Robot::new(2, WAREHOUSE_CELL, WAREHOUSE_CELL)],
        );
        state.stations[0].part_inventory = 0;
        state.stations[1].part_inventory = 0;
        // 스테이션 2는 일부러 그대로 둔다(가득 참) — 배정 로직이 고갈된
        // 스테이션만 골라내고 멀쩡한 스테이션은 안 건드리는지도 같이 확인.
        assert_eq!(state.stations[2].part_inventory, STATION_MAX_INVENTORY);

        let next = tick(&state, true);

        let assignment_of = |s: &SimState, id: u32| s.robots.iter().find(|r| r.id == id).unwrap().helper_assignment;
        assert_eq!(assignment_of(&next, 1), Some(HelperTask::RestockStation { station_index: 0 }));
        assert_eq!(assignment_of(&next, 2), Some(HelperTask::RestockStation { station_index: 1 }));
        assert_ne!(
            assignment_of(&next, 1),
            assignment_of(&next, 2),
            "두 헬퍼가 같은 태스크를 놓고 경쟁하면 안 되고 서로 다른 스테이션을 맡아야 한다"
        );

        let mut state = next;
        let mut station0_done = false;
        let mut station1_done = false;
        for _ in 0..500 {
            state = tick(&state, true);
            station0_done |= state.stations[0].part_inventory == STATION_MAX_INVENTORY;
            station1_done |= state.stations[1].part_inventory == STATION_MAX_INVENTORY;
            if station0_done && station1_done {
                break;
            }
        }
        assert!(station0_done, "스테이션 0도 결국 재고가 채워져야 한다");
        assert!(station1_done, "스테이션 1도 결국 재고가 채워져야 한다");
        assert_eq!(
            state.stations[2].part_inventory, STATION_MAX_INVENTORY,
            "원래부터 가득 차 있던 스테이션 2는 그대로 유지돼야 한다"
        );
    }

    #[test]
    fn multiple_simultaneously_idle_helpers_do_not_permanently_block_a_helper_returning_to_the_warehouse_cell() {
        // 실제로 있었던 배포 정지 버그의 sim_core 레벨 회귀 테스트
        // (bisected to commit 6576653). 예전엔 `set_robot_count`의 성장
        // 분기가 새 헬퍼를 전부 `WAREHOUSE_CELL` 자체에 스폰했다. 유휴
        // (`helper_assignment == None`) 헬퍼는 `plan_helper`가 절대
        // 움직이지 않으므로, 헬퍼 수가 동시 대기 작업 수보다 많아지면
        // 남는 헬퍼들이 `WAREHOUSE_CELL`을 영구 점거했다 — `find_path`는
        // 목표 칸이 점유돼 있어도 그쪽으로 향하는 경로 자체는 허용하지만
        // (pathfind.rs 참고), `advance_along_path`의 마지막 한 칸 진입은
        // `!occupied.contains(&next_cell)`로 막혀서, 실제 픽업을 하러 그
        // 칸에 도착해야 하는 활성 헬퍼가 도착 직전에서 영원히 멈췄다.
        //
        // 지금은 `set_robot_count`가 새 헬퍼를 `HELPER_SPAWN_STAGING_CELLS`
        // (WAREHOUSE_CELL 자체는 피함)로 분산 스폰한다 — 이 테스트는 그
        // 스폰 규칙을 그대로 재현해서(id를 배열 길이로 나눈 나머지로
        // 인덱싱, `set_robot_count`와 완전히 같은 방식) 여러 헬퍼가
        // 동시에 유휴 상태로 남아 있어도, 실제 배정을 받아 창고까지
        // 가야 하는 헬퍼가 끝내 도착해서 스테이션을 채우는지 검증한다.
        //
        // 뮤테이션 테스트로 실측: 아래 `idle_cell_for`가 `WAREHOUSE_CELL`을
        // 반환하도록(예전 버그처럼) 일시적으로 바꿔봤더니 이 테스트가
        // 2000틱 안에 restocked==false로 실패하는 것을 확인했다 — 즉 이
        // 테스트는 실제로 이 회귀를 잡아낸다(공허한 테스트가 아님).
        let idle_cell_for = |id: u32| HELPER_SPAWN_STAGING_CELLS[id as usize % HELPER_SPAWN_STAGING_CELLS.len()];

        // "mover" — 창고에서 멀리 떨어진 곳에서 시작하고 id가 가장 낮아
        // FIFO 배정 1순위(run_helper_logistics의 robot_id 오름차순
        // 규칙)라서, 아래 고갈된 스테이션의 RestockStation 작업을 반드시
        // 이 로봇이 받는다 — 그래서 실제로 WAREHOUSE_CELL까지 이동해야
        // 하는 상황이 보장된다.
        let mut robots = vec![Robot::new(1, (0, 6), (0, 6))];
        for id in [2u32, 3, 4, 5] {
            let cell = idle_cell_for(id);
            robots.push(Robot::new(id, cell, cell));
        }
        let mut state = SimState::new(Arc::new(Grid::new(9, 7)), robots);
        state.stations[0].part_inventory = 0;

        let mut restocked = false;
        for _ in 0..2000 {
            state = tick(&state, true);
            if state.stations[0].part_inventory == STATION_MAX_INVENTORY {
                restocked = true;
                break;
            }
        }

        assert!(
            restocked,
            "여러 헬퍼가 창고 대기 칸들에 동시에 유휴 상태로 남아 있어도, 배정받은 헬퍼가 결국 \
             WAREHOUSE_CELL까지 도달해 스테이션을 채워야 한다(유휴 헬퍼가 WAREHOUSE_CELL 자체를 \
             점거해 막는 회귀가 생기면 여기서 영원히 채워지지 않는다)"
        );
    }
}
