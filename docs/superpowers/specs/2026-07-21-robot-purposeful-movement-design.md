# 목적있는 이동 (작업 사이클) 설계

## 배경

라이브 데모를 확인한 사용자 피드백: "목적도 없이 돌아다니는 것 같다" — 로봇이 순찰(patrol)만 하고, 컨베이어 on/off나 실제 생산량과 전혀 무관하게 움직인다.

브레인스토밍(2026-07-21)으로 설계한 뒤 독립 리뷰어의 완결성 리뷰를 거쳤다. 최초 초안은 [`2026-07-21-robot-visual-redesign-design.md`](2026-07-21-robot-visual-redesign-design.md)(로봇 외형)와 한 문서였으나, 두 기능이 코드를 전혀 공유하지 않는다는 지적에 따라 분리했다. 이 문서는 리뷰에서 나온 실질적 갭(§2.2의 지점 충돌 버그, §2.6의 `TriggerArmAction` 악용 경로, `tick()` 시그니처 변경의 회귀 위험)을 전부 반영해 고친 버전이다.

## 범위

- **서버+클라이언트 변경**: 컨베이어 on/off에 실제로 연동되는 작업 사이클(픽업→운반→배치), 생산량 집계를 실제 작업 완료에 연결, 운반 중 시각화.
- **범위 밖**: 아이템 종류/재고 시스템(단일 화물 개념만), 관전용 다중 세션(기존 v1 스코프 유지), 로봇이 서로 같은 작업 지점을 두고 계속 경합하는 경우의 완전한 회피 로직(아래 §2.2 "잔여 한계" 참고 — 받아들인 한계).

---

## 1. 핵심 아이디어

컨베이어가 켜져 있으면 로봇이 **자동으로** 픽업 지점 → 배치 지점을 오가며 화물을 나른다. 컨베이어가 꺼지면 기존 순찰(patrol)로 되돌아간다.

## 2. `sim_core::tick`에 컨베이어 상태 전달

`Conveyor`(현재 `game_state.rs`, 바이너리 크레이트 전용)는 지금까지 시뮬레이션에 아무 영향도 못 줬다(순수 UI 토글, `tick()`은 그 존재를 모름). 작업 사이클을 sim_core 안에서(다른 로봇 상태 전이와 같은 방식으로, 결정적이고 순수하게) 판단하려면 `tick`이 이 정보를 알아야 한다. `Conveyor` 구조체 자체를 sim_core로 옮기지 않고, 최소 변경으로 불리언 하나만 파라미터로 추가한다:

```rust
pub fn tick(state: &SimState, conveyor_running: bool) -> SimState
```

`plan_robot`/`safe_plan_robot`/`safe_tick`까지 이 값을 관통시킨다. `game_state.rs`는 여전히 `Conveyor`를 소유하고 매 틱 `tick(&self.sim, self.conveyor.running)`으로 호출만 한다 — sim_core는 네트워크/UI 개념을 계속 모른다.

**기존 호출부 마이그레이션(회귀 위험 — 리뷰에서 지적됨)**: `tick(`/`plan_robot(`/`safe_plan_robot(`/`safe_tick(`를 직접 호출하는 기존 테스트가 `sim.rs`에 다수, `tick_properties.rs`에 7개 있다. 시그니처에 파라미터를 추가하면 컴파일이 깨져 전부 명시적으로 값을 넘겨야 하는데, **순찰 전용 동작을 검증하는 테스트는 반드시 `conveyor_running = false`로 호출해야 한다** — 그렇지 않으면 조용히(컴파일은 되지만) 다른 로직을 검증하게 된다. 최소한 아래 테스트는 `false`로 마이그레이션:
- `next_patrol_goal_alternates_between_the_two_patrol_points`
- `robot_picks_a_new_patrol_goal_and_moves_on_the_same_tick_it_arrives`
- 그 외 "patrol"이라는 이름이 들어간 모든 기존 테스트

