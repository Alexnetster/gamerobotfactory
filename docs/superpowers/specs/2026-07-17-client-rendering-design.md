# 클라이언트 렌더링 설계 (Plan 4)

브레인스토밍 세션(2026-07-17, 비주얼 컴패니언 사용)에서 확정된 설계. 서버가 계산한 시뮬레이션 상태를 아이소메트릭 캔버스로 렌더링하고, 최소한의 조작 UI(사이드바)로 커맨드를 보내는 웹 클라이언트를 만든다.

## 왜 이 기능인가

이 프로젝트의 1차 목표는 백엔드/서버 엔지니어링 역량 시연이다(`docs/robot-arm-conveyor-game-design.md` 참고). 지금까지 서버는 완성됐지만 실제로 동작을 "보여줄" 방법이 `wscat`으로 JSON을 눈으로 읽는 것뿐이었다. 클라이언트는 그 자체로 새로운 백엔드 역량을 증명하지는 않지만(설계문서가 명시하듯 클라이언트는 판단 로직을 갖지 않는다), **서버가 이미 증명한 것(결정적 시뮬레이션, 델타 동기화, 재접속, 장애 격리)을 리뷰어가 실제로 눈으로 확인할 수 있게 만드는 마지막 조각**이다.

## 스코프 (v1)

설계문서(`docs/robot-arm-conveyor-game-design.md` line 117-122, v1 범위 표)가 요구하는 항목을 전부 한 번에 포함한다:

- 아이소메트릭 투영 렌더링 (바운딩박스 기준 z-order)
- 서버 상태 보간(interpolation)/외삽(extrapolation)
- 상태/센서 시각화 HUD (배터리=내구도, 현재 작업, 경로 디버그 라인)
- 전역 컨트롤(컨베이어 on/off, 로봇 수) + 로봇 선택/팔 동작/수리 커맨드

포함하지 않는 것(v2 이상, 설계문서의 v2 백로그와 일치):
- 다중 관측자 동시 접속 UI(v1 서버가 애초에 단일 오퍼레이터 세션 전제)
- 데모 영상/Docker 배포(Plan 5)
- 사운드, 애니메이션 폴리싱(걷기 사이클/팔 움직임은 서버가 보내는 `leg_cycle_progress`/포즈를 그대로 그리기만 하고, 별도의 클라이언트 측 연출은 추가하지 않음)

## 서버 쪽 변경 (Plan 4에 포함되는 선행 작업)

브레인스토밍 중 두 가지 사실이 확인됐다:

1. 경로 디버그 라인을 그리려면 서버가 로봇의 계획된 경로를 와이어로 노출해야 한다. `Robot` 구조체엔 이미 `path: Vec<CellId>` 필드가 있지만(`server/src/sim.rs:57`) `RobotView`엔 없다.
2. 마스터 설계문서(line 120)의 "팔이 인접 타일 위로 뻗을 수 있다"는 전제가 성립하려면 로봇이 어느 방향을 보는지(facing)와 팔이 실제로 어디를 향하는지(arm pose)가 필요한데, `ik.rs`/`posture.rs`는 순수 함수로 존재하고 자기 자신의 테스트만 통과할 뿐 `sim_core::sim::tick()`이나 `RobotView` 어디에서도 실제로 호출되지 않는다(`grep` 결과 호출부가 각 파일 `mod tests` 안뿐). `Robot`엔 방향(facing) 필드도 없다. 또한 `robot.pose`(Standing/Crouching)는 `sim.rs:74`에서 항상 `Standing`으로 초기화된 뒤 프로덕션 코드 어디서도 바뀌지 않는다 — "낮은 컨베이어는 웅크려서" 같은 자세 전환도 미구현 상태다. `Conveyor`(`game_state.rs:4-6`)는 `{running: bool}` 하나뿐이라 컨베이어에 공간적(칸별) 개념 자체가 없으므로, "칸마다 높이가 다른 컨베이어" 같은 새 시뮬레이션 메커닉은 발명하지 않는다(사용자 확인: 자세는 위치가 아니라 **task로만** 결정하면 충분 — "이동/대기 중엔 서고, 작업 중엔 적당히 웅크리는" 정도).

