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
/// `upper_len`/`lower_len`은 0.01 미만이면 0.01로 올림 처리한다 — 그렇지
/// 않으면 두 뼈 길이가 거의 같거나 극단적으로 짧을 때 클램프 구간
/// (`min_reach..=max_reach`)이 뒤집혀 패닉할 수 있다. `target`이 정확히
/// 어깨 위치(거리 0)에 있으면 방향을 정의할 수 없으므로, 몸체 로컬 +x
/// (전방)를 향하는 것으로 정한다.
pub fn solve_two_bone_ik(upper_len: f32, lower_len: f32, target: Point2) -> ArmPose {
    let upper_len = upper_len.max(0.01);
    let lower_len = lower_len.max(0.01);

    let max_reach = upper_len + lower_len;
    let min_reach = (upper_len - lower_len).abs();
    let epsilon = (max_reach * 0.001).max(0.0001);
    let lo = min_reach + epsilon;
    let hi = (max_reach - epsilon).max(lo);

    let dist = (target.x * target.x + target.y * target.y).sqrt();
    let clamped_dist = dist.clamp(lo, hi);

    let target = if dist > 0.0 {
        let scale = clamped_dist / dist;
        Point2 { x: target.x * scale, y: target.y * scale }
    } else {
        Point2 { x: clamped_dist, y: 0.0 }
    };
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

    #[test]
    fn does_not_panic_on_degenerate_bone_lengths() {
        let pose = solve_two_bone_ik(0.0, 0.0, Point2 { x: 1.0, y: 0.0 });
        assert!(pose.shoulder_angle.is_finite());
        assert!(pose.elbow_angle.is_finite());

        let pose = solve_two_bone_ik(0.7, 0.0001, Point2 { x: 0.5, y: 0.0 });
        assert!(pose.shoulder_angle.is_finite());
        assert!(pose.elbow_angle.is_finite());
    }

    #[test]
    fn target_at_origin_with_nonzero_min_reach_points_forward() {
        // upper != lower, so min_reach > 0 and the target-at-origin fallback
        // direction actually matters (arm can't fold to exactly zero length).
        let pose = solve_two_bone_ik(0.7, 0.3, Point2 { x: 0.0, y: 0.0 });
        let result = forward_kinematics(0.7, 0.3, pose);
        assert!(result.x > 0.0, "fallback direction should point along +x (forward)");
        assert!(result.y.abs() < 0.01, "fallback direction should have ~0 y-component");
    }
}
