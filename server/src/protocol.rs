use crate::game_state::{Conveyor, GameState};
use serde::{Deserialize, Serialize};
use sim_core::grid::CellId;
use sim_core::sim::{BodyPose, Robot, Task};

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
}

impl From<&Robot> for RobotView {
    fn from(r: &Robot) -> RobotView {
        RobotView {
            id: r.id,
            pos: r.pos.into(),
            pose: r.pose.into(),
            leg_cycle_progress: r.leg_cycle_progress,
            task: r.task.into(),
        }
    }
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
