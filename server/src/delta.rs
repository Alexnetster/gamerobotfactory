use crate::protocol::{ConveyorView, RobotView, ServerMessage, PROTOCOL_VERSION};

/// `previous`(이 클라이언트에게 마지막으로 보낸 스냅샷)와 `current`를
/// 비교해, 바뀐 로봇만 담긴 델타 메시지를 만든다. 유휴 상태로 멈춰있는
/// 로봇은 매 틱 다시 보내지 않아도 되므로 대역폭을 아낀다.
pub fn compute_delta(
    previous_conveyor: ConveyorView,
    previous_robots: &[RobotView],
    current_tick: u64,
    current_conveyor: ConveyorView,
    current_robots: &[RobotView],
) -> ServerMessage {
    let conveyor = if previous_conveyor == current_conveyor { None } else { Some(current_conveyor) };

    let changed_robots: Vec<RobotView> = current_robots
        .iter()
        .filter(|current| {
            let unchanged = previous_robots.iter().any(|prev| prev == *current);
            !unchanged
        })
        .cloned()
        .collect();

    let removed_robot_ids: Vec<u32> = previous_robots
        .iter()
        .filter(|prev| !current_robots.iter().any(|current| current.id == prev.id))
        .map(|prev| prev.id)
        .collect();

    ServerMessage::Delta { v: PROTOCOL_VERSION, tick: current_tick, conveyor, changed_robots, removed_robot_ids }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{WireCellId, WireTask};
    use sim_core::sim::BodyPose;

    fn robot_view(id: u32, x: i32) -> RobotView {
        RobotView {
            id,
            pos: WireCellId { x, y: 0 },
            pose: BodyPose::Standing.into(),
            leg_cycle_progress: 0.0,
            task: WireTask::Idle,
        }
    }

    #[test]
    fn unchanged_robots_are_omitted_from_delta() {
        let prev = vec![robot_view(1, 0)];
        let curr = vec![robot_view(1, 0)];

        let msg = compute_delta(ConveyorView { running: true }, &prev, 1, ConveyorView { running: true }, &curr);

        match msg {
            ServerMessage::Delta { conveyor, changed_robots, removed_robot_ids, .. } => {
                assert!(conveyor.is_none());
                assert!(changed_robots.is_empty());
                assert!(removed_robot_ids.is_empty());
            }
            _ => panic!("expected Delta"),
        }
    }

    #[test]
    fn moved_robot_is_included_in_delta() {
        let prev = vec![robot_view(1, 0)];
        let curr = vec![robot_view(1, 1)];

        let msg = compute_delta(ConveyorView { running: true }, &prev, 1, ConveyorView { running: true }, &curr);

        match msg {
            ServerMessage::Delta { changed_robots, .. } => {
                assert_eq!(changed_robots, vec![robot_view(1, 1)]);
            }
            _ => panic!("expected Delta"),
        }
    }

    #[test]
    fn removed_robot_id_is_reported() {
        let prev = vec![robot_view(1, 0), robot_view(2, 0)];
        let curr = vec![robot_view(1, 0)];

        let msg = compute_delta(ConveyorView { running: true }, &prev, 1, ConveyorView { running: true }, &curr);

        match msg {
            ServerMessage::Delta { removed_robot_ids, changed_robots, .. } => {
                assert_eq!(removed_robot_ids, vec![2]);
                assert!(changed_robots.is_empty());
            }
            _ => panic!("expected Delta"),
        }
    }

    #[test]
    fn new_robot_is_included_in_delta() {
        let prev = vec![robot_view(1, 0)];
        let curr = vec![robot_view(1, 0), robot_view(2, 5)];

        let msg = compute_delta(ConveyorView { running: true }, &prev, 1, ConveyorView { running: true }, &curr);

        match msg {
            ServerMessage::Delta { changed_robots, removed_robot_ids, .. } => {
                assert_eq!(changed_robots, vec![robot_view(2, 5)], "only the newly-added robot should appear");
                assert!(removed_robot_ids.is_empty());
            }
            _ => panic!("expected Delta"),
        }
    }

    #[test]
    fn conveyor_change_is_reported_only_when_it_changed() {
        let msg = compute_delta(ConveyorView { running: true }, &[], 1, ConveyorView { running: false }, &[]);
        match msg {
            ServerMessage::Delta { conveyor, .. } => assert_eq!(conveyor, Some(ConveyorView { running: false })),
            _ => panic!("expected Delta"),
        }
    }
}
