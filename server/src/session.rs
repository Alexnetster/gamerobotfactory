use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const RECONNECT_GRACE_PERIOD: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
struct SessionEntry {
    last_seen: Instant,
}

/// 활성 세션들을 추적해, 유예시간 내 재접속인지 판단한다. 순수 로직만
/// 담당하고(시간은 주입받은 `Instant` 기준), 실제 소켓/네트워킹은 모른다.
#[derive(Debug, Default)]
pub struct SessionRegistry {
    sessions: HashMap<Uuid, SessionEntry>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        SessionRegistry { sessions: HashMap::new() }
    }

    pub fn start_session(&mut self, now: Instant) -> Uuid {
        let id = Uuid::new_v4();
        self.sessions.insert(id, SessionEntry { last_seen: now });
        id
    }

    pub fn touch(&mut self, id: Uuid, now: Instant) {
        if let Some(entry) = self.sessions.get_mut(&id) {
            entry.last_seen = now;
        }
    }

    /// `id`가 아직 유예시간 내에 있으면 `true`(재접속 시 델타 기준선을
    /// 이어갈 수 있다는 뜻), 만료됐거나 존재한 적 없으면 `false`.
    pub fn is_within_grace_period(&self, id: Uuid, now: Instant) -> bool {
        match self.sessions.get(&id) {
            Some(entry) => now.duration_since(entry.last_seen) < RECONNECT_GRACE_PERIOD,
            None => false,
        }
    }

    pub fn evict_expired(&mut self, now: Instant) {
        self.sessions.retain(|_, entry| now.duration_since(entry.last_seen) < RECONNECT_GRACE_PERIOD);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_session_is_within_grace_period() {
        let mut registry = SessionRegistry::new();
        let now = Instant::now();
        let id = registry.start_session(now);
        assert!(registry.is_within_grace_period(id, now));
    }

    #[test]
    fn session_expires_after_grace_period() {
        let mut registry = SessionRegistry::new();
        let now = Instant::now();
        let id = registry.start_session(now);
        let later = now + RECONNECT_GRACE_PERIOD + Duration::from_secs(1);
        assert!(!registry.is_within_grace_period(id, later));
    }

    #[test]
    fn touch_extends_the_grace_period() {
        let mut registry = SessionRegistry::new();
        let now = Instant::now();
        let id = registry.start_session(now);

        let mid = now + Duration::from_secs(20);
        registry.touch(id, mid);

        let later = mid + Duration::from_secs(20); // 40s after start, but only 20s after touch
        assert!(registry.is_within_grace_period(id, later));
    }

    #[test]
    fn unknown_session_is_never_within_grace_period() {
        let registry = SessionRegistry::new();
        assert!(!registry.is_within_grace_period(Uuid::new_v4(), Instant::now()));
    }

    #[test]
    fn evict_expired_removes_only_stale_sessions() {
        let mut registry = SessionRegistry::new();
        let now = Instant::now();
        let fresh = registry.start_session(now);
        let stale = registry.start_session(now - RECONNECT_GRACE_PERIOD - Duration::from_secs(1));

        registry.evict_expired(now);

        assert!(registry.is_within_grace_period(fresh, now));
        assert!(!registry.is_within_grace_period(stale, now));
    }
}
