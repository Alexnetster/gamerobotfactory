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

    /// `find_path`는 `blocked`(다른 로봇이 현재 점유한 칸)를 장애물로 취급해
    /// 우회해야 하지만, 목표 칸 자체는 `blocked`에 있어도 예외적으로 항상
    /// 도달 가능해야 한다 (그 로봇이 다음 틱에 비킬 수도 있으므로). 이
    /// 테스트는 `blocked`를 무작위로 채워, 경로 중 목표 칸을 제외한 어떤
    /// 칸도 `blocked`에 속하지 않음을 확인한다.
    #[test]
    fn path_avoids_blocked_cells_except_goal(
        ((grid, start, goal), blocked) in (
            arbitrary_grid_and_endpoints(),
            proptest::collection::hash_set((0..SIZE, 0..SIZE), 0..6),
        )
    ) {
        prop_assume!(grid.is_walkable(start));
        prop_assume!(grid.is_walkable(goal));

        if let Some(path) = find_path(&grid, start, goal, &blocked) {
            let last_index = path.len().saturating_sub(1);
            for (i, cell) in path.iter().enumerate() {
                if i == last_index {
                    // the goal cell itself is always allowed, even if blocked
                    continue;
                }
                prop_assert!(!blocked.contains(cell));
            }
        }
    }
}
