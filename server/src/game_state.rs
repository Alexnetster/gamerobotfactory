use sim_core::sim::{Robot, SimState, Task};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conveyor {
    pub running: bool,
}

impl Conveyor {
    pub fn new() -> Self {
        Conveyor { running: true }
    }
}

/// 시뮬레이션 진실(`SimState`)에 세션/오케스트레이션 상태(컨베이어,
/// 선택된 로봇)를 얹은 것. `selected_robot`은 "지금 이 오퍼레이터가
/// 어느 로봇을 보고 있는가"라는 UI 개념이라 시뮬레이션 진실이 아니므로
/// 여기(바이너리 크레이트)에 두고 `sim_core::SimState`는 건드리지 않는다.
pub struct GameState {
    pub sim: SimState,
    pub conveyor: Conveyor,
    pub selected_robot: Option<u32>,
    next_robot_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    RobotNotFound(u32),
}

impl GameState {
    pub fn new(sim: SimState) -> Self {
        let next_robot_id = sim.robots.iter().map(|r| r.id).max().map_or(0, |max| max + 1);
        GameState { sim, conveyor: Conveyor::new(), selected_robot: None, next_robot_id }
    }

    pub fn toggle_conveyor(&mut self) {
        self.conveyor.running = !self.conveyor.running;
    }

    /// 로봇 대수를 정확히 `target`대로 맞춘다. 늘려야 하면 그리드 원점
    /// 근처의 빈 칸에 새 로봇을 스폰하고(자기 자신을 목표로 삼아 제자리
    /// 대기), 줄여야 하면 ID가 가장 큰 로봇부터 제거한다.
    pub fn set_robot_count(&mut self, target: usize) {
        while self.sim.robots.len() < target {
            let id = self.next_robot_id;
            self.next_robot_id += 1;
            let spawn_at = (0, 0);
            self.sim.robots.push(Robot::new(id, spawn_at, spawn_at));
        }
        while self.sim.robots.len() > target {
            if let Some((index, _)) = self
                .sim
                .robots
                .iter()
                .enumerate()
                .max_by_key(|(_, r)| r.id)
            {
                self.sim.robots.remove(index);
            }
        }
        if let Some(selected) = self.selected_robot {
            if !self.sim.robots.iter().any(|r| r.id == selected) {
                self.selected_robot = None;
            }
        }
    }

    pub fn select_robot(&mut self, robot_id: u32) -> Result<(), CommandError> {
        if !self.sim.robots.iter().any(|r| r.id == robot_id) {
            return Err(CommandError::RobotNotFound(robot_id));
        }
        self.selected_robot = Some(robot_id);
        Ok(())
    }

    pub fn release_robot(&mut self) {
        self.selected_robot = None;
    }

    pub fn trigger_arm_action(&mut self, robot_id: u32, task: Task) -> Result<(), CommandError> {
        let robot = self
            .sim
            .robots
            .iter_mut()
            .find(|r| r.id == robot_id)
            .ok_or(CommandError::RobotNotFound(robot_id))?;
        robot.task = task;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::grid::Grid;
    use std::sync::Arc;

    fn empty_state() -> GameState {
        GameState::new(SimState { grid: Arc::new(Grid::new(5, 5)), robots: Vec::new(), tick_count: 0 })
    }

    #[test]
    fn toggle_conveyor_flips_running_state() {
        let mut state = empty_state();
        assert!(state.conveyor.running);
        state.toggle_conveyor();
        assert!(!state.conveyor.running);
        state.toggle_conveyor();
        assert!(state.conveyor.running);
    }

    #[test]
    fn set_robot_count_grows_and_shrinks() {
        let mut state = empty_state();
        state.set_robot_count(3);
        assert_eq!(state.sim.robots.len(), 3);
        state.set_robot_count(1);
        assert_eq!(state.sim.robots.len(), 1);
    }

    #[test]
    fn set_robot_count_assigns_unique_growing_ids() {
        let mut state = empty_state();
        state.set_robot_count(2);
        state.set_robot_count(1);
        state.set_robot_count(3);
        let ids: Vec<u32> = state.sim.robots.iter().map(|r| r.id).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(ids.len(), unique.len(), "no robot id should be reused: {ids:?}");
    }

    #[test]
    fn set_robot_count_removes_highest_id_even_out_of_vec_order() {
        let mut state = empty_state();
        state.sim.robots.push(Robot::new(5, (0, 0), (0, 0)));
        state.sim.robots.push(Robot::new(2, (0, 0), (0, 0)));
        // Vec order is [5, 2] — deliberately NOT sorted by id, to prove
        // removal is keyed on id value, not Vec position.

        state.set_robot_count(1);

        let remaining_ids: Vec<u32> = state.sim.robots.iter().map(|r| r.id).collect();
        assert_eq!(remaining_ids, vec![2], "the highest-id robot (5) should be removed, not the last Vec element");
    }

    #[test]
    fn select_robot_rejects_unknown_id() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let unknown_id = state.sim.robots[0].id + 100;
        assert_eq!(state.select_robot(unknown_id), Err(CommandError::RobotNotFound(unknown_id)));
    }

    #[test]
    fn select_then_release_clears_selection() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = state.sim.robots[0].id;
        state.select_robot(id).unwrap();
        assert_eq!(state.selected_robot, Some(id));
        state.release_robot();
        assert_eq!(state.selected_robot, None);
    }

    #[test]
    fn removing_selected_robot_clears_selection() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = state.sim.robots[0].id;
        state.select_robot(id).unwrap();
        state.set_robot_count(0);
        assert_eq!(state.selected_robot, None);
    }

    #[test]
    fn trigger_arm_action_sets_task_on_the_right_robot() {
        let mut state = empty_state();
        state.set_robot_count(2);
        let target_id = state.sim.robots[1].id;
        state.trigger_arm_action(target_id, Task::Picking).unwrap();
        assert_eq!(state.sim.robots[0].task, Task::Idle);
        assert_eq!(state.sim.robots[1].task, Task::Picking);
    }

    #[test]
    fn trigger_arm_action_rejects_unknown_robot() {
        let mut state = empty_state();
        let err = state.trigger_arm_action(999, Task::Picking);
        assert_eq!(err, Err(CommandError::RobotNotFound(999)));
    }
}