새로 작성하는 작업 사이클 테스트(§6)는 `conveyor_running = true`로 호출한다. `Conveyor::new()`의 기본값이 `running: true`라는 점(`game_state.rs`)도 유의 — `GameState`를 통하지 않고 `SimState`/`tick()`을 직접 쓰는 테스트는 이 기본값의 영향을 받지 않으므로 값을 빠뜨리면 컴파일 에러로 바로 드러난다(런타임에 조용히 틀리게 동작할 위험은 이 경로엔 없음 — 위 patrol 테스트들만 "컴파일은 되지만 의미가 달라지는" 진짜 위험군이다).

## 3. 로봇 필드 추가

```rust
pub struct Robot {
    // ...기존 필드...
    pub carrying: bool,
    pub work_ticks_remaining: u32,
}
```

`Robot::new`의 기본값은 각각 `false`/`0`. `work_ticks_remaining`은 `RobotStatus::Repairing{remaining_ticks}`와 달리 `Task`에 데이터를 얹지 않고 별도 필드로 둔다 — `Task`는 여전히 데이터 없는 3-variant enum으로 남아 `TriggerArmAction`(오퍼레이터가 보내는 값도 `Task`) 와이어 형태가 안 바뀐다. 이 필드는 자동 작업 사이클 내부 카운트다운 전용이라 프로토콜(`RobotView`)에는 노출하지 않는다 — 진행 상황은 어차피 팔의 연속적인 IK 포즈(`arm_pose`)로 이미 보이기 때문에 별도 숫자 표시가 필요 없다.

**`Failed`/`Repairing` 중 보존**: `carrying`/`work_ticks_remaining`은 `task`와 똑같이 `update_status`가 건드리지 않는다 — 화물을 들고 있던 로봇이 고장나면 복구될 때까지 (시각적으로도) 계속 들고 있는 상태로 남고, 복구되면 하던 작업을 이어서 끝낸다. `task`가 이미 이렇게 동작하므로(§2.4의 "Repairing 중에도 `task`는 그대로 보존" 설계) 일관성을 위해 동일하게 처리한다.

## 4. 작업 지점 — `patrol_points` 재사용하지 않음

**당초 설계는 기존 `patrol_points(id, grid)`를 픽업/배치 지점으로 그대로 재사용하는 것이었으나, 완결성 리뷰에서 실제 버그가 발견되어 바꿨다**: `patrol_points`는 `(id*7 mod w, id*3 mod h)`처럼 x/y를 각각 독립적으로 계산하므로, 정확히 10×10 그리드(프로덕션 기본 크기)에서는 `id`와 `id+10`이 **완전히 같은 좌표 쌍**을 받는다 — 목표 규모(20~50대)에서 반드시 여러 번 발생한다. 순찰에서는 스쳐 지나가는 통과점이라 무해했지만(로봇이 거기 머무르지 않음), 작업 사이클에서는 로봇이 그 지점에서 `PICK_TICKS`/`PLACE_TICKS`(각 20틱, §5) 동안 **정지**하므로, 같은 지점을 노리는 다른 로봇이 그만큼 오래 못 들어가고 막힐 수 있다.

대신 새 함수 `work_points(id, grid) -> (CellId, CellId)`를 추가한다. 이미 있는 `deterministic_roll(seed_a, seed_b)` 해시(로봇 고장 판정에 쓰는 것과 같은 순수 함수)를 재사용해 그리드 전체 칸(`w*h`개) 중 하나를 인덱스로 뽑는다:

```
pickup_idx = floor(deterministic_roll(id, PICKUP_SEED) * w * h) % (w * h)
place_idx  = floor(deterministic_roll(id, PLACE_SEED)  * w * h) % (w * h)
// place_idx == pickup_idx면 +1 (mod w*h)해서 항상 서로 다른 두 지점을 보장
```

(`PICKUP_SEED`/`PLACE_SEED`는 `deterministic_roll`의 두 번째 인자 자리에 넣는 고정 상수 2개 — 예: `0`, `1`. 원래 이 자리는 `tick_count`가 들어가던 자리지만, 여기서는 "픽업용 해시"와 "배치용 해시"를 구분하는 용도로 재사용하는 것뿐이다.)

