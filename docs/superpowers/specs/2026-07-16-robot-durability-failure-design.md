# 로봇 내구도/고장/복구 설계

브레인스토밍 세션(2026-07-16)에서 확정된 설계. 사용시간이 누적될수록 로봇이 고장날 확률이 높아지고, 고장나면 오퍼레이터가 부품을 교체(복구)해야 하는 메커니즘을 추가한다.

## 왜 이 기능인가

이 프로젝트의 1차 목표는 백엔드 엔지니어링 역량 시연이다(설계문서 `docs/robot-arm-conveyor-game-design.md` 참고). 이 기능은 게임성 확장이 아니라, Plan 1~3에서 이미 구축한 **장애 격리/관측가능성 서사를 시뮬레이션 도메인 레벨로 한 겹 더 쌓는 것**이다:

- 인프라 레벨(이미 구현됨): 틱 하나가 패닉해도 서버는 안 죽음(`safe_tick`), 패닉 횟수가 메트릭으로 노출됨(`tick_panics_total`).
- 도메인 레벨(이번 설계): 로봇 하나가 마모로 "고장"나도 다른 로봇/서버는 안 죽음, 고장 발생/복구가 메트릭+이력으로 노출됨.

즉 "장애 감지 → 조치 → 관측"이라는 동일한 패턴을 인프라 장애(패닉)와 도메인 장애(로봇 고장) 양쪽에서 일관되게 보여주는 것이 이 기능의 핵심 가치다.

## 스코프 (v1)

- 로봇 전체에 대한 **단일 내구도 값**만 사용한다(부품별 개별 마모 없음).
- 내구도는 **작업 중(Picking/Placing)일 때만** 깎인다.
- 복구는 **오퍼레이터가 커맨드로 트리거**하고 **고정 틱 수 동안 진행**된다(예비부품 재고 관리 등은 v1 범위 밖).
- 결정성(같은 입력 → 같은 결과)을 반드시 유지한다 — 기존 `sim_core`의 핵심 불변식.

## 데이터 모델

`server/src/sim.rs`의 `Robot` 구조체에 필드 추가:

```rust
pub struct Robot {
    // ...기존 필드(id, pos, path, task, pose, leg_cycle_progress 등)...
    pub worn_ticks: u64,
    pub status: RobotStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotStatus {
    Operational,
    Failed,
    Repairing { remaining_ticks: u32 },
}
```

`task`(Idle/Picking/Placing)와 `status`를 별도 필드로 분리한다. 이유: `task`는 원래 "팔로 무슨 작업을 하는지"만 나타내고 이동은 항상 자동이었다(설계문서 참고) — 고장이 이동까지 멈춰야 하므로 `task`에 끼워넣으면 이동을 막을 자연스러운 훅이 없다. `status`를 따로 두면 "동작 가능 여부"(`status`)와 "무슨 작업을 하려던 참인지"(`task`, Repairing 중에도 보존됨 — 복구 완료 후 하던 일을 잊지 않음)가 깔끔히 분리된다.

새 로봇(`SetRobotCount`로 생성)은 `worn_ticks: 0, status: Operational`로 시작한다.

## 마모 → 고장 확률

매 틱, `status == Operational`이고 `task`가 `Picking`/`Placing`이면:

```rust
robot.worn_ticks += 1;
```

고장 확률(이번 틱에 고장날 확률):

```rust
const WEAR_LIMIT_TICKS: u64 = 2000;   // 100초 분량의 작업(20Hz 기준) — 튜닝 대상
const MAX_FAILURE_PROB: f64 = 0.05;   // 완전 마모 상태에서의 틱당 최대 고장 확률 — 튜닝 대상

let wear_ratio = (robot.worn_ticks as f64 / WEAR_LIMIT_TICKS as f64).min(1.0);
let failure_prob = wear_ratio * MAX_FAILURE_PROB;
```

`worn_ticks`가 `WEAR_LIMIT_TICKS`를 넘어도 확률은 `MAX_FAILURE_PROB`에서 더 안 올라간다(100% 확정 고장은 "확률적" 느낌을 없애므로 상한을 둔다) — 대신 내구도 표시는 0%에서 멈춘다(아래 참고).

### 결정적 "난수"