이 확인을 바탕으로 서버 쪽 변경 범위를 다음과 같이 최소화해서 확정한다 — **`sim_core`(결정성이 걸린 핵심)에 새로 추가되는 진짜 상태는 `facing` 하나뿐**이고, 자세/팔 각도는 이미 안정적으로 결정된 값(`task`, `facing`, `pos`)에서 매 요청마다 다시 계산하는 **프로토콜 계층의 순수 변환**(기존 `durability_remaining`이 `wear_ratio()`에서 매번 계산되는 것과 같은 패턴)으로 처리해 proptest/결정성에 새 위험을 추가하지 않는다.

### 1) `sim_core`: `facing` 필드 추가 (유일한 신규 저장 상태)

```rust
// server/src/sim.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction { North, East, South, West }

impl Direction {
    /// 로봇이 `from`에서 `to`로 한 칸 이동했을 때의 방향. 그리드는
    /// 4방향 이동만 지원하므로(`Grid::neighbors`, `grid.rs:33-39`)
    /// 대각선 케이스는 없다. 이동이 없으면(from == to) 기존 방향을 유지한다.
    pub fn from_move(from: CellId, to: CellId) -> Option<Direction> {
        match (to.0 - from.0, to.1 - from.1) {
            (1, 0) => Some(Direction::East),
            (-1, 0) => Some(Direction::West),
            (0, 1) => Some(Direction::North),
            (0, -1) => Some(Direction::South),
            _ => None,
        }
    }
}

pub struct Robot {
    // ...기존 필드...
    pub facing: Direction, // 스폰 시 기본값 East(ik.rs의 "타겟이 원점이면 +x를 향한다"는 기존 폴백 방향과 일치)
}
```

**주의**: `plan_robot`(`server/src/sim.rs:202-239`)이 계획하는 `next.pos`는 아직 확정이 아니다 — `tick()`이 그 다음에 `resolve_intents`로 타이브레이크를 돌려서, 진 로봇은 `final_pos`가 다시 원래 칸으로 되돌아간다(`sim.rs:183-185`). 그래서 `facing`을 `plan_robot` 안에서 갱신하면, 타이브레이크에서 진 로봇이 "시도했지만 실제로는 안 한 이동" 방향으로 잘못 회전해버리는 버그가 생긴다. 실제 최종 위치가 확정되는 지점은 `tick()`의 `new_robots` 매핑(`sim.rs:178-197`)이고, 이미 정확히 같은 조건(`robot.pos != original.pos`)으로 `leg_cycle_progress`를 갱신하고 있다(`sim.rs:192-194`) — `facing`도 같은 지점, 같은 조건으로 갱신한다:

```rust
// sim.rs:183-196, tick() 안의 new_robots 매핑 — 기존 leg_cycle_progress 갱신 바로 옆에 추가
.map(|((original, mut robot), final_pos)| {
    let lost_tiebreak = final_pos != robot.pos;
    robot.pos = final_pos;
    if lost_tiebreak {
        robot.path.clear();
        robot.ticks_until_repath = 0;
    }
    if robot.pos != original.pos {
        robot.leg_cycle_progress = (robot.leg_cycle_progress + LEG_CYCLE_SPEED).rem_euclid(1.0);
        if let Some(dir) = Direction::from_move(original.pos, robot.pos) {
            robot.facing = dir;
        }
    }
    robot
})
```

정지 중(이동이 없으면)엔 마지막 방향을 유지 — 새 결정성 위험 없음(순수하게 확정된 `pos` 변화로부터 유도되는 값이라 기존 더블 버퍼링/타이브레이크 불변식에 영향 없음, 오히려 타이브레이크 이후 값을 쓰므로 안전).

`robot.pose: BodyPose` 필드는 **제거**한다 — 지금까지 한 번도 `Standing`이 아닌 값으로 바뀐 적이 없는 죽은 상태였고, 아래 2)에서 `task`로부터 매번 유도하는 값으로 대체되기 때문이다.

