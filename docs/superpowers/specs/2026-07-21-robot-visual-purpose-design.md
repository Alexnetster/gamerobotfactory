# 로봇 외형 리디자인 + 목적있는 이동 설계

## 배경

라이브 데모(`https://gamerobotfactory.fly.dev`)를 직접 확인한 사용자 피드백 2건:

1. "로봇 같지 않은 것들이 돌아다닌다" — 현재 렌더링(평평한 주황색 사각형 몸통 + 일자 다리 4개)이 로봇처럼 안 보임.
2. "목적도 없이 돌아다니는 것 같다" — 로봇이 순찰(patrol)만 하고, 컨베이어 on/off나 실제 생산량과 전혀 무관하게 움직임.

브레인스토밍(2026-07-21, 비주얼 컴패니언 다수 목업 반복)으로 두 문제를 각각 설계했다.

## 범위

- **클라이언트 전용 변경**: 로봇 렌더링 방식(`client/src/render/canvas.ts`의 `drawRobot`).
- **서버+클라이언트 변경**: 컨베이어 on/off에 실제로 연동되는 작업 사이클(픽업→운반→배치), 생산량 집계를 실제 작업 완료에 연결, 운반 중 시각화.
- **범위 밖**: 아이템 종류/재고 시스템(단일 화물 개념만), 여러 개의 픽업/배치 스테이션(로봇당 기존 순찰 지점 2개를 그대로 재사용), 관전용 다중 세션(기존 v1 스코프 유지).

---

## 1. 로봇 외형 렌더링

### 1.1 시각 컨셉

Boston Dynamics Spot을 참조한 4족 로봇: 각진 박스형 몸통(패널 이음선 포함), 상단 센서 헤드, 어깨 장착 블록에서 뻗는 팔(기존 2-본 IK 그대로 재사용), digitigrade(사람과 반대로 꺾이는) 다리 4개.

### 1.2 투영 각도 — 타일과 로봇을 분리

바닥 타일 그리드는 기존 엄격한 아이소메트릭(2:1 다이아몬드) 투영을 그대로 유지한다. 로봇 캐릭터만 타일과 다른 투영인 **3/4 정면 틀어진 각도**(Diablo/Stardew Valley 방식)로 그린다 — 몸통의 큰 앞면이 시청자를 향하고 센서 헤드가 정면을 보게 해서 실루엣과 "얼굴"이 뚜렷해진다. 다리 4개 중 앞 2개(시청자 쪽)는 크고 뚜렷하게, 뒤 2개는 몸통에 부분적으로 가려지게 그린다.

이 방식은 타일 투영과 로봇 그림이 수학적으로 완전히 일치하지 않지만(대부분의 아이소메트릭 게임이 이미 쓰는 관행), "장난감 같다"는 원래 문제를 가장 직접적으로 해결한다고 목업 비교로 확인됨.

### 1.3 다리/팔 관절 — 반드시 지켜야 할 두 가지 제약

목업 반복 과정에서 실제로 겪은 문제 2건을 명시적 제약으로 남긴다:

1. **관절 마디가 시각적으로 끊어지면 안 됨**: 허벅지-정강이(또는 어깨-팔꿈치) 경계에 색/굵기가 다르거나 강조용 점(ball-joint 마커)이 있으면 "장난감 부품을 조립한 것처럼" 보인다. 굽는 지점 앞뒤로 스트로크 색과 굵기를 완전히 동일하게 써서 하나의 매끈한 곡선처럼 보이게 한다.
2. **다리 4개는 전부 몸통 실루엣 경계에서 시작해야 함**: 뒤쪽 다리(시청자에게 가려지는 2개)라고 해서 몸통과 떨어진 허공에서 시작하면 안 된다 — 몸통 밑면 모서리에서 시작해서(몸통 도형보다 먼저 그려 살짝 가려지게) 옆으로 뻗어나가야, 정지 상태뿐 아니라 걷는 동안에도 몸통에서 안 떨어진 것처럼 보인다. (실제로 겪은 버그: 뒷다리 시작점이 몸통 바깥 좌표라 걸을 때 눈에 띄게 떠 보였음.)

### 1.4 걸음 애니메이션 — 위상→각도 매핑

서버가 이미 매 틱 보내주는 `leg_cycle_progress`(0.0~1.0, 다리별로 0.25씩 위상차)는 그대로 재사용한다 — 새 서버 데이터나 프로토콜 변경 불필요. 클라이언트가 이 위상을 (엉덩이 각도, 무릎 각도) 쌍으로 매핑하는 함수만 새로 만든다:

- **디딤 구간(위상 0~60%)**: 엉덩이가 천천히 회전(발이 바닥에 붙은 채 몸통 아래로), 무릎은 편 상태 유지.
- **흔듦 구간(위상 60~100%)**: 엉덩이가 빠르게 앞으로 복귀, 무릎이 굽혀져(발을 들어올림) 스윙.

이 비대칭 타이밍이 "미끄러지듯 이동한다"(과거 리뷰에서 "manta ray" 라고 표현됨)는 인상을 줄인다. 팔도 동일하게 어깨/팔꿈치를 독립적으로 굽히되, 각도 자체는 기존 `arm_pose`(서버의 2-본 IK 결과)를 그대로 쓴다 — 새로 애니메이션 값을 만들지 않는다.

### 1.5 상태 표시

센서 헤드의 눈 색상으로 로봇 상태를 표시한다: 평상시(회색/꺼짐 톤), 실제로 작업 중(`task != Idle`)일 때 노란색 점등. 기존 `RobotStatus`(`Failed`는 이미 다른 방식으로 구별돼 있음 — 예: 정지 상태)와 충돌하지 않도록, 이 눈 색상은 오직 "지금 팔이 뭔가를 하고 있는가"만 나타낸다.

---

## 2. 목적있는 이동 (작업 사이클)

### 2.1 핵심 아이디어

컨베이어가 켜져 있으면 로봇이 **자동으로** 픽업 지점 → 배치 지점을 오가며 화물을 나른다. 컨베이어가 꺼지면 기존 순찰(patrol)로 되돌아간다. 새 스테이션 좌표 체계를 만들지 않고, 기존 `patrol_points(id, grid)`가 반환하는 두 지점을 그대로 "픽업 지점(A)"/"배치 지점(B)"으로 재해석한다.

### 2.2 `sim_core::tick`에 컨베이어 상태 전달

`Conveyor`(현재 `game_state.rs`, 바이너리 크레이트 전용)는 지금까지 시뮬레이션에 아무 영향도 못 줬다(순수 UI 토글). 작업 사이클을 sim_core 안에서(다른 로봇 상태 전이와 같은 방식으로, 결정적이고 순수하게) 판단하려면 `tick`이 이 정보를 알아야 한다. `Conveyor` 구조체 자체를 sim_core로 옮기지 않고, 최소 변경으로 불리언 하나만 파라미터로 추가한다:

```rust
pub fn tick(state: &SimState, conveyor_running: bool) -> SimState
```

`plan_robot`/`safe_plan_robot`/`safe_tick`까지 이 값을 관통시킨다. `game_state.rs`는 여전히 `Conveyor`를 소유하고 매 틱 `tick(&self.sim, self.conveyor.running)`으로 호출만 한다 — 레이어 경계(그림 3 "아키텍처" 참고: sim_core는 네트워크/UI 개념을 모름)는 그대로 유지된다.

**영향받는 호출부**(구현 계획에서 전부 갱신 필요): `main.rs`의 틱 루프, `tick_properties.rs`의 모든 proptest, `sim.rs` 자체 유닛테스트 다수. 기계적이지만 범위가 넓은 변경이다.

### 2.3 로봇 필드 추가

```rust
pub struct Robot {
    // ...기존 필드...
    pub carrying: bool,
    pub work_ticks_remaining: u32,
}
```

`Robot::new`의 기본값은 각각 `false`/`0`. `work_ticks_remaining`은 `RobotStatus::Repairing{remaining_ticks}`와 달리 `Task`에 데이터를 얹지 않고 별도 필드로 둔다 — `Task`는 여전히 데이터 없는 3-variant enum으로 남아 `TriggerArmAction`(오퍼레이터가 보내는 값도 `Task`) 와이어 형태가 안 바뀐다. 이 필드는 자동 작업 사이클 내부 카운트다운 전용이라 프로토콜(`RobotView`)에는 노출하지 않는다 — 진행 상황은 어차피 팔의 연속적인 IK 포즈(`arm_pose`)로 이미 보이기 때문에 별도 숫자 표시가 필요 없다.

### 2.4 상태 전이 (Operational 로봇만 해당 — Failed/Repairing은 기존과 동일하게 완전히 얼어붙음)

`plan_robot` 안에서 `update_status` 이후, 기존 순찰 분기를 아래 로직으로 대체한다:

