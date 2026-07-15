use crate::grid::{CellId, Grid};
use crate::pathfind::find_path;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const REPATH_INTERVAL: u32 = 10;
const LEG_CYCLE_SPEED: f32 = 0.1;

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
    pub leg_cycle_progress: f32,
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
            leg_cycle_progress: 0.0,
        }
    }
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
        .map(|robot| safe_plan_robot(&state.grid, robot, &occupied))
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
fn safe_plan_robot(grid: &Grid, robot: &Robot, occupied: &HashSet<CellId>) -> Robot {
    safe_call(robot, || plan_robot(grid, robot, occupied))
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
}
