use sim_core::sim::{Robot, RobotStatus, SimState, Task, REPAIR_TICKS};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conveyor {
    pub running: bool,
}

impl Conveyor {
    pub fn new() -> Self {
        Conveyor { running: true }
    }
}

impl Default for Conveyor {
    fn default() -> Self {
        Self::new()
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

// The shared `RobotNot*` prefix is intentional here (mirrors the domain
// language used across game_state/ws/protocol), not an accidental naming
// collision, so the lint is suppressed rather than acted on.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    RobotNotFound(u32),
    RobotNotOperational(u32),
    RobotNotFailed(u32),
}

/// 설계문서의 성능 목표(20~50대)를 넉넉히 웃도는 상한. 이보다 큰 값이
/// 오면 거부하지 않고 이 값으로 잘라서 받아들인다 — 클라이언트 실수나
/// 악의적 입력으로 전역 락을 잡은 채 무한 할당 루프에 빠지는 것을 막는다.
pub const MAX_ROBOT_COUNT: usize = 200;

impl GameState {
    pub fn new(sim: SimState) -> Self {
        let next_robot_id = sim.robots.iter().map(|r| r.id).max().map_or(0, |max| max + 1);
        GameState { sim, conveyor: Conveyor::new(), selected_robot: None, next_robot_id }
    }

    pub fn toggle_conveyor(&mut self) {
        self.conveyor.running = !self.conveyor.running;
    }

    /// 로봇 대수를 정확히 `target`대로 맞춘다(단 `MAX_ROBOT_COUNT`로
    /// 클램프). 늘려야 하면 그리드 원점 근처의 빈 칸에 새 로봇을
    /// 스폰하고(자기 자신을 목표로 삼아 제자리 대기), 줄여야 하면 ID가
    /// 가장 큰 로봇부터 제거한다.
    pub fn set_robot_count(&mut self, target: usize) {
        let target = target.min(MAX_ROBOT_COUNT);
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
        if robot.status != RobotStatus::Operational {
            return Err(CommandError::RobotNotOperational(robot_id));
        }
        robot.task = task;
        Ok(())
    }

    /// 고장난(`Failed`) 로봇을 복구 시작 상태로 전이시킨다. `REPAIR_TICKS`
    /// 동안 `Repairing` 상태를 거친 뒤 `sim_core::sim::tick()`(Task 1에서
    /// 추가한 `update_status`)이 자동으로 `Operational`로 되돌리고
    /// `worn_ticks`를 리셋한다 — 이 함수는 그 카운트다운을 시작만 한다.
    pub fn repair_robot(&mut self, robot_id: u32) -> Result<(), CommandError> {
        let robot = self
            .sim
            .robots
            .iter_mut()
            .find(|r| r.id == robot_id)
            .ok_or(CommandError::RobotNotFound(robot_id))?;
        if robot.status != RobotStatus::Failed {
            return Err(CommandError::RobotNotFailed(robot_id));
        }
        robot.status = RobotStatus::Repairing { remaining_ticks: REPAIR_TICKS };
        Ok(())
    }