### 2) `protocol.rs`: 경로/방향/팔 각도를 `RobotView`에 노출 (순수 변환, 신규 저장 상태 없음)

```rust
// server/src/protocol.rs
pub struct RobotView {
    // ...기존 필드(id, pos, leg_cycle_progress, task, status, durability_remaining)...
    pub pose: WirePose,              // 기존 필드, 계산 방식만 바뀜(아래 참고)
    pub path: Vec<WireCellId>,       // 신규
    pub facing: WireDirection,       // 신규 — North | East | South | West, WireTask/WirePose와 같은 태그 없는 문자열 인코딩
    pub arm_pose: WireArmPose,       // 신규 — { shoulder_angle: f32, elbow_angle: f32 }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct WireArmPose {
    pub shoulder_angle: f32,
    pub elbow_angle: f32,
}

// 클라이언트와 반드시 동기화해서 유지해야 하는 고정 상수(와이어로 안 보냄 — 안 바뀌는 값이라 대역폭 낭비).
const WORK_TARGET_HEIGHT: f32 = 0.75; // 작업 대상의 월드 기준 높이(컨베이어 칸별 차이 없음 — task 확인 결과, 위치 기반 높이는 불필요)
const WORK_TARGET_FORWARD: f32 = 0.6; // 몸체 전방으로 팔이 뻗는 거리(칸 하나=1.0 기준, 즉 인접 칸을 완전히 넘어가진 않음)
const UPPER_ARM_LEN: f32 = 0.7;
const LOWER_ARM_LEN: f32 = 0.6;
const IDLE_ARM_POSE: WireArmPose = WireArmPose { shoulder_angle: 0.0, elbow_angle: 0.0 }; // 대기 시 접힌 기준 자세, 시각적 튜닝은 구현 시 조정 가능

fn arm_pose_for(robot: &Robot) -> WireArmPose {
    if robot.task == Task::Idle {
        return IDLE_ARM_POSE;
    }
    let pose = if robot.task == Task::Idle { BodyPose::Standing } else { BodyPose::Crouching };
    let local_target = world_target_to_body_local(WORK_TARGET_HEIGHT, WORK_TARGET_FORWARD, pose);
    let solved = solve_two_bone_ik(UPPER_ARM_LEN, LOWER_ARM_LEN, local_target);
    WireArmPose { shoulder_angle: solved.shoulder_angle, elbow_angle: solved.elbow_angle }
}

impl From<&Robot> for RobotView {
    fn from(r: &Robot) -> RobotView {
        RobotView {
            // ...기존 필드...
            pose: (if r.task == Task::Idle { BodyPose::Standing } else { BodyPose::Crouching }).into(),
            path: r.path.iter().map(|&c| c.into()).collect(),
            facing: r.facing.into(),
            arm_pose: arm_pose_for(r),
        }
    }
}
```

`WireDirection`은 `WireTask`/`WirePose`와 동일하게 데이터 없는 plain enum(JSON에서 `"East"` 같은 문자열)으로 직렬화한다.

**클라이언트가 알아야 할 상수**: `UPPER_ARM_LEN`/`LOWER_ARM_LEN`(`0.7`/`0.6`)은 각도를 실제 화면 좌표(팔꿈치/손목 위치)로 되돌리는 데 필요하므로, 클라이언트 쪽에도 같은 값을 상수로 둔다(와이어로 안 보내는 이유: 튜닝 상수라 바뀔 일이 거의 없고, 매 메시지 싣기엔 낭비). `render/projection.ts`의 순수 함수 테스트에서 서버 `ik.rs`의 `forward_kinematics`와 동일한 공식(상수만 다름)을 TS로 재구현해 사용한다.

**델타 압축 영향**: `path`는 재계획 주기(`REPATH_INTERVAL`)마다만 바뀌고, `facing`은 `pos`가 바뀔 때만 같이 바뀌며, `arm_pose`는 `task`/`facing`/`pose`가 안 바뀌면(제자리에서 계속 작업 중이어도) 매번 같은 값이 나오므로 `compute_delta`의 `PartialEq` 비교에서 자연히 델타 밖으로 빠진다 — `durability_remaining`처럼 별도 양자화가 필요 없다(누적되며 계속 drift하는 값이 아니기 때문).

