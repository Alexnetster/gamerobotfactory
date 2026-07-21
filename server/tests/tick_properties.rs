use proptest::prelude::*;
use proptest::sample::subsequence;
use sim_core::grid::{CellId, Grid};
use sim_core::sim::{tick, Robot, RobotStatus, SimState};
use std::collections::HashSet;
use std::sync::Arc;

const SIZE: i32 = 8;
const ROBOT_COUNT: usize = 5;

fn all_cells() -> Vec<CellId> {
    (0..SIZE).flat_map(|x| (0..SIZE).map(move |y| (x, y))).collect()
}

/// `ROBOT_COUNT`개의 서로 다른 시작 칸을 뽑는다. `subsequence`는 중복 없는
/// `all_cells()` 목록에서 순서를 보존한 부분열을 뽑으므로, 독립적으로 무작위
/// 칸을 뽑아 우연히 겹치지 않기를 바라는 방식과 달리 distinctness가 항상
/// 보장된다.
fn distinct_starts() -> impl Strategy<Value = Vec<CellId>> {
    subsequence(all_cells(), ROBOT_COUNT)
}

fn goals() -> impl Strategy<Value = Vec<CellId>> {
    proptest::collection::vec((0..SIZE, 0..SIZE), ROBOT_COUNT)
}

fn arbitrary_sim_state() -> impl Strategy<Value = SimState> {
    (distinct_starts(), goals()).prop_map(|(starts, goals)| {
        let robots: Vec<Robot> = starts
            .into_iter()
            .zip(goals)
            .enumerate()
            .map(|(i, (pos, goal))| Robot::new(i as u32, pos, goal))
            .collect();
        SimState { grid: Arc::new(Grid::new(SIZE, SIZE)), robots, tick_count: 0 }
    })
}

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

proptest! {
    /// 서로 다른 칸에서 시작한 N대의 로봇은, 한 틱을 진행한 뒤에도 서로
    /// 다른 칸에 있어야 한다. `tick`의 계획은 틱 시작 시점의 점유 스냅샷을
    /// 기준으로만 다음 칸이 비어 있는지 확인하고, 같은 빈 칸을 여러 로봇이
    /// 노리면 `resolve_intents`가 정확히 하나만 승자로 남기고 나머지는
    /// (원래 서로 달랐던) 자기 칸으로 되돌리므로, N대로 확장해도 충돌이
    /// 없어야 한다는 것이 크레이트의 핵심 주장이다.
    #[test]
    fn tick_never_produces_collisions(state in arbitrary_sim_state(), conveyor_running: bool) {
        let next = tick(&state, conveyor_running);

        let mut seen = HashSet::new();
        for robot in &next.robots {
            prop_assert!(seen.insert(robot.pos), "duplicate position after tick: {:?}", robot.pos);
        }
    }

    /// `tick`은 순수 함수이므로, 동일한 입력 상태에서 두 번 호출해도 항상
    /// 같은 결과 위치들을 내야 한다 (스레드 스케줄링이나 rayon의 병렬 실행
    /// 순서에 영향받지 않는 결정적 타이브레이크 덕분).
    #[test]
    fn tick_is_deterministic(state in arbitrary_sim_state(), conveyor_running: bool) {
        let positions_a: Vec<CellId> = tick(&state, conveyor_running).robots.iter().map(|r| r.pos).collect();
        let positions_b: Vec<CellId> = tick(&state, conveyor_running).robots.iter().map(|r| r.pos).collect();
        prop_assert_eq!(positions_a, positions_b);
    }

    /// Failed/Repairing 로봇이 섞여 있어도 충돌 방지 불변식이 유지된다.
    #[test]
    fn tick_never_produces_collisions_with_frozen_robots(state in arbitrary_sim_state_with_some_frozen_robots(), conveyor_running: bool) {
        let next = tick(&state, conveyor_running);

        let mut seen = HashSet::new();
        for robot in &next.robots {
            prop_assert!(seen.insert(robot.pos), "duplicate position after tick: {:?}", robot.pos);
        }
    }

    /// Failed/Repairing 로봇이 섞여 있어도(마모/고장 로직이 결정적 해시를
    /// 쓰므로) tick()은 여전히 순수 함수여야 한다.
    #[test]
    fn tick_is_deterministic_with_frozen_robots(state in arbitrary_sim_state_with_some_frozen_robots(), conveyor_running: bool) {
        let a: Vec<(CellId, RobotStatus)> = tick(&state, conveyor_running).robots.iter().map(|r| (r.pos, r.status)).collect();
        let b: Vec<(CellId, RobotStatus)> = tick(&state, conveyor_running).robots.iter().map(|r| (r.pos, r.status)).collect();
        prop_assert_eq!(a, b);
    }

    /// Failed로, 또는 Repairing으로(단 이번 틱에 복구가 끝나지 않는
    /// `remaining_ticks > 1` 조건으로) 시딩된 로봇은 한 틱이 지나도 원래
    /// 칸에 그대로 있어야 한다 — Task 1의 예시 기반 단위테스트를 임의의
    /// 그리드/로봇 배치로 넓게 재확인한다.
    ///
    /// `remaining_ticks == 1`은 일부러 제외한다: `plan_robot`은
    /// `update_status`를 먼저 실행해 상태를 갱신한 *뒤에* "Operational이
    /// 아니면 얼어붙는다"를 검사하므로(server/src/sim.rs), 복구가 이번
    /// 틱에 끝나는 로봇(remaining_ticks: 1 -> Operational)은 같은 틱 안에서
    /// 바로 이동까지 재개할 수 있다 — 이는 실제로 관찰된 동작이며(하던
    /// 작업을 복구 완료 후 잊지 않는다는 설계 의도와 일치), 버그가
    /// 아니라 이 프로퍼티가 적용되는 대상에서 제외해야 할 경우다.
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
}