    /// 현재 `Failed`인 로봇 전부를 한 번에 수리 시작 상태로 전이시킨다.
    /// `repair_robot`과 같은 전이를 쓰지만, 대상이 아예 없거나 일부만
    /// `Failed`여도 오류를 내지 않는다("전부 다"라는 벌크 커맨드의
    /// 의미상 부분 매칭이 실패가 아니기 때문) — 실제로 수리를 시작시킨
    /// 로봇 수를 반환해서 로깅/관측에 쓸 수 있게 한다.
    pub fn repair_all_failed_robots(&mut self) -> usize {
        let mut repaired = 0;
        for robot in self.sim.robots.iter_mut() {
            if robot.status == RobotStatus::Failed {
                robot.status = RobotStatus::Repairing { remaining_ticks: REPAIR_TICKS };
                repaired += 1;
            }
        }
        repaired
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
    fn set_robot_count_clamps_to_max() {
        let mut state = empty_state();
        state.set_robot_count(usize::MAX);
        assert_eq!(state.sim.robots.len(), MAX_ROBOT_COUNT);
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

    #[test]
    fn trigger_arm_action_rejects_non_operational_robot() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = state.sim.robots[0].id;
        state.sim.robots[0].status = RobotStatus::Failed;

        let err = state.trigger_arm_action(id, Task::Picking);

        assert_eq!(err, Err(CommandError::RobotNotOperational(id)));
    }

    #[test]
    fn repair_robot_transitions_a_failed_robot_to_repairing() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = state.sim.robots[0].id;
        state.sim.robots[0].status = RobotStatus::Failed;

        state.repair_robot(id).unwrap();

        assert_eq!(state.sim.robots[0].status, RobotStatus::Repairing { remaining_ticks: REPAIR_TICKS });
    }

    #[test]
    fn repair_robot_rejects_a_non_failed_robot() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = state.sim.robots[0].id;

        let err = state.repair_robot(id);

        assert_eq!(err, Err(CommandError::RobotNotFailed(id)));
    }

    #[test]
    fn repair_robot_rejects_unknown_robot() {
        let mut state = empty_state();
        let err = state.repair_robot(999);
        assert_eq!(err, Err(CommandError::RobotNotFound(999)));
    }

    #[test]
    fn repair_all_failed_robots_repairs_only_the_failed_ones_and_counts_them() {
        let mut state = empty_state();
        state.set_robot_count(3);
        state.sim.robots[0].status = RobotStatus::Failed;
        state.sim.robots[1].status = RobotStatus::Operational;
        state.sim.robots[2].status = RobotStatus::Failed;

        let repaired = state.repair_all_failed_robots();

        assert_eq!(repaired, 2);
        assert_eq!(state.sim.robots[0].status, RobotStatus::Repairing { remaining_ticks: REPAIR_TICKS });
        assert_eq!(state.sim.robots[1].status, RobotStatus::Operational, "an already-Operational robot must be left alone");
        assert_eq!(state.sim.robots[2].status, RobotStatus::Repairing { remaining_ticks: REPAIR_TICKS });
    }

    #[test]
    fn repair_all_failed_robots_is_a_harmless_no_op_when_nothing_is_failed() {
        let mut state = empty_state();
        state.set_robot_count(2);

        let repaired = state.repair_all_failed_robots();

        assert_eq!(repaired, 0);
        assert!(state.sim.robots.iter().all(|r| r.status == RobotStatus::Operational));
    }

    #[test]
    fn select_robot_works_on_a_failed_robot() {
        // 스펙의 명시적 결정: 고장난 로봇도 선택은 계속 허용해야 오퍼레이터가
        // 상태를 보고 RepairRobot 대상으로 지정할 수 있다.
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = state.sim.robots[0].id;
        state.sim.robots[0].status = RobotStatus::Failed;

        state.select_robot(id).unwrap();

        assert_eq!(state.selected_robot, Some(id));
    }

    #[test]
    fn set_robot_count_shrink_removes_highest_id_even_if_it_is_repairing() {
        // 스펙의 명시적 v1 결정: 상태 인지 제거 우선순위는 두지 않는다 —
        // 복구 중인 로봇도 ID가 가장 크면 그대로 제거 대상이다.
        let mut state = empty_state();
        state.set_robot_count(2);
        let highest_id = state.sim.robots.iter().map(|r| r.id).max().unwrap();
        state
            .sim
            .robots
            .iter_mut()
            .find(|r| r.id == highest_id)
            .unwrap()
            .status = RobotStatus::Repairing { remaining_ticks: 10 };

        state.set_robot_count(1);

        assert!(
            !state.sim.robots.iter().any(|r| r.id == highest_id),
            "a Repairing robot is not special-cased during shrink"
        );
    }
}