`delta.rs`의 `compute_delta`는 `RobotView` 전체를 `PartialEq`로 비교하므로 이 필드 추가만으로 자동으로 델타 비교 대상에 포함된다 — 별도 로직 추가 불필요. 기존 `delta.rs`/`protocol.rs` 테스트의 `RobotView`/`Robot` 리터럴 생성 헬퍼(예: `delta.rs:39`의 `robot_view(id, x)`, `sim.rs`의 `Robot::new`)는 새 필드(`facing`, `path`, `arm_pose`)를 채우도록 갱신해야 한다.

## 클라이언트 아키텍처

```
client/
  src/
    net/      - WebSocket 연결, 프로토콜 타입(서버 protocol.rs와 대칭), 세션/재접속
    state/    - 델타 병합(로컬 미러), 보간/외삽 스토어
    render/   - 아이소메트릭 투영, z-order 정렬, 캔버스 드로우
    ui/       - 사이드바 패널(바닐라 TS + DOM)
    main.ts   - 부트스트랩(WS 연결 → 상태 스토어 → requestAnimationFrame 루프)
  tests/
    unit/         - vitest: 순수 함수
    integration/  - vitest + `ws` 패키지: 실제 서버 바이너리 + 진짜 WS
    e2e/          - Playwright: 실제 서버 + 빌드된 클라이언트 + 실제 브라우저
  index.html
  vite.config.ts
  package.json
```

**스택 선택**:
- **Vite + TypeScript** — 설계문서가 명시.
- **네이티브 `WebSocket`/Canvas 2D API** — 별도 렌더링 라이브러리(PixiJS 등) 없이 직접 그린다. 씬 복잡도가 낮고(로봇 20~50대, 단순 도형), 포트폴리오 스코프에서 "왜 이 라이브러리를 썼는지"를 설명할 필요가 없는 최소 의존성을 택함.
- **프레임워크 없이 바닐라 DOM** — 사이드바가 버튼/텍스트 몇 개 수준이라 React/Vue 등은 과함. 상태 변경 시 필요한 DOM 노드만 직접 갱신.
- **npm** — 특별한 이유 없으면 기본값.

## 프로토콜 연동

### 로컬 미러 상태

서버의 `Snapshot`/`Delta`를 그대로 받아 "현재 알려진 전체 로봇 상태 맵"을 클라이언트에 재구성한다:
- `Snapshot` 수신 시 맵을 통째로 교체.
- `Delta` 수신 시 `changed_robots`로 덮어쓰고 `removed_robot_ids`로 삭제.

서버의 델타 프로토콜을 클라이언트에서 그대로 재생하는 거울이다 — 서버 쪽 `compute_delta`/`apply_delta` 로직과 대칭되는 순수 함수로 구현해 유닛테스트 대상으로 삼는다.

### 보간/외삽

- 최근 2개 틱의 로봇 상태(prev/curr)를 보관.
- 매 rAF: `alpha = clamp((now - curr를_받은_시각) / 50ms, 0, 1)`로 prev→curr 사이 위치/포즈를 선형 보간.
- 다음 틱이 50ms를 넘겨 지연되면, curr 시점의 속도(= curr와 prev의 위치 차이 / 50ms)로 alpha 1 이후에도 짧게 외삽. 실제 틱이 도착하면 그 시점 위치로 스냅 보정(순간이동처럼 보일 수 있음을 감수 — 서버 권위 원칙상 클라이언트가 임의로 미래를 더 오래 예측하지 않는다).
- 브라우저 탭이 백그라운드로 가서 rAF가 스로틀되는 동안엔 외삽이 과도해지지 않도록 alpha를 그냥 1로 클램프(마지막 알려진 위치에 정지).

### 세션/재접속