**잔여 한계(명시적으로 받아들임)**: 해시 충돌로 서로 다른 로봇이 우연히 같은 인덱스를 뽑을 가능성은 남아있다(생일 문제 — 그리드가 100칸이면 20~50대 규모에서도 확률이 0은 아니다). 다만 `patrol_points`처럼 "정확히 10대마다 100% 겹침"이 아니라 "낮은 확률로 가끔 겹침"으로 바뀐다. 겹치더라도 시뮬레이션이 멈추거나 깨지지 않는다 — 기존 `occupied` 스냅샷+타이브레이크가 칸 단위 충돌 자체는 계속 해소하고, 다만 겹친 로봇 중 하나는 그 칸에 못 들어가고 근처에서 재계획을 반복하며 대기한다(`failed_robot_permanently_blocks_the_cell_for_other_robots` 테스트가 보여주듯 이런 정체는 상대가 그 칸을 계속 차지하는 한 지속될 수 있음). 그 로봇의 생산 기여가 일시적으로 줄어드는 정도로 bounded — 포트폴리오 스코프에서 완전한 회피 로직(재시도/대체 지점 탐색)까지는 만들지 않는다(YAGNI).

## 5. 상태 전이 — `task`는 읽지 않고 오직 파생시켜서 쓴다

`plan_robot` 안에서 `update_status` 이후, 기존 순찰 분기를 아래 로직으로 대체한다. **핵심 설계 원칙(리뷰에서 나온 악용 경로를 막기 위한 결정)**: 이 로직은 로봇의 현재 `task` 값을 절대 입력으로 읽지 않는다 — 오직 `(carrying, work_ticks_remaining, pos)`만 보고 매 틱 `task`를 다시 계산해서 덮어쓴다. `task`는 이 로직의 순수한 출력(렌더링용 신호)일 뿐이다. 이렇게 해야 §6에서 설명하는 `TriggerArmAction` 악용 경로가 원천 차단된다.

- **컨베이어 꺼짐**(`conveyor_running == false`): 만약 `task != Idle` 이거나 `carrying == true` 이거나 `work_ticks_remaining > 0`이면 전부 리셋(`task = Idle`, `carrying = false`, `work_ticks_remaining = 0`) — 어중간한 상태로 남지 않도록 하는 결정적 규칙. 이후 기존 순찰 로직 그대로(두 지점 왕복, `patrol_points` 그대로 사용 — 여기는 안 바뀜).
- **컨베이어 켜짐**(`conveyor_running == true`): 픽업 지점 = `work_points().0`, 배치 지점 = `work_points().1`.

  1. `work_ticks_remaining > 0`인 동안: 이동하지 않음(제자리). `task`를 `carrying`이면 `Placing`, 아니면 `Picking`으로 설정(매 틱 다시 씀). `work_ticks_remaining`을 1 감소시키고, 0이 되면 `carrying`을 반전하고 `task = Idle`로 설정(같은 틱 안에서 완료 처리 — 생산량 반영은 §6).
  2. `work_ticks_remaining == 0`인 동안: 목표 지점 = `carrying`이면 배치 지점, 아니면 픽업 지점.
     - `pos != 목표`: 목표 지점으로 이동(기존 A* 재사용). `task = Idle`(이동 중에는 항상 Idle로 덮어씀 — `TriggerArmAction`이 뭘 설정했든 무시). **목표가 이전 틱과 달라졌다면(`carrying`이 방금 반전된 직후 등) `path`를 비우고 `ticks_until_repath = 0`으로 만들어 즉시 재계획하게 한다** — 목표만 바꾸고 남은 경로/재계획 타이머를 안 지우면 낡은 경로를 계속 따라가는 버그가 난다(`sim.rs`의 기존 순찰 재계획 테스트가 이미 이 요구사항을 명시).
     - `pos == 목표`: `task`를 `carrying`이면 `Placing`, 아니면 `Picking`으로 설정하고 `work_ticks_remaining = PLACE_TICKS`(또는 `PICK_TICKS`)로 채운다. 이동하지 않음.

  `PICK_TICKS`/`PLACE_TICKS`는 `REPAIR_TICKS`(100)와 같은 위치(`sim.rs` 상단 pub const)에 각각 `20`(20Hz 틱 기준 약 1초)으로 정의한다 — 데모에서 반복되는 모습을 자주 볼 수 있을 만큼 짧게.