- **컨베이어 꺼짐**(`conveyor_running == false`): 기존 순찰 로직 그대로(두 지점 왕복). 단, 만약 로봇이 `task != Idle` 이거나 `carrying == true`인 상태에서 컨베이어가 꺼졌다면, 진행 중이던 작업은 즉시 리셋한다(`task = Idle`, `carrying = false`, `work_ticks_remaining = 0`) — 어중간한 상태로 남지 않도록 하는 결정적 규칙.
- **컨베이어 켜짐**(`conveyor_running == true`): 아래 상태 기계를 따른다(픽업 지점 = `patrol_points().0`, 배치 지점 = `patrol_points().1`):

  | 현재 상태 (task, carrying, 위치) | 다음 행동 |
  |---|---|
  | Idle, 안 들고 있음, 픽업 지점 아님 | 픽업 지점으로 이동 (기존 A* 재사용) |
  | Idle, 안 들고 있음, 픽업 지점 도착 | `task = Picking`, `PICK_TICKS` 동안 유지 |
  | Picking 카운트다운 종료 | `carrying = true`, `task = Idle` |
  | Idle, 들고 있음, 배치 지점 아님 | 배치 지점으로 이동 |
  | Idle, 들고 있음, 배치 지점 도착 | `task = Placing`, `PLACE_TICKS` 동안 유지 |
  | Placing 카운트다운 종료 | `carrying = false`, `task = Idle` (생산량 반영은 2.5절) → 사이클 반복 |

  `PICK_TICKS`/`PLACE_TICKS`는 `REPAIR_TICKS`(100)와 같은 위치(`sim.rs` 상단 pub const)에 각각 `20`(20Hz 틱 기준 약 1초)으로 정의한다 — 데모에서 반복되는 모습을 자주 볼 수 있을 만큼 짧게. `task`가 `Picking`/`Placing`으로 바뀌는 순간 `work_ticks_remaining`을 해당 상수로 설정하고, 그 상태가 유지되는 매 틱 1씩 감소시키다가 0이 되면 표에 적힌 다음 행동(`carrying` 반전 + `task = Idle`)을 실행한다.

  Picking/Placing 진행 중에는 이동하지 않는다(제자리에서 팔 동작만) — 기존 `RobotStatus::Failed`가 이동을 막는 것과 같은 이유(팔 작업 중 몸통이 움직이면 IK가 부자연스러움).

  **도착 판정과 목표 선택**: "지점 도착"은 기존 순찰과 동일하게 `next.pos == 목표지점`으로 판단한다. 다만 목표 지점을 고르는 규칙은 기존 `next_patrol_goal`(도착하면 무조건 반대 지점으로 alternation)과 다르다 — 컨베이어가 켜져 있을 때는 `carrying` 값이 목표를 결정한다(들고 있으면 배치 지점, 안 들고 있으면 픽업 지점). 컨베이어가 꺼지면 다시 기존 `next_patrol_goal`의 단순 alternation으로 되돌아간다.

### 2.5 생산량 집계 — 실제 작업 완료에 연결

현재 `main.rs`(틱 루프)는 컨베이어가 켜져 있기만 하면 로봇 존재 개수 × 0.01을 매 틱 더한다(`task`/`carrying`과 완전히 무관 — 사용자가 지적한 "목적 없음"의 실제 근거 중 하나였다). 이를 다음으로 대체한다:

- 기존 `detect_status_transitions`(고장/복구 이벤트 감지, 순수 함수, `main.rs`)와 같은 패턴으로 `detect_completed_placements(prev_robots: &[RobotView], curr_robots: &[RobotView]) -> Vec<u32>`를 추가한다 — `carrying`이 `true`(이전) → `false`(이후)로 바뀐 로봇 ID만 뽑는다(= 방금 배치를 완료함).
- `total_production` 호출에 넘기는 `units_per_robot`을, 이 목록에 있는 로봇에게만 `UNIT_PER_CYCLE`(예: `1.0`)을, 나머지는 `0.0`을 주도록 바꾼다. `sim_core::production::total_production` 자체의 시그니처는 안 바뀐다(원래도 외부에서 만든 맵을 받는 구조였음).

이렇게 하면 "컨베이어가 켜져 있으면 로봇 수만큼 생산량이 오른다"에서 "실제로 배치를 완료한 로봇 수만큼 생산량이 오른다"로 바뀐다 — 로봇이 고장나거나(Failed) 아직 픽업 지점으로 이동 중이면 생산에 기여하지 않는다.

### 2.6 `TriggerArmAction`과의 관계

기존 `TriggerArmAction` 커맨드(오퍼레이터가 수동으로 `task`를 지정)는 그대로 남긴다 — 단, 컨베이어가 켜져 있는 동안은 위 자동 사이클이 매 틱 `task`/`carrying`을 계속 갱신하므로, 수동으로 지정한 `task`는 다음 틱에 자동 로직이 덮어쓸 수 있다(우선순위: 자동 사이클이 항상 이김). 컨베이어가 꺼져 있을 때는 자동 사이클이 동작하지 않으므로 `TriggerArmAction`이 그대로 유효하다 — 순찰 중인 로봇을 대상으로 수동으로 팔 동작(포즈)만 데모하고 싶을 때 여전히 쓸 수 있다.

