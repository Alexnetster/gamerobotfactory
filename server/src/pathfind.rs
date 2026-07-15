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
/// 있어도 그쪽으로 향하는 시도는 막지 않는다 — 그 로봇이 다음 틱에 비킬
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

    #[test]
    fn goal_cell_is_reachable_even_if_blocked() {
        let grid = Grid::new(3, 1);
        let mut blocked = HashSet::new();
        blocked.insert((2, 0)); // the goal itself is occupied
        let path = find_path(&grid, (0, 0), (2, 0), &blocked).unwrap();
        assert_eq!(path, vec![(1, 0), (2, 0)]);
    }
}
