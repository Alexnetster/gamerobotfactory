# 조립 라인 (일자형 벨트 + 스테이션 + 헬퍼 로봇) 설계

## 배경

라이브 데모를 확인한 사용자 피드백: "컨베이어는 왜 돌고, 물건은 어디서 와서 어디로 흘러가는게 아니라, 갑자기 나타나서 갑자기 사라짐? 로봇은 마술사?" — 현재(목적있는 이동 기능, [`2026-07-21-robot-purposeful-movement-design.md`](2026-07-21-robot-purposeful-movement-design.md))는 로봇이 그리드 아무 칸에서나 화물을 "생성"해 다른 아무 칸으로 "운반"할 뿐, 실제 조립 라인이라는 그림이 없었다. 후속 피드백: "컨베이어는 물건만, 로봇은 그 위에 조립만 추가하는 것이 맞는 그림 아니야? 컨베이어 바깥에 필요한 부품이 있고, 로봇이 그걸 가져다 조립하는 과정이지, 부품이 부족하면 창고에서 물건 박스를 가져와서 조립하는 곳에 가져다 놓는 헬퍼 로봇도 따로 있어서 역할을 나눠야지?"

브레인스토밍(2026-07-24)으로 설계를 확정했다. 시각 브레인스토밍 도구로 레이아웃 옵션(U자 벨트 재사용 vs 일자형 벨트 신규)과 4발 드론의 단계별 조립 진행 과정을 직접 목업으로 보여주고 확인받았다.

## 범위

- **서버+클라이언트 전면 교체**: 기존 자유이동 픽업→운반→배치 사이클(`work_points`/U자 `belt_cells`/`Robot.carrying`+`work_ticks_remaining`)을 완전히 대체한다. 병존하지 않는다.
- **신규**: 일자형 벨트, 제품(Product) 엔티티와 단계 진행, 고정 배치된 조립 로봇, 창고에서 부품/빈 프레임을 나르는 헬퍼 로봇, 스테이션별 부품 재고.
- **범위 밖**: 부품 종류의 다양성(재고는 스테이션당 "그 스테이션 전용 부품 1종"만 취급 — 여러 부품을 조합하는 로직은 안 만듦), 다중 라인/병렬 벨트, 기존 저장된 DB 상태의 마이그레이션(§9 참고 — 초기화하고 새로 시작).

---

## 1. 레이아웃

시각 브레인스토밍에서 확정한 "B안"(일자형): 그리드(9×7, 기존과 동일) 가운데 가로줄(`y = 3`)이 벨트, `x = 1..=7`. 벨트 위쪽(`y = 0..=1`)이 창고 구역. 스테이션은 벨트 위 3개 고정 칸(`x = 2, 4, 6`)에 위치하고, 조립 로봇은 그 바로 옆(`y = 2`, 벨트 칸이 아님)에 붙박이로 선다 — 로봇이 벨트 칸 자체를 차지하지 않아 제품 이동과 물리적으로 겹치지 않는다.

```
y=0,1   [창고 구역 (helper 출발/도착 지점, 무한 부품/프레임 공급원)]
y=2     .  .  🤖1 .  🤖2 .  🤖3 .
y=3     ▓  ▓  ▓  ▓  ▓  ▓  ▓  ▓  ▓   (벨트, x=1..7)
```

라인 시작점 = `(1, 3)`, 종료(완성품 반출) 지점 = `(7, 3)`.

## 2. 제품(Product) 엔티티

```rust
pub struct Product {
    pub id: u32,
    pub stage: u8,       // 0=빈 프레임, 1=+배터리, 2=+프로펠러 4개, 3=완성(검사/포장 통과)
    pub pos: CellId,     // 벨트 위 현재 칸
}
```

`SimState`에 `products: Vec<Product>` 필드를 추가한다(기존 `robots: Vec<Robot>`과 나란히). 여러 제품이 벨트 위에 동시에 존재할 수 있다.

## 3. 조립 스테이션과 부품 재고

```rust
pub struct Station {
    pub index: u8,          // 0, 1, 2
    pub robot_cell: CellId, // 조립 로봇이 서 있는 칸 (y=2)
    pub belt_cell: CellId,  // 담당하는 벨트 칸 (y=3)
    pub part_inventory: u32,
}
```

`SimState.stations: Vec<Station>` — 그리드/로봇 수와 무관하게 항상 3개(레이아웃에 고정). `STATION_MAX_INVENTORY`(pub const, 예: 5) — 헬퍼가 한 번 보충하면 이 값까지 채운다.

