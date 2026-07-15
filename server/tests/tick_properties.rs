use proptest::prelude::*;
use proptest::sample::subsequence;
use sim_core::grid::{CellId, Grid};
use sim_core::sim::{tick, Robot, SimState};
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

proptest! {
    /// 서로 다른 칸에서 시작한 N대의 로봇은, 한 틱을 진행한 뒤에도 서로
    /// 다른 칸에 있어야 한다. `tick`의 계획은 틱 시작 시점의 점유 스냅샷을
    /// 기준으로만 다음 칸이 비어 있는지 확인하고, 같은 빈 칸을 여러 로봇이
    /// 노리면 `resolve_intents`가 정확히 하나만 승자로 남기고 나머지는
    /// (원래 서로 달랐던) 자기 칸으로 되돌리므로, N대로 확장해도 충돌이
    /// 없어야 한다는 것이 크레이트의 핵심 주장이다.
    #[test]
    fn tick_never_produces_collisions(state in arbitrary_sim_state()) {
        let next = tick(&state);

        let mut seen = HashSet::new();
        for robot in &next.robots {
            prop_assert!(seen.insert(robot.pos), "duplicate position after tick: {:?}", robot.pos);
        }
    }

    /// `tick`은 순수 함수이므로, 동일한 입력 상태에서 두 번 호출해도 항상
    /// 같은 결과 위치들을 내야 한다 (스레드 스케줄링이나 rayon의 병렬 실행
    /// 순서에 영향받지 않는 결정적 타이브레이크 덕분).
    #[test]
    fn tick_is_deterministic(state in arbitrary_sim_state()) {
        let positions_a: Vec<CellId> = tick(&state).robots.iter().map(|r| r.pos).collect();
        let positions_b: Vec<CellId> = tick(&state).robots.iter().map(|r| r.pos).collect();
        prop_assert_eq!(positions_a, positions_b);
    }
}
