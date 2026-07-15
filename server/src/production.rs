use crate::sim::Robot;
use std::collections::HashMap;

/// 이번 틱 총 생산량. 로봇 ID 오름차순으로 고정해서 합산한다 — 이 틱을
/// 계산하기 전에 로봇 갱신이 어떤 순서로 병렬화됐든, 부동소수점 합산
/// 순서가 항상 같아서 결과가 실행마다 재현 가능하다.
pub fn total_production(robots: &[Robot], units_per_robot: &HashMap<u32, f32>) -> f32 {
    let mut ids: Vec<u32> = robots.iter().map(|r| r.id).collect();
    ids.sort_unstable();

    ids.iter().map(|id| units_per_robot.get(id).copied().unwrap_or(0.0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::Robot;

    #[test]
    fn sums_in_ascending_id_order_regardless_of_input_order() {
        let robots_a = vec![Robot::new(3, (0, 0), (0, 0)), Robot::new(1, (0, 0), (0, 0))];
        let robots_b = vec![Robot::new(1, (0, 0), (0, 0)), Robot::new(3, (0, 0), (0, 0))];

        let mut units = HashMap::new();
        units.insert(1, 0.1_f32);
        units.insert(3, 0.2_f32);

        assert_eq!(total_production(&robots_a, &units), total_production(&robots_b, &units));
    }

    #[test]
    fn missing_robot_contributes_zero() {
        let robots = vec![Robot::new(5, (0, 0), (0, 0))];
        let units = HashMap::new();
        assert_eq!(total_production(&robots, &units), 0.0);
    }
}
