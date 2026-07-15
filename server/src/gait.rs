/// 다리 4개: 앞왼쪽/앞오른쪽/뒤왼쪽/뒤오른쪽.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegId {
    FrontLeft,
    FrontRight,
    BackLeft,
    BackRight,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LegPhase {
    Stance, // 발이 땅을 딛고 체중을 지지
    Swing,  // 발이 들려 앞으로 이동 중
}

fn diagonal_offset(leg: LegId) -> f32 {
    match leg {
        LegId::FrontLeft | LegId::BackRight => 0.0,
        LegId::FrontRight | LegId::BackLeft => 0.5,
    }
}

/// 트롯 걸음: 대각선 다리 쌍(앞왼쪽+뒤오른쪽, 앞오른쪽+뒤왼쪽)이 반 주기
/// 어긋나 함께 움직인다. `cycle_progress`는 [0.0, 1.0) 범위이며 로봇이
/// 이동 중일 때만 전진한다(정지 중엔 고정). 듀티 팩터를 스윙 40% /
/// 스탠스 60%로 둬서 항상 최소 한 쌍은 접지 상태를 유지한다 — 이게
/// 없으면 발이 미끄러지듯 보인다.
pub fn leg_phase(leg: LegId, cycle_progress: f32) -> LegPhase {
    let local = (cycle_progress + diagonal_offset(leg)).rem_euclid(1.0);
    if local < 0.4 { LegPhase::Swing } else { LegPhase::Stance }
}

/// 렌더링용 발 들림 높이. 접지 중엔 0.0, 스윙 구간 중간에 `lift`만큼
/// 올라갔다가 착지 직전 다시 0.0으로 돌아온다.
pub fn foot_lift(leg: LegId, cycle_progress: f32, lift: f32) -> f32 {
    let local = (cycle_progress + diagonal_offset(leg)).rem_euclid(1.0);
    if local < 0.4 {
        let swing_progress = local / 0.4;
        lift * (swing_progress * std::f32::consts::PI).sin()
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagonal_pairs_share_phase() {
        let progress = 0.1;
        assert_eq!(leg_phase(LegId::FrontLeft, progress), leg_phase(LegId::BackRight, progress));
        assert_eq!(leg_phase(LegId::FrontRight, progress), leg_phase(LegId::BackLeft, progress));
    }

    #[test]
    fn opposite_diagonals_are_out_of_phase() {
        let progress = 0.1;
        assert_ne!(leg_phase(LegId::FrontLeft, progress), leg_phase(LegId::FrontRight, progress));
    }

    #[test]
    fn at_least_one_diagonal_pair_is_always_grounded() {
        for i in 0..100 {
            let progress = i as f32 / 100.0;
            let fl_grounded = leg_phase(LegId::FrontLeft, progress) == LegPhase::Stance;
            let fr_grounded = leg_phase(LegId::FrontRight, progress) == LegPhase::Stance;
            assert!(fl_grounded || fr_grounded, "progress {progress}에서 두 대각쌍이 동시에 떠있음");
        }
    }

    #[test]
    fn planted_foot_has_zero_lift() {
        assert_eq!(foot_lift(LegId::FrontLeft, 0.5, 0.1), 0.0);
    }

    #[test]
    fn swinging_foot_lifts_off_the_ground() {
        assert!(foot_lift(LegId::FrontLeft, 0.1, 0.1) > 0.0);
    }
}