`sim_core::sim::tick`은 `rayon`으로 로봇들을 병렬 갱신하며, 각 로봇은 틱 시작 시점의 스냅샷만 읽는다(더블 버퍼링, 공유 가변 상태 없음). 상태를 가진 RNG(예: `rand::thread_rng()`)를 쓰면 이 병렬-무공유 불변식이 깨진다. 대신 `(robot_id, tick_count)`를 시드로 한 순수 해시 함수로 `[0.0, 1.0)` 구간의 결정적 값을 뽑는다:

```rust
/// (robot_id, tick_count)를 섞어 [0.0, 1.0) 구간의 결정적 의사난수를 낸다.
/// splitmix64 파이널라이저를 재사용 — 암호학적 강도는 필요 없고, 입력이
/// 조금만 달라져도 출력이 크게 달라지는 성질(avalanche)만 있으면 된다.
fn deterministic_roll(robot_id: u32, tick_count: u64) -> f64 {
    let mut x = (robot_id as u64).wrapping_mul(0x9E3779B97F4A7C15) ^ tick_count.wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^= x >> 31;
    (x as f64) / (u64::MAX as f64)
}
```

`deterministic_roll(robot.id, state.tick_count) < failure_prob`이면 `status = Failed`로 전이한다. 이 함수는 순수 함수(입력만으로 출력이 결정)이므로 로봇별 병렬 계산 중에 호출해도 안전하고, 같은 시뮬레이션을 재생하면 항상 같은 틱에 같은 로봇이 고장난다 — 기존 `tick_is_deterministic` proptest가 이 불변식을 그대로 검증할 수 있다.

## 이동/작업 정지

`Robot::status != Operational`이면 `plan_robot`(이동 로직)과 `TriggerArmAction`(팔 동작) 둘 다 아무 일도 하지 않는다 — 로봇은 현재 칸에 얼어붙는다.

**그리드 장애물 처리는 새 코드가 필요 없다.** `sim_core::sim::tick`은 이미 매 틱 `occupied: HashSet<CellId> = state.robots.iter().map(|r| r.pos).collect()`로 전체 로봇의 현재 위치를 모아 다른 로봇들의 `find_path` 호출에서 장애물로 사용한다(`server/src/sim.rs:84`, `:137-139`). 고장난 로봇은 단지 위치가 안 바뀔 뿐이므로, 다른 로봇들의 A*는 기존 로직 그대로 그 칸을 피해간다.

`TriggerArmAction`을 `status != Operational`인 로봇에 보내면 기존 `CommandError` 패턴을 따라 거부한다(신규 variant, 예: `CommandError::RobotNotOperational(u32)`). `SelectRobot`은 계속 허용한다(고장난 로봇을 선택해서 상태를 확인하고 복구 커맨드를 보낼 대상으로 지정할 수 있어야 하므로).

## 복구 프로세스

```rust
const REPAIR_TICKS: u32 = 100; // 20Hz 기준 5초 — 튜닝 대상
```

- `RepairRobot { robot_id: u32 }` 커맨드(신규) 처리 시: 대상 로봇이 `status == Failed`가 아니면 `game_state::CommandError`(기존 `RobotNotFound` 하나뿐인 enum에 신규 variant 추가, 예: `CommandError::RobotNotFailed(u32)`)로 거부. 맞으면 `status = Repairing { remaining_ticks: REPAIR_TICKS }`로 전이.
- 매 틱, `Repairing { remaining_ticks }` 상태인 로봇은 `remaining_ticks`를 1 감소시킨다. 0이 되면 `status = Operational`, `worn_ticks = 0`으로 리셋(부품 교체 완료).
- **동시 복구 대수 제한 없음**(v1) — 여러 로봇이 동시에 `Repairing` 상태일 수 있고, 각자 독립적으로 `remaining_ticks`가 줄어든다. "정비 인력/슬롯" 같은 자원 제약은 스코프 밖(아래 참고).

## 프로토콜 변경

`server/src/protocol.rs`:

```rust
// ClientCommand에 추가
RepairRobot { robot_id: u32 },

// RobotView에 추가
pub status: WireStatus,
pub durability_remaining: f32,  // 1.0 - (worn_ticks / WEAR_LIMIT_TICKS), [0.0, 1.0] 클램프

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum WireStatus {
    Operational,
    Failed,
    Repairing { remaining_ticks: u32 },
}
```

