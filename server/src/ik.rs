/// 팔 끝(손목) 목표 위치 — 몸체 로컬 좌표(원점 = 어깨, x = 전방, y = 위).
/// 몸체가 웅크리거나 서도 이 숫자 자체는 바뀌지 않는다 — 어깨의 월드
/// 트랜스폼만 바뀌고, 최종 월드 포즈는 "몸체 트랜스폼 * 이 팔 로컬
/// 포즈"로 합성된다. 이게 몸체 자세와 무관하게 팔이 어깨에 붙어있도록
/// 보장하는 지점이다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct ArmPose {
    pub shoulder_angle: f32,
    pub elbow_angle: f32,
}

/// 어깨-팔꿈치-손목 2-본 해석적 IK. `target`이 팔이 닿을 수 있는 최대
/// 거리보다 멀면, 같은 방향을 유지한 채 최대 도달 거리로 클램프한
/// 뒤 푼다 — 그래서 목표가 너무 멀어도 팔은 최소한 그 방향을 향한다.
pub fn solve_two_bone_ik(upper_len: f32, lower_len: f32, target: Point2) -> ArmPose {
    let max_reach = upper_len + lower_len;
    let min_reach = (upper_len - lower_len).abs();
    let dist = (target.x * target.x + target.y * target.y).sqrt();

    let clamped_dist = dist.clamp(min_reach.max(0.001), max_reach - 0.001);
    let scale = if dist > 0.0 { clamped_dist / dist } else { 1.0 };
    let target = Point2 { x: target.x * scale, y: target.y * scale };
    let dist = clamped_dist;

    let cos_elbow = (upper_len.powi(2) + lower_len.powi(2) - dist.powi(2)) / (2.0 * upper_len * lower_len);
    let elbow_angle = std::f32::consts::PI - cos_elbow.clamp(-1.0, 1.0).acos();

    let angle_to_target = target.y.atan2(target.x);
    let cos_shoulder_offset = (upper_len.powi(2) + dist.powi(2) - lower_len.powi(2)) / (2.0 * upper_len * dist);
    let shoulder_offset = cos_shoulder_offset.clamp(-1.0, 1.0).acos();
    let shoulder_angle = angle_to_target - shoulder_offset;

    ArmPose { shoulder_angle, elbow_angle }
}

/// 주어진 포즈에서 손목이 실제로 어디에 있는지 계산한다 — 테스트와
/// 렌더러가 각도를 다시 좌표로 바꿀 때 쓴다.
pub fn forward_kinematics(upper_len: f32, lower_len: f32, pose: ArmPose) -> Point2 {
    let elbow = Point2 {
        x: upper_len * pose.shoulder_angle.cos(),
        y: upper_len * pose.shoulder_angle.sin(),
    };
    let wrist_angle = pose.shoulder_angle + pose.elbow_angle;
    Point2 { x: elbow.x + lower_len * wrist_angle.cos(), y: elbow.y + lower_len * wrist_angle.sin() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(a: f32, b: f32) {
        assert!((a - b).abs() < 0.01, "{a} != {b}");
    }

    #[test]
    fn reaches_target_within_arm_length() {
        let target = Point2 { x: 1.0, y: 0.0 };
        let pose = solve_two_bone_ik(0.7, 0.6, target);
        let result = forward_kinematics(0.7, 0.6, pose);
        assert_close(result.x, target.x);
        assert_close(result.y, target.y);
    }

    #[test]
    fn clamps_target_beyond_max_reach() {
        let far_target = Point2 { x: 10.0, y: 0.0 };
        let pose = solve_two_bone_ik(0.7, 0.6, far_target);
        let result = forward_kinematics(0.7, 0.6, pose);
        let reached_dist = (result.x.powi(2) + result.y.powi(2)).sqrt();
        assert!(reached_dist <= 0.7 + 0.6 + 0.01);
        assert!(result.x > 0.0);
    }

    #[test]
    fn handles_target_at_origin_without_panicking() {
        let pose = solve_two_bone_ik(0.7, 0.6, Point2 { x: 0.0, y: 0.0 });
        assert!(pose.shoulder_angle.is_finite());
        assert!(pose.elbow_angle.is_finite());
    }
}
