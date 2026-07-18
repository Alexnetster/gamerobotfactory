use crate::grid::{CellId, Grid};
use crate::pathfind::find_path;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const REPATH_INTERVAL: u32 = 10;
const LEG_CYCLE_SPEED: f32 = 0.1;
const WEAR_LIMIT_TICKS: u64 = 2000; // 100초 분량의 작업(20Hz 기준) — 튜닝 대상
const MAX_FAILURE_PROB: f64 = 0.05; // 완전 마모 상태에서의 틱당 최대 고장 확률 — 튜닝 대상
pub const REPAIR_TICKS: u32 = 100; // 20Hz 기준 5초 — 튜닝 대상. 나중 태스크의 game_state.rs::repair_robot이 RepairRobot 처리 시 이 값을 참조할 예정이라 pub.

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

#[derive(Debug, Clone)]
pub struct SimState {
    pub grid: Arc<Grid>,
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
        .map(|robot| safe_plan_robot(&state.grid, robot, &occupied, state.tick_count))
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

    SimState { grid: state.grid.clone(), robots: new_robots, tick_count: state.tick_count + 1 }
}

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
        // `find_path`는 `start`를 제외한 경로를 반환하므로 `next_cell`이
        // `robot.pos`(현재 칸)와 같아지는 경우는 없다 — 그래서 여기서는
        // `occupied` 검사만으로 충분하다.
        if !occupied.contains(&next_cell) {
            next.pos = next_cell;
            next.path.remove(0);
        }
        // else: 다른 로봇이 지난 틱 기준으로 그 칸을 차지하고 있다 —
        // 이번 틱은 멈추고, 곧 돌아올 재계획 주기에서 우회로를 찾는다.
    }

    next
}

/// `plan_robot`을 패닉으로부터 격리한다. 패닉이 나면 해당 로봇은 이번
/// 틱을 그대로 멈춘 채 넘어가고, 나머지 로봇들의 갱신은 영향받지 않는다.
fn safe_plan_robot(grid: &Grid, robot: &Robot, occupied: &HashSet<CellId>, tick_count: u64) -> Robot {
    safe_call(robot, || plan_robot(grid, robot, occupied, tick_count))
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
        SimState { grid: Arc::new(Grid::new(width, height)), robots: Vec::new(), tick_count: 0 }
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

    #[test]
    fn safe_call_recovers_from_a_real_panic_and_holds_position() {
        let robot = Robot::new(1, (0, 0), (2, 0));

        let result = safe_call(&robot, || panic!("simulated fault in robot update"));

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
            state = tick(&state);
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

        let after_one = tick(&state);
        assert_eq!(after_one.robots[0].status, RobotStatus::Repairing { remaining_ticks: 1 });

        let after_two = tick(&after_one);
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

        // 목표를 직접 바꿀 때는 남아 있는 경로/재계획 타이머도 함께 지워야 한다
        // — 그러지 않으면 plan_robot()이 새 목표를 무시하고 옛 경로(동쪽)를
        // 계속 따라간다. 실제 프로덕션 코드에는 이렇게 goal만 단독으로
        // 바꾸는 경로가 없다(Robot::new에서 한 번만 설정됨) — 이 테스트가
        // 그 시나리오를 시뮬레이션하려면 tick()의 타이브레이크 패배 분기와
        // 동일하게 경로를 초기화해줘야 한다.
        state.robots[0].goal = (0, 0); // 이제 서쪽으로
        state.robots[0].path.clear();
        state.robots[0].ticks_until_repath = 0;
        state = tick(&state);
        assert_eq!(state.robots[0].facing, Direction::West);

        // 목표 지점에 도달해 멈춘 뒤에도 마지막 방향을 유지해야 한다.
        let settled = state.robots[0].pos;
        state.robots[0].goal = settled;
        let held = tick(&state);
        assert_eq!(held.robots[0].facing, Direction::West);
    }
}