## 4. 로봇 역할 분리 — `Robot.role`

기존 `Robot`은 전부 같은 방식(순찰 또는 자유 픽업/운반)으로 움직였다. 이제 두 역할로 나뉜다:

```rust
pub enum RobotRole {
    Assembly { station_index: u8 },  // 벨트 옆에 고정, 절대 이동 안 함
    Helper,                          // 창고<->스테이션/라인시작 이동
}
```

`Robot`에 `pub role: RobotRole` 필드를 추가한다. **조립 로봇은 스테이션 수만큼(3대) 시작 시 자동 생성되고 개수 조절 불가** — `SetRobotCount`는 이제 **헬퍼 로봇 개수만** 의미한다(§8).

기존 `carrying`/`work_ticks_remaining` 필드는 제거하지 않고 의미를 좁혀 재사용한다 — 조립 로봇의 "지금 조립 작업 중" 카운트다운, 헬퍼 로봇의 "지금 물건을 들고 이동 중"에 그대로 쓴다(완전히 새 필드를 만드는 대신 기존 것의 의미를 역할별로 특화 — 중복 방지 원칙에 따름).

## 5. 제품 이동과 조립 — 매 틱 로직

컨베이어가 꺼져 있으면(`conveyor_running == false`) 제품은 이동하지 않고 조립도 진행되지 않는다(기존 on/off 토글 의미 유지). 켜져 있으면:

1. **전진**: 벨트 위 각 제품은, 다음 칸이 (a) 스테이션 칸이 아니거나 이미 지나간 스테이션이면 그냥 한 칸 전진, (b) 자신이 아직 안 거친 스테이션 칸이면 그 스테이션에 진입 시도. 앞 칸에 이미 다른 제품이 있으면 대기(기존 로봇 이동 충돌 회피와 같은 패턴 — 이전 틱 스냅샷만 읽고 제품 ID로 타이브레이크, §7에서 상술).
2. **스테이션 진입**: 제품이 스테이션의 `belt_cell`에 도달하면:
   - `part_inventory > 0`: 그 자리에 정지, `ASSEMBLY_TICKS`(신규 pub const, `PICK_TICKS`와 같은 20) 카운트다운 시작. 끝나면 `part_inventory -= 1`, `product.stage += 1`, 제품 다시 전진 가능.
   - `part_inventory == 0`: 제품은 그 자리에서 그냥 대기(카운트다운 시작 안 함). 헬퍼가 보충해서 `part_inventory > 0`이 되는 순간 자동으로 위 카운트다운이 시작된다 — 제품이 다시 진입할 필요 없음, 이미 그 자리에 있음.
3. **완성**: 세 번째 스테이션(`x=6`)을 통과해 `stage == 3`이 된 제품이 벨트 끝 반출 지점(`x=7`)에 도달하면 `products`에서 제거하고 생산량 +`UNIT_PER_CYCLE`(기존 `main.rs::detect_completed_placements` 패턴을 `detect_completed_assemblies`로 이름만 바꿔 재사용 — 로직은 "직전 틱엔 있었는데 이번 틱엔 없어진 제품 ID"를 감지).
4. **라인 시작점 보충**: `(1,3)`에 제품이 없고, 아직 배정된 "프레임 배달" 헬퍼 작업이 없으면(§6) 헬퍼 작업 큐에 `DeliverFrame` 요청을 추가한다. 헬퍼가 배달을 완료하면 그 칸에 `stage: 0`인 새 제품이 생성된다.

## 6. 헬퍼 로봇 — 작업 배정

미해결 요청 2종(`RestockStation(station_index)`, `DeliverFrame`)을 **먼저 발생한 순서(= 재고/프레임이 먼저 0이 된 순서)**로 FIFO 큐에 쌓는다. 노는(Idle) 헬퍼는 매 틱 큐 맨 앞의 요청을 하나 가져가 배정받는다. 배정된 헬퍼는 창고 칸으로 이동 → 픽업(고정 틱 카운트다운, 화물 유형과 무관하게 동일) → 목적지(스테이션 `robot_cell` 또는 라인 시작점 `(1,3)`)로 이동 → 드롭(스테이션이면 `part_inventory = STATION_MAX_INVENTORY`, 라인 시작점이면 새 제품 생성) → 다시 Idle.

