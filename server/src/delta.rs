use crate::protocol::{ConveyorView, ProductView, RobotView, ServerMessage, StationView, PROTOCOL_VERSION};

/// `previous`(이 클라이언트에게 마지막으로 보낸 스냅샷)와 `current`를
/// 비교해, 바뀐 로봇/제품만 담긴 델타 메시지를 만든다. 유휴 상태로
/// 멈춰있는 로봇/제품은 매 틱 다시 보내지 않아도 되므로 대역폭을 아낀다.
/// 스테이션(`STATION_COUNT`개뿐)은 매번 풀 목록을 싣는다(설계문서 §8).
#[allow(clippy::too_many_arguments)]
pub fn compute_delta(
    previous_conveyor: ConveyorView,
    previous_robots: &[RobotView],
    previous_products: &[ProductView],
    current_tick: u64,
    current_conveyor: ConveyorView,
    current_robots: &[RobotView],
    current_stations: &[StationView],
    current_products: &[ProductView],
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

    let changed_products: Vec<ProductView> = current_products
        .iter()
        .filter(|current| !previous_products.iter().any(|prev| prev == *current))
        .cloned()
        .collect();

    let removed_product_ids: Vec<u32> = previous_products
        .iter()
        .filter(|prev| !current_products.iter().any(|current| current.id == prev.id))
        .map(|prev| prev.id)
        .collect();

    ServerMessage::Delta {
        v: PROTOCOL_VERSION,
        tick: current_tick,
        conveyor,
        changed_robots,
        removed_robot_ids,
        stations: current_stations.to_vec(),
        changed_products,
        removed_product_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{WireArmPose, WireCellId, WireDirection, WireRobotRole, WireStatus, WireTask};
    use sim_core::sim::BodyPose;

    fn robot_view(id: u32, x: i32) -> RobotView {
        RobotView {
            id,
            pos: WireCellId { x, y: 0 },
            pose: BodyPose::Standing.into(),
            leg_cycle_progress: 0.0,
            task: WireTask::Idle,
            status: WireStatus::Operational,
            durability_remaining: 1.0,
            path: Vec::new(),
            facing: WireDirection::East,
            arm_pose: WireArmPose { shoulder_angle: 0.0, elbow_angle: 0.0 },
            carrying: false,
            role: WireRobotRole::Helper,
        }
    }

    fn product_view(id: u32, x: i32, stage: u8) -> ProductView {
        ProductView { id, stage, pos: WireCellId { x, y: 0 } }
    }

    #[test]
    fn unchanged_robots_are_omitted_from_delta() {
        let prev = vec![robot_view(1, 0)];
        let curr = vec![robot_view(1, 0)];

        let msg = compute_delta(ConveyorView { running: true }, &prev, &[], 1, ConveyorView { running: true }, &curr, &[], &[]);

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

        let msg = compute_delta(ConveyorView { running: true }, &prev, &[], 1, ConveyorView { running: true }, &curr, &[], &[]);

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

        let msg = compute_delta(ConveyorView { running: true }, &prev, &[], 1, ConveyorView { running: true }, &curr, &[], &[]);

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

        let msg = compute_delta(ConveyorView { running: true }, &prev, &[], 1, ConveyorView { running: true }, &curr, &[], &[]);

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
        let msg = compute_delta(ConveyorView { running: true }, &[], &[], 1, ConveyorView { running: false }, &[], &[], &[]);
        match msg {
            ServerMessage::Delta { conveyor, .. } => assert_eq!(conveyor, Some(ConveyorView { running: false })),
            _ => panic!("expected Delta"),
        }
    }

    #[test]
    fn changed_product_is_included_and_unchanged_is_omitted() {
        let prev_products = vec![product_view(1, 0, 0), product_view(2, 5, 1)];
        let curr_products = vec![product_view(1, 1, 0), product_view(2, 5, 1)];

        let msg = compute_delta(
            ConveyorView { running: true },
            &[],
            &prev_products,
            1,
            ConveyorView { running: true },
            &[],
            &[],
            &curr_products,
        );

        match msg {
            ServerMessage::Delta { changed_products, removed_product_ids, .. } => {
                assert_eq!(changed_products, vec![product_view(1, 1, 0)]);
                assert!(removed_product_ids.is_empty());
            }
            _ => panic!("expected Delta"),
        }
    }

    #[test]
    fn removed_product_id_is_reported() {
        let prev_products = vec![product_view(1, 0, 0)];

        let msg = compute_delta(
            ConveyorView { running: true },
            &[],
            &prev_products,
            1,
            ConveyorView { running: true },
            &[],
            &[],
            &[],
        );

        match msg {
            ServerMessage::Delta { removed_product_ids, .. } => assert_eq!(removed_product_ids, vec![1]),
            _ => panic!("expected Delta"),
        }
    }
}
