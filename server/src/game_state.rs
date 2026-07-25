use sim_core::sim::{Robot, RobotRole, RobotStatus, SimState, Task, REPAIR_TICKS};

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
        let mut state = GameState { sim, conveyor: Conveyor::new(), selected_robot: None, next_robot_id };
        state.ensure_assembly_robots_exist();
        state
    }

    /// 조립 로봇이 하나도 없으면(=새 게임 시작) 스테이션 수만큼(3대) 자동
    /// 생성한다. 이미 있으면(예: 향후 영속화된 상태를 복원하는 경우) 다시
    /// 만들지 않는다 — 멱등. 사용자가 조절할 수 없다(설계문서 §4) —
    /// `set_robot_count`는 헬퍼만 다룬다.
    fn ensure_assembly_robots_exist(&mut self) {
        let has_assembly = self.sim.robots.iter().any(|r| matches!(r.role, RobotRole::Assembly { .. }));
        if has_assembly {
            return;
        }
        for station in self.sim.stations.clone() {
            let id = self.next_robot_id;
            self.next_robot_id += 1;
            let mut robot = Robot::new(id, station.robot_cell, station.robot_cell);
            robot.role = RobotRole::Assembly { station_index: station.index };
            self.sim.robots.push(robot);
        }
    }

    pub fn toggle_conveyor(&mut self) {
        self.conveyor.running = !self.conveyor.running;
    }

    /// 헬퍼 로봇 대수를 정확히 `target`대로 맞춘다(조립 로봇 3대는
    /// 여기서 건드리지 않는다 — 설계문서 §4). 하한 1을 강제한다(설계문서
    /// §6) — 0명이 되면 재고가 바닥난 스테이션을 영영 못 채워 라인
    /// 전체가 회복 불가능하게 멈추기 때문. 상한은 기존 `MAX_ROBOT_COUNT`
    /// 그대로(조립 로봇 3대를 더한 총 로봇 수가 아니라 헬퍼 수 자체에
    /// 적용). 몇 대를 추가/제거할지 시작 시점에 한 번만 계산해 두므로
    /// 반복마다 `filter().count()`를 다시 돌지 않는다.
    pub fn set_robot_count(&mut self, target: usize) {
        let target = target.clamp(1, MAX_ROBOT_COUNT);
        let current = self.sim.robots.iter().filter(|r| r.role == RobotRole::Helper).count();

        if current < target {
            for _ in 0..(target - current) {
                let id = self.next_robot_id;
                self.next_robot_id += 1;
                self.sim.robots.push(Robot::new(id, sim_core::sim::WAREHOUSE_CELL, sim_core::sim::WAREHOUSE_CELL));
            }
        } else {
            for _ in 0..(current - target) {
                if let Some((index, _)) = self
                    .sim
                    .robots
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.role == RobotRole::Helper)
                    .max_by_key(|(_, r)| r.id)
                {
                    self.sim.robots.remove(index);
                }
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
        GameState::new(SimState::new(Arc::new(Grid::new(5, 5)), Vec::new()))
    }

    fn helper_robots(state: &GameState) -> Vec<&Robot> {
        state.sim.robots.iter().filter(|r| r.role == RobotRole::Helper).collect()
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
        assert_eq!(helper_robots(&state).len(), 3);
        state.set_robot_count(1);
        assert_eq!(helper_robots(&state).len(), 1);
    }

    #[test]
    fn set_robot_count_clamps_to_max() {
        let mut state = empty_state();
        state.set_robot_count(usize::MAX);
        assert_eq!(helper_robots(&state).len(), MAX_ROBOT_COUNT);
    }

    #[test]
    fn set_robot_count_never_goes_below_one_helper() {
        let mut state = empty_state();
        state.set_robot_count(5);
        state.set_robot_count(0);
        assert_eq!(helper_robots(&state).len(), 1, "헬퍼는 최소 1명이어야 한다");
    }

    #[test]
    fn game_state_new_always_creates_exactly_station_count_assembly_robots() {
        let state = empty_state();
        let assembly_count =
            state.sim.robots.iter().filter(|r| matches!(r.role, RobotRole::Assembly { .. })).count();
        assert_eq!(assembly_count, sim_core::sim::STATION_COUNT);
    }

    #[test]
    fn set_robot_count_never_removes_an_assembly_robot() {
        let mut state = empty_state();
        state.set_robot_count(0);
        let assembly_count =
            state.sim.robots.iter().filter(|r| matches!(r.role, RobotRole::Assembly { .. })).count();
        assert_eq!(
            assembly_count,
            sim_core::sim::STATION_COUNT,
            "set_robot_count(0)이어도 조립 로봇은 그대로 남아야 한다"
        );
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

        let remaining_ids: Vec<u32> = helper_robots(&state).iter().map(|r| r.id).collect();
        assert_eq!(remaining_ids, vec![2], "the highest-id robot (5) should be removed, not the last Vec element");
    }

    #[test]
    fn select_robot_rejects_unknown_id() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let unknown_id = helper_robots(&state)[0].id + 100;
        assert_eq!(state.select_robot(unknown_id), Err(CommandError::RobotNotFound(unknown_id)));
    }

    #[test]
    fn select_then_release_clears_selection() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = helper_robots(&state)[0].id;
        state.select_robot(id).unwrap();
        assert_eq!(state.selected_robot, Some(id));
        state.release_robot();
        assert_eq!(state.selected_robot, None);
    }

    #[test]
    fn removing_selected_robot_clears_selection() {
        // NOTE: set_robot_count now floors at 1 helper (design §6), so
        // shrinking from 1 -> 0 no longer removes anything. To actually
        // exercise "the selected robot gets removed", select the
        // highest-id helper out of 2 and shrink to 1 (shrink always
        // removes the highest-id helper).
        let mut state = empty_state();
        state.set_robot_count(2);
        let id = helper_robots(&state)[1].id;
        state.select_robot(id).unwrap();
        state.set_robot_count(1);
        assert_eq!(state.selected_robot, None);
    }

    #[test]
    fn trigger_arm_action_sets_task_on_the_right_robot() {
        let mut state = empty_state();
        state.set_robot_count(2);
        let target_id = helper_robots(&state)[1].id;
        state.trigger_arm_action(target_id, Task::Picking).unwrap();
        assert_eq!(helper_robots(&state)[0].task, Task::Idle);
        assert_eq!(helper_robots(&state)[1].task, Task::Picking);
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
        let id = helper_robots(&state)[0].id;
        state.sim.robots.iter_mut().find(|r| r.id == id).unwrap().status = RobotStatus::Failed;

        let err = state.trigger_arm_action(id, Task::Picking);

        assert_eq!(err, Err(CommandError::RobotNotOperational(id)));
    }

    #[test]
    fn repair_robot_transitions_a_failed_robot_to_repairing() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = helper_robots(&state)[0].id;
        state.sim.robots.iter_mut().find(|r| r.id == id).unwrap().status = RobotStatus::Failed;

        state.repair_robot(id).unwrap();

        assert_eq!(
            state.sim.robots.iter().find(|r| r.id == id).unwrap().status,
            RobotStatus::Repairing { remaining_ticks: REPAIR_TICKS }
        );
    }

    #[test]
    fn repair_robot_rejects_a_non_failed_robot() {
        let mut state = empty_state();
        state.set_robot_count(1);
        let id = helper_robots(&state)[0].id;

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
        // NOTE: with 3 Assembly robots always occupying indices 0..3,
        // `state.sim.robots[0..3]` would silently hit Assembly robots
        // instead of the 3 helpers this test means to target — resolve
        // by id via `helper_robots` instead of by raw index.
        let ids: Vec<u32> = helper_robots(&state).iter().map(|r| r.id).collect();
        state.sim.robots.iter_mut().find(|r| r.id == ids[0]).unwrap().status = RobotStatus::Failed;
        state.sim.robots.iter_mut().find(|r| r.id == ids[1]).unwrap().status = RobotStatus::Operational;
        state.sim.robots.iter_mut().find(|r| r.id == ids[2]).unwrap().status = RobotStatus::Failed;

        let repaired = state.repair_all_failed_robots();

        assert_eq!(repaired, 2);
        assert_eq!(
            state.sim.robots.iter().find(|r| r.id == ids[0]).unwrap().status,
            RobotStatus::Repairing { remaining_ticks: REPAIR_TICKS }
        );
        assert_eq!(
            state.sim.robots.iter().find(|r| r.id == ids[1]).unwrap().status,
            RobotStatus::Operational,
            "an already-Operational robot must be left alone"
        );
        assert_eq!(
            state.sim.robots.iter().find(|r| r.id == ids[2]).unwrap().status,
            RobotStatus::Repairing { remaining_ticks: REPAIR_TICKS }
        );
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
        let id = helper_robots(&state)[0].id;
        state.sim.robots.iter_mut().find(|r| r.id == id).unwrap().status = RobotStatus::Failed;

        state.select_robot(id).unwrap();

        assert_eq!(state.selected_robot, Some(id));
    }

    #[test]
    fn set_robot_count_shrink_removes_highest_id_even_if_it_is_repairing() {
        // 스펙의 명시적 v1 결정: 상태 인지 제거 우선순위는 두지 않는다 —
        // 복구 중인 로봇도 ID가 가장 크면 그대로 제거 대상이다.
        let mut state = empty_state();
        state.set_robot_count(2);
        let highest_id = helper_robots(&state).iter().map(|r| r.id).max().unwrap();
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