**중복 요청 방지**: 어떤 스테이션/라인시작점에 대해 이미 큐에 있거나 이미 배정된 요청이 있으면 같은 대상으로 새 요청을 또 만들지 않는다(재고가 여전히 0인 채 매 틱 계속 큐에 쌓이는 것 방지) — 요청은 "재고/프레임이 막 0이 된 순간"에 한 번만 생성되고, 그 요청이 처리 완료될 때까지 다시 생성되지 않는다.

**헬퍼 최소 1명**: 사이드바 `-` 버튼은 헬퍼 로봇 수를 1 밑으로 내릴 수 없다(§8) — 0명이 되면 재고가 0인 스테이션은 영원히 못 풀리고 라인 전체가 멈춰서 회복 불가능하기 때문.

## 7. 결정성 유지 — 제품도 로봇과 같은 이중버퍼+타이브레이크 패턴

이 프로젝트의 핵심 보장(`tick_is_deterministic`/`tick_never_produces_collisions` proptest)은 "매 로봇이 직전 틱 스냅샷만 읽고, 동시 충돌은 ID로 결정적으로 타이브레이크"하는 데서 나온다. 제품 전진 로직도 반드시 같은 패턴을 따른다 — 직전 틱의 제품 위치 스냅샷만 읽어 다음 칸 점유 여부를 판단하고, 두 제품이 같은 칸을 노리면 제품 ID가 작은 쪽이 우선한다. 새 proptest `products_never_occupy_the_same_cell`을 기존 로봇용 proptest 옆에 추가해 이를 검증한다.

## 8. 프로토콜 / 사이드바 의미 변경

- `ClientCommand::SetRobotCount { count }` — 의미가 "전체 로봇 수"에서 "헬퍼 로봇 수"로 바뀐다. 서버는 `count.max(1)`로 하한을 강제한다(§6). 조립 로봇 3대는 이 카운트에 포함되지 않는다.
- `RobotView`에 `role: RobotRole`(직렬화: `{"kind":"Assembly","station_index":0}` / `{"kind":"Helper"}`) 추가.
- 신규 `StationView { index: u8, robot_cell_x, robot_cell_y, part_inventory: u32 }` — 매 틱(또는 델타) 전체 스테이션 목록 전송(3개뿐이라 델타 압축 불필요, 항상 풀 목록).
- 신규 `ProductView { id: u32, stage: u8, x: i32, y: i32 }` — 델타 방식은 기존 `RobotView` 델타와 같은 패턴(추가/갱신/삭제) 재사용.
- 사이드바 라벨: "로봇 수" → "헬퍼 로봇 수"로 변경, `-` 버튼이 1에서 비활성화.

## 9. 기존 코드 정리 (완전 대체)

다음은 이 기능으로 완전히 대체되어 삭제된다: `work_points`, U자 `belt_cells`, 관련 테스트(`work_points_are_always_distinct_for_a_reasonably_sized_grid`, `work_points_always_land_on_a_conveyor_belt_cell`, `belt_cells_form_a_u_shape_open_on_the_right`, `full_work_cycle_moves_to_pickup_picks_up_carries_and_places`, `turning_conveyor_off_mid_work_resets_task_and_carrying_immediately`, `manual_trigger_arm_action_cannot_skip_the_work_cycle_wait`), 클라이언트 `isConveyorCell`(U자 판정)과 그 렌더링.

`Robot.carrying`/`work_ticks_remaining`은 필드 자체는 유지하되 의미가 역할별로 바뀐다(§4). `PICK_TICKS`/`PLACE_TICKS`는 `ASSEMBLY_TICKS`로 대체(조립 로봇), 헬퍼의 픽업/드롭 카운트다운은 새 상수(`HELPER_PICKUP_TICKS`/`HELPER_DROP_TICKS`, 동일 값으로 시작) 사용.

**기존 DB 상태 초기화**: 저장된 스냅샷은 옛 필드 의미(U자 벨트 좌표 기준 `work_ticks_remaining` 등)를 담고 있어 새 스키마와 호환되지 않는다. 마이그레이션 코드를 만들지 않고, 이 기능 배포 시 기존 DB 파일을 지우고 새로 시작한다(실사용자 없는 포트폴리오 프로젝트라 감수 가능한 트레이드오프).

## 10. 의도적으로 배제한 것