### 2.7 운반 시각화 (와이어 프로토콜)

`RobotView`에 `carrying: bool` 필드를 추가한다(`status`/`durability_remaining`을 추가했던 것과 같은 패턴). 양자화 불필요(이미 불리언이라 델타 압축에 영향 없음).

클라이언트는 `carrying == true`인 로봇의 그리퍼(팔 `arm_pose`의 wrist 좌표) 위치에 작은 화물 아이콘을 그린다. 컨베이어 타일도 `conveyor.running` 값에 따라 기존 장식용 색(꺼짐=무채색, 켜짐=활성색 줄무늬)을 유지한다(이미 있는 로직 — 변경 없음).

### 2.8 의도적으로 배제한 것

- 아이템 종류(재고/상품 다양성) — `carrying: bool` 하나로 충분. 여러 종류가 필요해지면 그때 `Option<ItemKind>`로 확장.
- 스테이션 여러 개/컨베이어 경로 상 임의 지점 픽업 — 기존 순찰 지점 2개 재사용으로 충분. 로봇마다 이미 결정적으로 다른 두 지점을 갖고 있어(각 로봇 ID 기반), 여러 로봇이 있으면 자연히 서로 다른 곳에서 작업하는 것처럼 보인다.
- 여러 로봇이 정확히 같은 픽업/배치 지점을 두고 경합하는 경우의 특별 처리 — 기존 `occupied` 스냅샷+타이브레이크 메커니즘이 이미 칸 단위 충돌을 해소하므로 추가 로직 불필요.

---

## 3. 테스트 전략

기존 컨벤션을 그대로 따른다(실제 컴파일된 바이너리 + 진짜 WS/HTTP 클라이언트로 검증, 뮤테이션 테스트로 공허한 테스트 방지):

- `sim.rs`: 컨베이어 켜짐/꺼짐 각각에 대해 작업 사이클 상태 전이 유닛테스트(픽업 도착→Picking→carrying=true→배치 이동→Placing→carrying=false 전체 사이클을 여러 틱에 걸쳐 실제로 재현). 컨베이어가 꺼지는 순간 진행 중이던 작업이 리셋되는지도 별도 테스트.
- `tick_properties.rs`: 기존 결정성/무충돌 proptest에 `conveyor_running: bool`을 임의 입력으로 추가(양쪽 다 결정적이어야 함).
- `main.rs`: `detect_completed_placements` 순수 함수 유닛테스트(기존 `detect_status_transitions` 테스트와 같은 스타일) — 뮤테이션 테스트로 "carrying 변화 무시하고 다른 조건으로 발화" 케이스를 잡아낼 수 있는지 확인.
- REST/통합테스트: 컨베이어를 켜고 실제로 로봇이 여러 틱에 걸쳐 이동→작업→생산량 증가까지 하는 것을 `GET /api/stats/history`로 확인(기존 `insert_stats`/`stats_history` 테스트 패턴 재사용).
- 클라이언트: vitest 단위로 위상→각도 매핑 함수(1.4절)를 뮤테이션 테스트와 함께 검증, `carrying` 필드가 있을 때만 화물 아이콘이 그려지는지 확인. Playwright E2E로 실제 브라우저에서 컨베이어 on/off 토글 시 로봇이 순찰↔작업 사이클 사이를 전환하는 것 확인.

## 4. 영향받는 파일 (구현 계획 참고용)

- `server/src/sim.rs` — `Robot.carrying`, `tick`/`plan_robot`/`safe_plan_robot`/`safe_tick` 시그니처에 `conveyor_running` 추가, 작업 사이클 상태 기계, `PICK_TICKS`/`PLACE_TICKS`/`UNIT_PER_CYCLE` 상수.
- `server/src/game_state.rs` — `tick` 호출부에 `self.conveyor.running` 전달.
- `server/src/protocol.rs`/`delta.rs` — `RobotView.carrying`.
- `server/src/main.rs` — `detect_completed_placements`, `units_per_robot` 계산 로직 교체, 틱 루프의 `tick()` 호출부.
- `server/tests/tick_properties.rs` — `conveyor_running` proptest 입력 추가.
- `client/src/render/canvas.ts` — `drawRobot` 전면 재작성(1절 전체).
- `client/src/net/protocol.ts` — `RobotView.carrying` 타입 추가.
- `README.md`/`docs/KANBAN.md` — 완료 후 기존 컨벤션대로 갱신.