기존 `WireTask`/`WirePose` 변환 패턴과 동일하게 `RobotStatus -> WireStatus` 변환 함수를 추가한다. `Delta`/`Snapshot` 메시지는 이미 `RobotView`를 통째로 담으므로 별도 배선 없이 자동으로 `status`/`durability_remaining`이 실려간다.

## 관측가능성

**메트릭** (`server/src/metrics.rs`, 기존 `Metrics` 구조체에 추가):
- `robot_failures_total: IntCounter` — 로봇이 `Operational -> Failed`로 전이할 때마다 증가.
- `robots_repairing: IntGauge` — 매 틱, 현재 `Repairing` 상태인 로봇 수로 갱신(기존 `robot_count` 갱신과 같은 자리에서).

**영속화** (`server/src/persistence.rs`, 신규 테이블 — 기존 `stats_history`와 같은 패턴):

```sql
CREATE TABLE IF NOT EXISTS robot_failure_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    tick INTEGER NOT NULL,
    robot_id INTEGER NOT NULL,
    event_type TEXT NOT NULL  -- 'failed' | 'repaired'
);
```

`insert_failure_event(conn, tick, robot_id, event_type)` / `recent_failure_events(conn, limit)` 함수를 `insert_stats`/`recent_stats`와 같은 스타일로 추가. 신규 REST 엔드포인트 `GET /api/robots/failures`(기존 `GET /api/stats/history`와 동일한 배선 방식: `spawn_blocking` + 최근 50건).

상태 전이 자체는 `sim_core::sim::tick()`(네트워크 의존성 없는 순수 시뮬레이션 코어) 안에서 일어나므로, `main.rs`의 `spawn_tick_loop`는 매 틱 이전 스냅샷과 이번 틱 스냅샷의 로봇별 `status`를 비교해 전이를 감지한다(로봇 ID 기준 매칭 — 이미 `compute_delta`가 이전/현재 스냅샷을 비교하는 것과 같은 방식). `Operational -> Failed` 전이를 감지하면 이벤트 하나(`'failed'`), `* -> Operational`(Repairing 완료) 전이를 감지하면 이벤트 하나(`'repaired'`)를 만들어 `spawn_blocking`으로 적재한다(기존 `stats_row`를 매 N틱마다 적재하는 것과 달리, 이건 전이가 실제로 일어난 틱에만 적재 — 빈도가 낮으므로 매 틱 비교 정도는 성능에 문제없음).

## 테스트 전략

1. **`deterministic_roll` 단위테스트**: 같은 `(robot_id, tick_count)` 입력 → 항상 같은 출력. 여러 입력에 대해 결과가 `[0.0, 1.0)` 범위 안에 있는지, 대략 고르게 분포하는지(예: 10000개 샘플의 평균이 0.5 근처인지) 확인.
2. **상태 전이 단위테스트**: `worn_ticks` 누적(작업 중에만) / 고장 시 `Failed` 전이 / `RepairRobot` 거부(비고장 로봇) / 복구 진행(`remaining_ticks` 감소) / 복구 완료 시 `worn_ticks` 리셋.
3. **기존 결정성 proptest 재검증**: `tick_is_deterministic`/`tick_never_produces_collisions`가 고장 로직 추가 후에도 여전히 성립하는지 — 이 기능이 기존 핵심 불변식을 깨지 않는다는 가장 중요한 증거.
4. **고장난 로봇이 장애물로 취급되는지 통합/단위테스트**: 고장난 로봇의 칸을 다른 로봇의 `find_path`가 회피하는지 직접 검증(기존 `plan_robot`/`find_path` 테스트 패턴 재사용).
5. **REST/메트릭 통합테스트**: `robot_failures_total`/`robots_repairing`이 실제로 증가/증감하는지, `/api/robots/failures`에 실제 이벤트가 쌓이는지 — 지금까지와 같은 방식으로 뮤테이션 테스트(관련 로직을 일부러 깨서 테스트가 실패하는지)까지 확인.

## 스코프 밖 (v2 이상)

- 부품별(다리/팔) 개별 마모
- 예비부품 재고/소모 시스템
- MTBF/MTTR 같은 집계 통계 REST 엔드포인트(원본 이벤트는 저장하므로 나중에 추가 가능)
- 고장 확률에 영향을 주는 다른 요인(예: 로봇 나이, 환경 요인)