## 6. `TriggerArmAction`과의 관계 — 컨베이어 켜짐일 땐 완전한 no-op

§5의 설계(task를 절대 입력으로 읽지 않고 매 틱 다시 파생시켜 덮어씀) 때문에, 컨베이어가 켜져 있는 동안 오퍼레이터가 `TriggerArmAction`으로 어떤 `task`를 보내든 **바로 다음 틱에 자동 로직이 무조건 덮어쓴다** — "덮어쓸 수도 있다"가 아니라 항상 덮어쓴다. 이는 최초 설계 초안에서 리뷰가 지적한 악용 경로(수동으로 `task = Picking`을 보내면 `work_ticks_remaining`이 갱신 안 된 채 자동 로직이 이를 "이미 카운트다운 중"으로 오인해 대기 없이 즉시 `carrying = true`가 되는 것)를 막기 위한 결정이다 — 자동 로직은 애초에 `task`를 읽지 않으므로 이런 오인 자체가 불가능하다.

컨베이어가 꺼져 있을 때는 자동 사이클이 전혀 관여하지 않으므로 `TriggerArmAction`이 기존과 동일하게 완전히 유효하다 — 순찰 중인 로봇을 대상으로 수동으로 팔 동작(포즈)만 데모하고 싶을 때 여전히 쓸 수 있다.

## 7. 생산량 집계 — 실제 작업 완료에 연결

현재 `main.rs`(틱 루프)는 컨베이어가 켜져 있기만 하면 로봇 존재 개수 × 0.01을 매 틱 더한다(`task`/`carrying`과 완전히 무관 — 사용자가 지적한 "목적 없음"의 실제 근거 중 하나였다). 이를 다음으로 대체한다:

- 기존 `detect_status_transitions`(고장/복구 이벤트 감지, 순수 함수, `main.rs`)와 같은 패턴으로 `detect_completed_placements(prev_robots: &[RobotView], curr_robots: &[RobotView]) -> Vec<u32>`를 추가한다 — `carrying`이 `true`(이전) → `false`(이후)로 바뀐 로봇 ID만 뽑는다(= 방금 배치를 완료함).
- `total_production` 호출에 넘기는 `units_per_robot`을, 이 목록에 있는 로봇에게만 `UNIT_PER_CYCLE`(예: `1.0`)을, 나머지는 `0.0`을 주도록 바꾼다. `sim_core::production::total_production` 자체의 시그니처는 안 바뀐다(원래도 외부에서 만든 맵을 받는 구조였음).

이렇게 하면 "컨베이어가 켜져 있으면 로봇 수만큼 생산량이 오른다"에서 "실제로 배치를 완료한 로봇 수만큼 생산량이 오른다"로 바뀐다 — 로봇이 고장나거나(Failed) 아직 픽업 지점으로 이동 중이면 생산에 기여하지 않는다.

## 8. 운반 시각화 (와이어 프로토콜)

`RobotView`에 `carrying: bool` 필드를 추가한다(`status`/`durability_remaining`을 추가했던 것과 같은 패턴). 양자화 불필요(이미 불리언이라 델타 압축에 영향 없음).

클라이언트는 `carrying == true`인 로봇의 그리퍼(팔 `arm_pose`의 wrist 좌표) 위치에 작은 화물 아이콘을 그린다. 컨베이어 타일도 `conveyor.running` 값에 따라 기존 장식용 색(꺼짐=무채색, 켜짐=활성색 줄무늬)을 유지한다(이미 있는 로직 — 변경 없음).

## 9. 의도적으로 배제한 것

