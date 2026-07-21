use crate::game_state::{Conveyor, GameState};
use serde::{Deserialize, Serialize};
use sim_core::grid::CellId;
use sim_core::ik::solve_two_bone_ik;
use sim_core::posture::world_target_to_body_local;
use sim_core::sim::{BodyPose, Direction, Robot, RobotStatus, Task};

pub const PROTOCOL_VERSION: u8 = 1;

/// 클라이언트 → 서버 커맨드. `#[serde(tag = "type")]`로 JSON에서
/// `{"type": "ToggleConveyor"}` 같은 식으로 태그가 붙는다.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ClientCommand {
    SelectRobot { robot_id: u32 },
    ReleaseRobot,
    ToggleConveyor,
    SetRobotCount { count: usize },
    TriggerArmAction { robot_id: u32, task: WireTask },
    RepairRobot { robot_id: u32 },
    /// 유예시간 내 재접속인지 확인만 하는 순수 검증 커맨드다. 매 연결마다
    /// 항상 새 세션이 발급되고(핸드셰이크 시점에 이미 스냅샷과 함께
    /// 나간다), 델타 기준선이 전역 공유이므로 `Resume`이 서버 쪽에서
    /// 뭔가를 되살리거나 병합하는 것은 아니다 — `resumed: true/false`
    /// 응답 외에는 클라이언트가 받는 것이 Resume을 안 보냈을 때와 동일하다.
    Resume { session_id: uuid::Uuid },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WireTask {
    Idle,
    Picking,
    Placing,
}

impl From<WireTask> for Task {
    fn from(t: WireTask) -> Task {
        match t {
            WireTask::Idle => Task::Idle,
            WireTask::Picking => Task::Picking,
            WireTask::Placing => Task::Placing,
        }
    }
}

