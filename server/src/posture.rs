use crate::ik::Point2;
use crate::sim::BodyPose;

/// 월드 공간 목표(예: "이 컨베이어 슬롯의 높이")를 `solve_two_bone_ik`가
/// 기대하는 몸체 로컬 좌표로 바꾼다. 몸체 자세와 팔 IK가 각자 따로
/// 설계되지 않도록 하는 유일한 변환 지점 — 웅크리면 어깨가 낮아지므로,
/// 같은 월드 목표라도 몸체 로컬 y가 달라진다.
pub fn world_target_to_body_local(world_target_height: f32, world_target_forward: f32, pose: BodyPose) -> Point2 {
    Point2 { x: world_target_forward, y: world_target_height - pose.shoulder_height() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crouching_raises_the_target_in_body_local_space() {
        let target_height = 0.6;
        let standing = world_target_to_body_local(target_height, 0.5, BodyPose::Standing);
        let crouching = world_target_to_body_local(target_height, 0.5, BodyPose::Crouching);
        assert!(
            crouching.y > standing.y,
            "고정된 월드 목표는 웅크린 상태에서 몸체 로컬 기준 더 높게 보여야 한다"
        );
    }

    #[test]
    fn forward_offset_is_unaffected_by_posture() {
        let standing = world_target_to_body_local(0.6, 0.5, BodyPose::Standing);
        let crouching = world_target_to_body_local(0.6, 0.5, BodyPose::Crouching);
        assert_eq!(standing.x, crouching.x);
    }
}