- 최초 연결의 `Snapshot.session_id`를 `sessionStorage`에 저장.
- 연결이 끊기면 지수 백오프로 재연결을 시도하고, 성공하면 저장된 `session_id`로 `Resume` 커맨드를 보낸다. `ResumeAck.resumed`에 따라 사이드바에 "재접속됨"/"새 세션 시작"을 표시.
- `sessionStorage`를 쓰므로 같은 탭의 새로고침/재접속에서만 이어진다. 새 탭은 항상 새 세션(서버 v1이 단일 오퍼레이터 세션 전제이므로 여러 탭이 세션을 공유할 이유가 없다 — 의도된 동작).
- 서버가 Lagged로 전체 `Snapshot`을 다시 보내는 경우, 클라이언트는 로컬 미러 전체와 prev/curr 보간 상태를 통째로 리셋한다.

## 렌더링

- **투영**: 그리드 좌표(x, y) → 아이소메트릭 화면 좌표로 변환하는 순수 함수. 고정 각도(설계문서 line 119).
- **팔 좌표 복원**: `RobotView.facing` + `RobotView.arm_pose`(shoulder_angle/elbow_angle) + 클라이언트에도 미러링된 `UPPER_ARM_LEN`/`LOWER_ARM_LEN` 상수로, 서버 `ik.rs::forward_kinematics`와 동일한 공식을 TS로 재구현해 어깨/팔꿈치/손목의 몸체-로컬 좌표를 얻는다. 이 로컬 좌표(전방/높이)를 `facing`으로 월드 방향으로 회전시킨 뒤 로봇의 월드 위치에 더해 실제 화면 좌표를 계산한다.
- **z-order**: 정렬 키는 로봇 몸체 칸이 아니라 (몸체+팔이 차지하는) 바운딩 박스의 가장 먼 안쪽 모서리 기준 `x + y`(설계문서 line 120). 위에서 복원한 손목 월드 좌표까지 포함해 바운딩박스를 계산 — `arm_pose`가 실제 서버 계산값이므로 팔이 `facing` 방향의 인접 칸 쪽으로 넘어가는 경우를 정확히 반영한다(이전 초안은 이 데이터가 와이어에 없다는 걸 놓치고 있었음 — 위 "서버 쪽 변경" 절에서 보강).
- **드로우 순서**: 매 프레임 컨베이어 배경 → 바닥 타일 → z-order 정렬된 로봇(몸체+다리+팔) 순으로 그린다.
- **비주얼 스타일**: 브레인스토밍에서 확정된 "쉐이딩된 의사-3D 블록" — 단색 대신 그라디언트+그림자로 타일/로봇에 입체감을 준다.

### 컨베이어 시각화 (순수 장식, 서버 변경 없음)

