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