impl From<Task> for WireTask {
    fn from(t: Task) -> WireTask {
        match t {
            Task::Idle => WireTask::Idle,
            Task::Picking => WireTask::Picking,
            Task::Placing => WireTask::Placing,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WirePose {
    Standing,
    Crouching,
}

impl From<BodyPose> for WirePose {
    fn from(p: BodyPose) -> WirePose {
        match p {
            BodyPose::Standing => WirePose::Standing,
            BodyPose::Crouching => WirePose::Crouching,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WireDirection {
    North,
    East,
    South,
    West,
}

impl From<Direction> for WireDirection {
    fn from(d: Direction) -> WireDirection {
        match d {
            Direction::North => WireDirection::North,
            Direction::East => WireDirection::East,
            Direction::South => WireDirection::South,
            Direction::West => WireDirection::West,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum WireStatus {
    Operational,
    Failed,
    Repairing { remaining_ticks: u32 },
}

impl From<RobotStatus> for WireStatus {
    fn from(s: RobotStatus) -> WireStatus {
        match s {
            RobotStatus::Operational => WireStatus::Operational,
            RobotStatus::Failed => WireStatus::Failed,
            RobotStatus::Repairing { remaining_ticks } => WireStatus::Repairing { remaining_ticks },
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct WireCellId {
    pub x: i32,
    pub y: i32,
}

impl From<CellId> for WireCellId {
    fn from(pos: CellId) -> WireCellId {
        WireCellId { x: pos.0, y: pos.1 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RobotView {
    pub id: u32,
    pub pos: WireCellId,
    pub pose: WirePose,
    pub leg_cycle_progress: f32,
    pub task: WireTask,
    pub status: WireStatus,
    pub durability_remaining: f32,
    pub path: Vec<WireCellId>,
    pub facing: WireDirection,
    pub arm_pose: WireArmPose,
    pub carrying: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct WireArmPose {
    pub shoulder_angle: f32,
    pub elbow_angle: f32,
}

// 아래 네 상수는 클라이언트(`client/src/render/projection.ts`, 나중 태스크에서
// 작성됨)에도 그대로 미러링해서 유지해야 한다 — 와이어로 안 보내는 이유는
// 안 바뀌는 튜닝 상수를 매 메시지에 싣는 게 낭비이기 때문(설계문서 참고).
const WORK_TARGET_HEIGHT: f32 = 0.75;
const WORK_TARGET_FORWARD: f32 = 0.6;
const UPPER_ARM_LEN: f32 = 0.7;
const LOWER_ARM_LEN: f32 = 0.6;
const IDLE_ARM_POSE: WireArmPose = WireArmPose { shoulder_angle: 0.0, elbow_angle: 0.0 };

/// `task`만 보고 자세(Standing/Crouching)를 결정한다 — 컨베이어 칸별
/// 높이 같은 위치 기반 데이터는 시뮬레이션에 없고(설계문서 "서버 쪽 변경"
/// 절 참고), 그런 걸 새로 만들 필요도 없다고 확인됐다.
fn pose_for(task: Task) -> BodyPose {
    if task == Task::Idle {
        BodyPose::Standing
    } else {
        BodyPose::Crouching
    }
}

fn arm_pose_for(robot: &Robot) -> WireArmPose {
    if robot.task == Task::Idle {
        return IDLE_ARM_POSE;
    }
    let local_target = world_target_to_body_local(WORK_TARGET_HEIGHT, WORK_TARGET_FORWARD, pose_for(robot.task));
    let solved = solve_two_bone_ik(UPPER_ARM_LEN, LOWER_ARM_LEN, local_target);
    WireArmPose { shoulder_angle: solved.shoulder_angle, elbow_angle: solved.elbow_angle }
}

impl From<&Robot> for RobotView {
    fn from(r: &Robot) -> RobotView {
        RobotView {
            id: r.id,
            pos: r.pos.into(),
            pose: pose_for(r.task).into(),
            leg_cycle_progress: r.leg_cycle_progress,
            task: r.task.into(),
            status: r.status.into(),
            durability_remaining: quantize_durability(r.wear_ratio()),
            path: r.path.iter().map(|&c| c.into()).collect(),
            facing: r.facing.into(),
            arm_pose: arm_pose_for(r),
            carrying: r.carrying,
        }
    }
}

/// 델타 압축과의 상호작용(설계문서 참고) — 원값을 그대로 실으면 작업
/// 중인 로봇은 매 틱 델타에 실려서 "안 바뀌면 안 보낸다"는 대역폭 절약이
/// 무력화된다. 5% 단위로 반올림해서 값이 바뀌는 빈도를 낮춘다(약 100틱에
/// 한 번). `Repairing`의 `remaining_ticks`는 반대로 일부러 반올림하지
/// 않는다 — 진행률 표시로서의 가치가 크고, 최대 100틱(5초)으로 짧아서
/// 대역폭 비용이 무시할 만하다(설계문서 참고).
fn quantize_durability(wear_ratio: f32) -> f32 {
    let durability = 1.0 - wear_ratio;
    (durability * 20.0).round() / 20.0
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ConveyorView {
    pub running: bool,
}

impl From<Conveyor> for ConveyorView {
    fn from(c: Conveyor) -> ConveyorView {
        ConveyorView { running: c.running }
    }
}

/// 서버 → 클라이언트 메시지. `v` 필드로 프로토콜 확장성을 명시적으로
/// 남겨둔다(설계문서 참고) — 지금은 항상 1이지만, 나중에 필드가 늘어나도
/// 이 필드 하나로 하위 호환을 다룰 수 있다.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum ServerMessage {
    Snapshot { v: u8, tick: u64, session_id: uuid::Uuid, conveyor: ConveyorView, robots: Vec<RobotView> },
    Delta { v: u8, tick: u64, conveyor: Option<ConveyorView>, changed_robots: Vec<RobotView>, removed_robot_ids: Vec<u32> },
    ResumeAck { v: u8, session_id: uuid::Uuid, resumed: bool },
}

pub fn to_snapshot(state: &GameState, session_id: uuid::Uuid) -> ServerMessage {
    ServerMessage::Snapshot {
        v: PROTOCOL_VERSION,
        tick: state.sim.tick_count,
        session_id,
        conveyor: state.conveyor.into(),
        robots: state.sim.robots.iter().map(RobotView::from).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_command_deserializes_from_tagged_json() {
        let json = r#"{"type":"ToggleConveyor"}"#;
        let cmd: ClientCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd, ClientCommand::ToggleConveyor);

        let json = r#"{"type":"SelectRobot","robot_id":7}"#;
        let cmd: ClientCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd, ClientCommand::SelectRobot { robot_id: 7 });

        let json = r#"{"type":"TriggerArmAction","robot_id":3,"task":"Picking"}"#;
        let cmd: ClientCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd, ClientCommand::TriggerArmAction { robot_id: 3, task: WireTask::Picking });
    }

    #[test]
    fn client_command_round_trips_through_json() {
        let cmd = ClientCommand::TriggerArmAction { robot_id: 3, task: WireTask::Placing };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: ClientCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(cmd, back);
    }

    #[test]
    fn repair_robot_command_round_trips_through_json() {
        let cmd = ClientCommand::RepairRobot { robot_id: 5 };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: ClientCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(cmd, back);
    }

    #[test]
    fn robot_view_reports_operational_status_by_default() {
        use sim_core::sim::Robot;
        let robot = Robot::new(1, (0, 0), (0, 0));
        let view = RobotView::from(&robot);
        assert_eq!(view.status, WireStatus::Operational);
        assert_eq!(view.durability_remaining, 1.0);
    }

    #[test]
    fn robot_view_quantizes_durability_to_the_nearest_five_percent() {
        use sim_core::sim::Robot;
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        // wear_ratio = 3300/6000 = 0.55 -> raw durability_remaining 0.45,
        // 이미 5%의 배수라 반올림 여부와 무관하게 정확히 0.45가 나와야 한다.
        // (WEAR_LIMIT_TICKS가 2000->6000으로 튜닝되면서 3300으로 갱신됨.)
        robot.worn_ticks = 3300;

        let view = RobotView::from(&robot);

        assert_eq!(view.durability_remaining, 0.45);
    }

    #[test]
    fn robot_view_reports_repairing_status_with_remaining_ticks() {
        use sim_core::sim::{Robot, RobotStatus};
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        robot.status = RobotStatus::Repairing { remaining_ticks: 42 };

        let view = RobotView::from(&robot);

        assert_eq!(view.status, WireStatus::Repairing { remaining_ticks: 42 });
    }

    #[test]
    fn robot_view_reports_path_as_wire_cells() {
        use sim_core::sim::Robot;
        let mut robot = Robot::new(1, (0, 0), (5, 0));
        robot.path = vec![(1, 0), (2, 0)];

        let view = RobotView::from(&robot);

        assert_eq!(view.path, vec![WireCellId { x: 1, y: 0 }, WireCellId { x: 2, y: 0 }]);
    }

    #[test]
    fn robot_view_reports_facing() {
        use sim_core::sim::{Direction, Robot};
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        robot.facing = Direction::North;

        let view = RobotView::from(&robot);

        assert_eq!(view.facing, WireDirection::North);
    }

    #[test]
    fn robot_view_pose_is_standing_when_idle_and_crouching_while_working() {
        use sim_core::sim::{Robot, Task};
        let idle = Robot::new(1, (0, 0), (0, 0));
        assert_eq!(RobotView::from(&idle).pose, WirePose::Standing);

        let mut working = Robot::new(2, (0, 0), (0, 0));
        working.task = Task::Picking;
        assert_eq!(RobotView::from(&working).pose, WirePose::Crouching);
    }

    #[test]
    fn robot_view_arm_pose_is_idle_rest_when_task_is_idle() {
        use sim_core::sim::Robot;
        let robot = Robot::new(1, (0, 0), (0, 0));
        assert_eq!(RobotView::from(&robot).arm_pose, IDLE_ARM_POSE);
    }

    #[test]
    fn robot_view_arm_pose_is_solved_via_ik_while_working() {
        use sim_core::sim::{Robot, Task};
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        robot.task = Task::Picking;

        let view = RobotView::from(&robot);

        assert_ne!(view.arm_pose, IDLE_ARM_POSE, "작업 중이면 대기 자세가 아니라 실제 IK 해가 나와야 한다");
        assert!(view.arm_pose.shoulder_angle.is_finite());
        assert!(view.arm_pose.elbow_angle.is_finite());
    }

    #[test]
    fn robot_view_arm_pose_is_stable_when_task_and_facing_are_unchanged() {
        // task/facing이 같으면 다른 로봇(다른 id/위치)이어도 arm_pose가
        // 완전히 같아야 한다 — compute_delta의 PartialEq 비교가 이 필드를
        // 델타에서 제대로 걸러내는지(대역폭 회귀 방지)의 전제조건.
        use sim_core::sim::{Robot, Task};
        let mut a = Robot::new(1, (0, 0), (0, 0));
        a.task = Task::Picking;
        let mut b = Robot::new(2, (5, 5), (5, 5));
        b.task = Task::Picking;

        assert_eq!(RobotView::from(&a).arm_pose, RobotView::from(&b).arm_pose);
    }

    #[test]
    fn server_message_round_trips_through_json() {
        let msg = ServerMessage::Snapshot {
            v: 1,
            tick: 42,
            session_id: uuid::Uuid::nil(),
            conveyor: ConveyorView { running: true },
            robots: vec![],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn to_snapshot_reflects_current_game_state() {
        use crate::game_state::GameState;
        use sim_core::grid::Grid;
        use sim_core::sim::SimState;
        use std::sync::Arc;

        let mut state = GameState::new(SimState { grid: Arc::new(Grid::new(3, 3)), robots: Vec::new(), tick_count: 5 });
        state.set_robot_count(2);
        state.toggle_conveyor();

        let snapshot = to_snapshot(&state, uuid::Uuid::nil());
        match snapshot {
            ServerMessage::Snapshot { v, tick, conveyor, robots, .. } => {
                assert_eq!(v, PROTOCOL_VERSION);
                assert_eq!(tick, 5);
                assert!(!conveyor.running);
                assert_eq!(robots.len(), 2);
            }
            _ => panic!("expected Snapshot"),
        }
    }
}