- 아이템 종류(재고/상품 다양성) — `carrying: bool` 하나로 충분. 여러 종류가 필요해지면 그때 `Option<ItemKind>`로 확장.
- 로봇이 서로 같은 작업 지점을 두고 계속 경합하는 경우의 완전한 회피(재시도/대체 지점 탐색) — §4 "잔여 한계" 참고.

---

## 10. 테스트 전략

기존 컨벤션을 그대로 따른다(실제 컴파일된 바이너리 + 진짜 WS/HTTP 클라이언트로 검증, 뮤테이션 테스트로 공허한 테스트 방지):

- `sim.rs`: 컨베이어 켜짐 상태에서 전체 사이클(픽업 이동→도착→Picking 20틱→carrying=true→배치 이동→도착→Placing 20틱→carrying=false)을 실제로 여러 틱 재현하는 테스트. 컨베이어가 꺼지는 순간 진행 중이던 작업이 리셋되는 테스트. `work_points`가 항상 서로 다른 두 지점을 반환하는 테스트(기존 `patrol_points_are_always_distinct_for_a_reasonably_sized_grid`와 같은 스타일). **`TriggerArmAction`으로 수동 설정한 `task`가 다음 틱에 자동 로직으로 덮어써지고 `carrying`이 즉시 반전되지 않는지 검증하는 회귀 테스트**(§6에서 막은 악용 경로를 직접 뮤테이션으로 재현 — "task를 입력으로 읽도록" 일부러 되돌려서 이 테스트가 실패하는지 확인).
- §2의 마이그레이션 목록에 있는 기존 patrol 테스트들이 `conveyor_running=false`로 여전히 통과하는지 확인.
- `tick_properties.rs`: 기존 결정성/무충돌 proptest에 `conveyor_running: bool`을 임의 입력으로 추가(양쪽 다 결정적이어야 함).
- `main.rs`: `detect_completed_placements` 순수 함수 유닛테스트(기존 `detect_status_transitions` 테스트와 같은 스타일) — 뮤테이션 테스트로 "carrying 변화 무시하고 다른 조건으로 발화" 케이스를 잡아낼 수 있는지 확인.
- REST/통합테스트: 컨베이어를 켜고 실제로 로봇이 여러 틱에 걸쳐 이동→작업→생산량 증가까지 하는 것을 `GET /api/stats/history`로 확인(기존 `insert_stats`/`stats_history` 테스트 패턴 재사용).
- 클라이언트: `carrying` 필드가 있을 때만 화물 아이콘이 그려지는지 vitest 단위 확인. Playwright E2E로 실제 브라우저에서 컨베이어 on/off 토글 시 로봇이 순찰↔작업 사이클 사이를 전환하는 것 확인.

## 11. 영향받는 파일

- `server/src/sim.rs` — `Robot.carrying`/`work_ticks_remaining`, `tick`/`plan_robot`/`safe_plan_robot`/`safe_tick` 시그니처에 `conveyor_running` 추가, `work_points` 함수, 작업 사이클 상태 기계, `PICK_TICKS`/`PLACE_TICKS`/`UNIT_PER_CYCLE`/`PICKUP_SEED`/`PLACE_SEED` 상수, 기존 patrol 테스트 마이그레이션(§2).
- `server/src/game_state.rs` — `tick` 호출부에 `self.conveyor.running` 전달.
- `server/src/protocol.rs`/`delta.rs` — `RobotView.carrying`.
- `server/src/main.rs` — `detect_completed_placements`, `units_per_robot` 계산 로직 교체, 틱 루프의 `tick()` 호출부.
- `server/tests/tick_properties.rs` — `conveyor_running` proptest 입력 추가.
- `client/src/render/canvas.ts` — `carrying`이면 화물 아이콘 그리기(로봇 자체 재작성은 별도 시각 리디자인 문서 담당).
- `client/src/net/protocol.ts` — `RobotView.carrying` 타입 추가.
- `README.md`/`docs/KANBAN.md` — 완료 후 기존 컨벤션대로 갱신.