브레인스토밍에서 확인: `Conveyor`는 서버에 공간적 개념이 없는 전역 on/off 스위치([위 "서버 쪽 변경" 절](#서버-쪽-변경-plan-4에-포함되는-선행-작업) 참고)이고, 마스터 설계문서 line 34가 예시로 든 "정지된 컨베이어 구간이 장애물"이라는 아이디어도 실제로는 구현돼 있지 않다(로봇 이동 장애물은 여전히 "다른 로봇의 현재 위치"뿐). v1에서는 이 갭을 메우지 않는다 — 컨베이어는 **클라이언트가 그리는 고정 배경 장식**일 뿐, 로봇의 이동/작업 가능 여부에 어떤 영향도 주지 않는다(로봇은 지금처럼 그리드 전체를 자유롭게 이동).

- **모양**: 그리드의 위/왼쪽/아래 세 변을 따라 흐르는 U자, **오른쪽(사이드바와 맞닿은 쪽)이 열림**(브레인스토밍에서 확정) — 벨트가 오퍼레이터 쪽으로 흘러나오는 느낌.
- **구현**: `render/canvas.ts`에 고정된 셀 좌표 집합(그리드 크기에 따라 계산되는 테두리 3면)을 별도 타일 스타일(스트라이프/화살표 오버레이)로 그린다. `conveyor.running`이 `false`면 애니메이션(흐르는 스트라이프)을 멈추고 정적인 색으로 표시 — 유일하게 서버 상태(`ConveyorView.running`)와 연동되는 지점.
- **그리드 크기**: U자가 이소메트릭 각도에서 알아볼 수 있을 만큼 충분히 커야 한다 — 최소 7x6 이상 권장(브레인스토밍 목업 기준). 정확한 크기는 구현 태스크에서 시각적으로 확인하며 조정.

### 클라이언트 성능 목표

서버는 이미 실측 가능한 성능 목표(틱 처리시간 p99 < 10ms, `tick_duration_seconds` 히스토그램)를 갖고 있다(`docs/robot-arm-conveyor-game-design.md:95-102`). "목표를 문서에만 적어두고 측정 수단이 없는" 함정(README 참고)을 클라이언트에서도 반복하지 않기 위해, 렌더링 쪽에도 가벼운 목표를 둔다:

- 로봇 50대(서버 성능 목표 상한)를 렌더링하는 상태에서 60fps 근처를 유지한다 — 정밀한 프레임타임 히스토그램까지는 v1 스코프가 아니고, `requestAnimationFrame` 콜백 간 델타 시간을 콘솔/HUD에 간단히 로깅해 실측치를 README에 남기는 정도로 충분하다(서버 쪽 "실측 그래프/로그로 근거를 남긴다"는 원칙과 동일).
- 이 목표는 Plan 4 마지막 태스크(전체 검증)에서 로봇 수를 늘려가며 직접 확인한다 — Plan 5의 "성능 목표 실측"(서버 p99)과는 별개로, 클라이언트 쪽 실측은 Plan 4 안에서 끝낸다.

## UI/HUD

레이아웃: **우측 고정폭 사이드바** + 좌측 전체를 채우는 캔버스(브레인스토밍에서 확정).

- **전역 컨트롤**: 컨베이어 on/off 토글, 로봇 수 스텝퍼(+/-, 서버가 200으로 클램프하므로 클라이언트는 별도 상한 검증 없이 그대로 전송), 연결 상태(🟢 연결됨 / 🔴 재연결 중 + 스피너).
- **선택된 로봇 패널**: 캔버스에서 로봇 클릭 → `SelectRobot` 전송. 배터리(`durability_remaining`)/작업(`task`)/상태(Operational/Failed/`Repairing{remaining_ticks}`)/팔 동작 버튼(Idle/Picking/Placing, `TriggerArmAction`)/`Failed`일 때만 활성화되는 "수리" 버튼(`RepairRobot`).
- **경로 디버그 토글**: 사이드바의 체크박스/토글 버튼, **기본값 꺼짐**(로봇이 여러 대일 때 선이 겹쳐 어수선해지는 것 방지). 켜면 선택된 로봇뿐 아니라 화면에 보이는 모든 로봇의 `path`를 캔버스에 얇은 선으로 그린다.

## 테스트 전략 (서버와 동일한 엄격도로 적용)

서버 쪽에서 지금까지 TDD+뮤테이션 테스트로 실제 버그를 여러 번 잡아낸 규율을 클라이언트에도 동일하게 적용한다(`CLAUDE.md`가 요구하는 "self-report만 믿지 않는다" 원칙).

1. **단위(vitest)**: 투영 함수, 보간/외삽 계산(alpha 클램프 포함), 델타 병합(스냅샷→델타 적용 후 로컬 미러가 정확한지, `removed_robot_ids` 처리 포함), z-order 정렬 키 계산, `forward_kinematics` TS 재구현(서버 `ik.rs`의 기존 테스트 케이스 — 예: `reaches_target_within_arm_length` — 와 같은 입력/기대값으로 대조해 두 구현이 어긋나지 않는지 확인). 각 항목은 뮤테이션 테스트(계산식을 일부러 깨서 테스트가 실패하는지)로 공허함을 배제한다.
2. **서버 단위테스트 보강(Rust, `server/src/sim.rs`/`protocol.rs`)**: `Direction::from_move`가 4방향 각각과 "이동 없음" 케이스에서 옳은 값을 내는지, 로봇이 실제로 이동할 때 `facing`이 갱신되는지, `task`가 Idle↔Picking/Placing으로 바뀔 때 `RobotView.pose`/`arm_pose`가 기대한 값(대기 자세 vs IK 해)으로 바뀌는지, `task`/`facing`이 안 바뀌면 `arm_pose`가 매번 동일해서(따라서 델타에 실리지 않아서) 압축이 깨지지 않는지.
3. **통합(vitest + `ws` 패키지)**: 서버 통합테스트와 동일한 패턴 — `cargo build`로 만든 실제 서버 바이너리를 자식 프로세스로 기동하고, 진짜 `ws` 클라이언트로 커맨드 시퀀스를 실행해 클라이언트 상태 레이어(로컬 미러, 델타 적용 결과)가 기대와 일치하는지 검증. 모킹 없음.
4. **E2E(Playwright)**: 실제 서버 + 빌드된 클라이언트 + 실제 브라우저. 전체 페이지 스크린샷 diff는 폰트/OS 차이로 flaky하므로 쓰지 않고, 특정 좌표의 캔버스 픽셀 샘플링(`getImageData`)과 사이드바 DOM 텍스트 단언으로 "로봇이 실제로 그 자리에 그려졌는지"를 결정적으로 검증한다. 예: 로봇 1대 배치 → 알려진 초기 위치의 투영 좌표에서 픽셀이 배경색이 아닌지 확인, `SelectRobot` 클릭 후 사이드바에 해당 로봇 정보가 뜨는지 확인.

## 에러/엣지 케이스

- 잘못된 JSON/알 수 없는 서버 메시지: 서버의 "로그만 남기고 연결 유지" 정책과 대칭으로, 클라이언트도 파싱 실패 시 콘솔 경고만 남기고 연결을 유지한다(연결을 끊지 않음).
- 재연결 실패가 반복되는 경우(서버가 아예 다운): 지수 백오프 상한을 두고, 사이드바에 재연결 시도 횟수/상태를 계속 표시한다(무한 재시도이되 사용자가 인지 가능하게).

## 문서 갱신 의무 (Plan 4 마지막 태스크)

진행 방향 재점검(2026-07-17)에서 확인된 사실: 마스터 설계문서(`docs/robot-arm-conveyor-game-design.md:134-143`)의 v1 범위 표가 이미 stale하다 — 로봇 행이 "4족 보행+팔 IK+포즈"만 나열하고, 완료된 로봇 고장/수리 기능(2026-07-16 설계, 9개 태스크)이 반영 안 됨. Plan 4의 마지막 태스크(과거 Plan들과 같은 패턴 — "전체 검증 + 문서 갱신")에서 다음을 함께 고친다:

- v1 범위 표 로봇 행에 고장/수리 기능 추가.
- 같은 표/본문에 `RobotView.path`/`facing`/`arm_pose` 필드 추가와 "자세 전환이 이제 `task` 기반으로 실제 동작함"(이 문서의 "서버 쪽 변경" 절) 반영 — 마스터 설계문서 line 26-27의 자세 전환 서술이 이제 실제 코드와 일치하게 됨을 명시.
- README 프로토콜 표에도 `path`/`facing`/`arm_pose` 필드, 클라이언트 실행 방법(`client/` 디렉토리가 이제 존재) 추가.
- README "핵심 엔지니어링 결정"에 "IK/자세 계산이 처음으로 실제 틱/프로토콜에 연결됨"을 한 줄로 남길지 검토(그동안 죽어있던 코드가 실사용되는 시점이라 기록해둘 가치가 있음).

## 스코프 밖 (v2 이상)

- 다중 관측자 동시 접속에 따른 UI 분기(서버 v1이 단일 세션이므로 해당 없음)
- 클라이언트 측 예측(서버 권위를 넘어서는 로컬 시뮬레이션) — 지금의 보간/외삽은 이미 받은 상태 사이/직후만 다루고, 커맨드에 대한 낙관적 렌더링은 하지 않는다
- WebGL/PixiJS 등으로의 렌더러 교체(Canvas 2D로 충분한 스코프)
- 애니메이션 폴리싱, 사운드, 파티클 효과