- 부품 종류 다양성 — 스테이션당 "그 스테이션 전용 부품 1종"만 개념적으로 존재, 실제로는 `part_inventory: u32` 카운터 하나.
- 마모/고장 확률 재튜닝 — 조립 로봇이 "제품 있으면 계속 일함"이라는 새 사용 패턴에 맞춰 `WEAR_LIMIT_TICKS`/실패확률을 미리 추측해 바꾸지 않는다. 구현 후 실제 플레이해보고 필요하면 조정(기존에도 두 번 그렇게 조정한 전례).
- 헬퍼의 창고→목적지 이동 시 "가장 가까운 헬퍼" 같은 거리 기반 배정 — §6처럼 순수 FIFO만 사용.
- 단일 라인이 한 곳에서 막히면 전체가 멈추는 것에 대한 우회 로직(병렬 라인, 우회 경로 등) — 병목이 눈에 보이는 것 자체가 의도.

---

## 11. 테스트 전략

기존 컨벤션 그대로(실제 컴파일된 바이너리 + 진짜 WS/HTTP 클라이언트로 검증, 뮤테이션 테스트로 공허한 테스트 방지):

- `sim.rs`: 제품이 라인 시작점부터 종료 지점까지 3개 스테이션을 실제로 여러 틱에 걸쳐 통과하며 `stage`가 0→3으로 올라가는 것을 재현하는 테스트. 스테이션 재고가 0일 때 제품이 그 자리에 멈추고, 헬퍼가 보충한 뒤(테스트에서 직접 `part_inventory`를 채워) 자동으로 조립이 재개되는 테스트. 라인 시작점이 비었을 때 `DeliverFrame` 요청이 정확히 한 번만 큐에 들어가고 중복 생성되지 않는 테스트(§6 뮤테이션 대상). 헬퍼 최소 1명 하한 테스트.
- `tick_properties.rs`: 신규 `products_never_occupy_the_same_cell` proptest(§7).
- `main.rs`: `detect_completed_assemblies` 순수 함수 유닛테스트(기존 `detect_completed_placements` 스타일).
- REST/WS 통합테스트: 컨베이어를 켜고 실제로 제품이 여러 틱에 걸쳐 이동→조립→완성까지 하는 것을 확인, 완성 시 생산량이 정확히 오르는지 확인. `StationView`/`ProductView`가 실제 WS 메시지에 올바르게 실리는지 확인.
- 클라이언트: 일자 벨트 렌더링이 서버 `belt_cell` 좌표와 항상 일치하는지 확인하는 테스트(지난 U자 벨트 그리드 크기 불일치 버그 재발 방지 목적, §7 위험 항목). 제품 `stage`별로 다른 스프라이트가 그려지는지 vitest 단위 테스트. 스테이션 재고 0일 때 경고색 표시. Playwright E2E로 실제 브라우저에서 프레임 투입→3단계 조립→완성품 반출까지 전체 사이클 한 바퀴 확인.

## 12. 영향받는 파일

- `server/src/sim.rs` — `Product`, `Station`, `RobotRole`, `SimState.products`/`stations`, 제품 전진/조립 틱 로직, `ASSEMBLY_TICKS`/`HELPER_PICKUP_TICKS`/`HELPER_DROP_TICKS`/`STATION_MAX_INVENTORY` 상수, 기존 `work_points`/U자 `belt_cells` 및 관련 테스트 삭제.
- `server/src/game_state.rs` — 조립 로봇 3대 자동 생성(그리드/로봇 수 설정과 무관), `SetRobotCount`가 헬퍼 수만 조절하도록 변경 + 하한 1.
- `server/src/protocol.rs`/`delta.rs` — `RobotView.role`, `StationView`, `ProductView` 추가.
- `server/src/main.rs` — `detect_completed_assemblies`, 헬퍼 작업 큐 배정 로직.
- `server/tests/tick_properties.rs` — `products_never_occupy_the_same_cell` proptest.
- `client/src/render/canvas.ts` — 일자 벨트 렌더링(U자 `isConveyorCell` 대체), 제품 단계별 드론 스프라이트, 스테이션 재고 경고 표시.
- `client/src/net/protocol.ts` — `RobotView.role`, `StationView`, `ProductView` 타입 추가.
- `client/src/ui/sidebar.ts` — "로봇 수" → "헬퍼 로봇 수" 라벨, `-` 버튼 하한 1.
- `README.md`/`docs/KANBAN.md` — 완료 후 기존 컨벤션대로 갱신.
