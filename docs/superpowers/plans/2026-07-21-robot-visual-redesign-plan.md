# 로봇 외형 리디자인 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 클라이언트 캔버스 렌더러가 그리는 로봇 모양을 평평한 사각형+일자 다리에서, 실루엣이 뚜렷한 Spot 스타일 4족 로봇(3/4 정면 각도, 몸통에서 안 떨어진 매끈한 관절 다리)으로 바꾼다.

**Architecture:** 위상→다리각도 매핑은 새 순수 함수(`client/src/render/gait.ts`)로 분리해 뮤테이션 테스트로 검증하고, `client/src/render/canvas.ts`의 `drawRobot`은 그 함수를 써서 다리/몸통/팔/센서를 새로 그린다. 서버·프로토콜 변경 없음 — 기존 `leg_cycle_progress`/`arm_pose`/`facing`/`task` 필드만 재사용한다.

**Tech Stack:** TypeScript, Vite, vitest(단위), Playwright(E2E), Canvas 2D API.

**참고 설계 문서:** [`docs/superpowers/specs/2026-07-21-robot-visual-redesign-design.md`](../specs/2026-07-21-robot-visual-redesign-design.md)

---

## 사전 확인 사항 (구현자가 알아야 할 기존 코드)

- `client/src/render/canvas.ts`의 `drawRobot`(현재 173~230행)이 로봇 그리기 전체를 담당한다. `drawScene`이 로봇마다 이 함수를 호출한다.
- `client/src/render/projection.ts`의 `elbowWorldOffset`/`wristWorldOffset`은 서버 IK 결과(`arm_pose.shoulder_angle`/`elbow_angle`)를 월드 그리드 좌표로 변환한다 — **이 함수들은 안 바뀐다**, `drawRobot`이 그 결과를 어디에 그리는지만 바뀐다.
- `InterpolatedRobot` 타입(`client/src/state/interpolation.ts`)은 `leg_cycle_progress: number`, `task: 'Idle' | 'Picking' | 'Placing'`, `pose: 'Standing' | 'Crouching'`, `facing`, `arm_pose` 필드를 이미 갖고 있다 — 전부 그대로 재사용, 새 필드 불필요.
- `client/tests/unit/canvas.test.ts`의 `robotAt()` 헬퍼가 테스트용 `InterpolatedRobot`을 만든다 — 이 태스크에서 안 바뀐다(그 파일이 테스트하는 `isConveyorCell`/`sortRobotsForDrawing`/`conveyorFlowDirection`은 이 플랜과 무관).
- `client/tests/e2e/render.spec.ts`의 "draws a spawned robot with body-colored pixels" 테스트가 현재 몸체 색(`#ffd27a`~`#d99a2e`, 주황/금색 그라디언트, R>G)을 캔버스 픽셀에서 찾는다 — 몸체 색이 바뀌므로 Task 4에서 이 테스트를 새 색에 맞게 고친다.

---

### Task 1: 위상→다리각도 매핑 함수 (`gait.ts`)

**Files:**
- Create: `client/src/render/gait.ts`
- Test: `client/tests/unit/gait.test.ts`

- [ ] **Step 1: 실패하는 테스트 작성**

`client/tests/unit/gait.test.ts`:

```typescript
import { describe, it, expect } from 'vitest'
import { legAnglesForPhase } from '../../src/render/gait'

describe('legAnglesForPhase', () => {
  it('디딤 시작(위상 0)에서 엉덩이는 앞으로 기울고 무릎은 편 상태', () => {
    const { hipDeg, kneeDeg } = legAnglesForPhase(0)
    expect(hipDeg).toBeCloseTo(9, 5)
    expect(kneeDeg).toBeCloseTo(0, 5)
  })

  it('디딤 끝(위상 0.6)에서 엉덩이는 뒤로 기울고 무릎은 여전히 편 상태', () => {
    const { hipDeg, kneeDeg } = legAnglesForPhase(0.6)
    expect(hipDeg).toBeCloseTo(-8, 5)
    expect(kneeDeg).toBeCloseTo(0, 5)
  })

  it('디딤 구간 중간(위상 0.3)에서 무릎은 계속 0도(편 상태)', () => {
    const { hipDeg, kneeDeg } = legAnglesForPhase(0.3)
    expect(hipDeg).toBeCloseTo(0.5, 5)
    expect(kneeDeg).toBe(0)
  })

  it('흔듦 구간 정점(위상 0.8)에서 무릎이 최대로 굽음(발을 들어올림)', () => {
    const { hipDeg, kneeDeg } = legAnglesForPhase(0.8)
    expect(hipDeg).toBeCloseTo(0.5, 5)
    expect(kneeDeg).toBeCloseTo(28, 5)
  })

  it('흔듦 구간 후반(위상 0.9)에서 엉덩이는 앞으로 복귀 중, 무릎은 절반쯤 펴짐', () => {
    const { hipDeg, kneeDeg } = legAnglesForPhase(0.9)
    expect(hipDeg).toBeCloseTo(4.75, 5)
    expect(kneeDeg).toBeCloseTo(14, 5)
  })

  it('위상 1.0은 위상 0과 같다(한 주기가 매끈하게 이어짐)', () => {
    const atOne = legAnglesForPhase(1.0)
    const atZero = legAnglesForPhase(0)
    expect(atOne.hipDeg).toBeCloseTo(atZero.hipDeg, 5)
    expect(atOne.kneeDeg).toBeCloseTo(atZero.kneeDeg, 5)
  })

  it('음수 위상도 정규화되어 0.9와 같은 결과를 낸다', () => {
    const negative = legAnglesForPhase(-0.1)
    const equivalent = legAnglesForPhase(0.9)
    expect(negative.hipDeg).toBeCloseTo(equivalent.hipDeg, 5)
    expect(negative.kneeDeg).toBeCloseTo(equivalent.kneeDeg, 5)
  })

  it('디딤 구간(위상 0~0.6) 전체에서 무릎은 항상 0도 — 흔듦 구간에서만 굽는다는 비대칭 타이밍의 핵심 불변식', () => {
    for (const p of [0, 0.1, 0.2, 0.4, 0.59]) {
      expect(legAnglesForPhase(p).kneeDeg).toBe(0)
    }
  })
})
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cd client && npm test -- gait`
Expected: FAIL — `Cannot find module '../../src/render/gait'`

- [ ] **Step 3: 구현 작성**

`client/src/render/gait.ts`:

```typescript
export interface LegAngles {
  hipDeg: number
  kneeDeg: number
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t
}

const STANCE_END = 0.6
const KNEE_PEAK = 0.8
const HIP_STANCE_START_DEG = 9
const HIP_STANCE_END_DEG = -8
const KNEE_PEAK_DEG = 28

/** 위상(0~1, 걸음 한 주기)을 (엉덩이, 무릎) 각도(도)로 매핑한다.
 *
 * 0~60%(디딤): 발이 바닥에 붙은 채 엉덩이가 천천히 뒤로 기울며 몸통을
 * 앞으로 옮긴다 — 무릎은 편 상태(0도) 유지.
 * 60~100%(흔듦): 엉덩이가 빠르게 앞으로 복귀하고, 무릎이 굽혀져(최대
 * `KNEE_PEAK_DEG`) 발을 들어올렸다가 다시 편다.
 *
 * 대칭 사인파 하나로 다리를 흔들면 "미끄러지듯 이동한다"는 인상을 준다
 * (2026-07-21 렌더링 브레인스토밍에서 목업으로 재현하고 고친 문제) —
 * 디딤/흔듦의 비대칭 타이밍과 무릎이 흔듦 구간에서만 굽는다는 점이 그
 * 문제를 해결하는 핵심이다.
 */
export function legAnglesForPhase(phase: number): LegAngles {
  const p = ((phase % 1) + 1) % 1

  const hipDeg = p <= STANCE_END
    ? lerp(HIP_STANCE_START_DEG, HIP_STANCE_END_DEG, p / STANCE_END)
    : lerp(HIP_STANCE_END_DEG, HIP_STANCE_START_DEG, (p - STANCE_END) / (1 - STANCE_END))

  let kneeDeg: number
  if (p <= STANCE_END) {
    kneeDeg = 0
  } else if (p <= KNEE_PEAK) {
    kneeDeg = lerp(0, KNEE_PEAK_DEG, (p - STANCE_END) / (KNEE_PEAK - STANCE_END))
  } else {
    kneeDeg = lerp(KNEE_PEAK_DEG, 0, (p - KNEE_PEAK) / (1 - KNEE_PEAK))
  }

  return { hipDeg, kneeDeg }
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cd client && npm test -- gait`
Expected: PASS (8 tests)

- [ ] **Step 5: 뮤테이션 테스트 — 비대칭 타이밍이 실제로 검증되는지 확인**

`STANCE_END`를 `0.5`로 바꿔서 저장한 뒤 같은 명령을 다시 실행 — "디딤 구간 중간(위상 0.3)" 테스트와 "디딤 끝(위상 0.6)" 테스트가 실제로 실패하는지 확인한다(둘 다 정확한 경계값에 의존하므로). 확인 후 `0.6`으로 되돌리고 다시 테스트가 통과하는지 확인한다.

- [ ] **Step 6: 커밋**

```bash
git add client/src/render/gait.ts client/tests/unit/gait.test.ts
git commit -m "feat: add leg phase-to-angle gait mapping with asymmetric stance/swing timing"
```

---

### Task 2: 몸통 + 다리 다시 그리기 (Spot 스타일, 이음매 없이 부착)

**Files:**
- Modify: `client/src/render/canvas.ts:173-230` (`drawRobot` 함수)

- [ ] **Step 1: `drawRobot` 상단에 새 상수 추가하고 다리/몸통 그리기 교체**

`client/src/render/canvas.ts` 파일 최상단 import에 `legAnglesForPhase` 추가:

```typescript
import { gridToScreen, zOrderKey, wristWorldOffset, elbowWorldOffset, TILE_WIDTH, TILE_HEIGHT, RENDER_SCALE } from './projection'
import { legAnglesForPhase } from './gait'
import type { InterpolatedRobot } from '../state/interpolation'
import type { ConveyorView } from '../net/protocol'
```

`drawRobot` 함수 바로 위(232행이 되기 전)에 상수 블록 추가:

```typescript
const BODY_WIDTH = 26
const BODY_HEIGHT = 20
const BODY_DEPTH_X = 4 // 위/오른쪽 슬리버가 오른쪽 위로 밀리는 정도(3/4 정면 원근감)
const BODY_DEPTH_Y = 4
const HIP_X_OFFSETS = [-10, 10, -5, 5] // 앞왼쪽, 앞오른쪽, 뒤왼쪽, 뒤오른쪽
const THIGH_LEN = 12
const SHIN_LEN = 16
// 엉덩이 관절점을 몸통 밑면보다 이만큼 위(안쪽)에 둔다 — 다리를 먼저 그리고
// 몸통을 나중에 그리므로, 몸통 사각형이 이 겹친 부분을 덮어서 다리가
// 몸통에서 안 떨어진 것처럼 보인다. 뒷다리 시작점이 몸통 바깥 허공이라
// 걷는 동안 눈에 띄게 떠 보이던 버그(2026-07-21 렌더링 브레인스토밍에서
// 실측)를 이 겹침으로 방지한다.
const LEG_BODY_OVERLAP = 4
const LEG_COLOR = '#454c54'
const SHOULDER_BLOCK_SIZE = 6
```

`drawRobot` 함수 전체를 아래로 교체:

```typescript
function drawRobot(ctx: CanvasRenderingContext2D, robot: InterpolatedRobot, selected: boolean): void {
  const screen = gridToScreen(robot.renderPos.x, robot.renderPos.y)
  const armPoseInput = {
    pos: robot.renderPos, facing: robot.facing, shoulderAngle: robot.arm_pose.shoulder_angle, elbowAngle: robot.arm_pose.elbow_angle,
  }
  const elbow = elbowWorldOffset(armPoseInput)
  const elbowScreen = gridToScreen(elbow.x, elbow.y)
  const wrist = wristWorldOffset(armPoseInput)
  const wristScreen = gridToScreen(wrist.x, wrist.y)
  const bodyLift = robot.pose === 'Crouching' ? 6 : 12 // 자세에 따른 몸체 높이(화면 픽셀, 튜닝 대상)

  ctx.save()
  ctx.translate(screen.x, screen.y)

  const bodyBottomY = -bodyLift
  const bodyTopY = bodyBottomY - BODY_HEIGHT
  const hipY = bodyBottomY - LEG_BODY_OVERLAP

  // 다리 4개 — 엉덩이→무릎→발을 각각 하나의 연속된 stroke path로 그린다
  // (beginPath/moveTo/lineTo/lineTo/stroke 한 번). 별도 조각을 이어붙이지
  // 않으므로 굽는 지점(무릎)에 색/굵기가 다른 이음매가 생길 수가 없다 —
  // "관절 마디가 시각적으로 끊어지면 안 됨" 제약(설계문서 §3-1)을 여러
  // 조각을 세심하게 맞추는 대신 애초에 조각을 하나로 만들어서 만족시킨다.
  ctx.strokeStyle = LEG_COLOR
  ctx.lineWidth = 4
  ctx.lineCap = 'round'
  ctx.lineJoin = 'round'
  for (let i = 0; i < 4; i++) {
    const phase = (robot.leg_cycle_progress + i * 0.25) % 1
    const { hipDeg, kneeDeg } = legAnglesForPhase(phase)
    const hipRad = (hipDeg * Math.PI) / 180
    const shinRad = ((hipDeg + kneeDeg) * Math.PI) / 180
    const hipX = HIP_X_OFFSETS[i]
    const kneeX = hipX + THIGH_LEN * Math.sin(hipRad)
    const kneeY = hipY + THIGH_LEN * Math.cos(hipRad)
    const footX = kneeX + SHIN_LEN * Math.sin(shinRad)
    const footY = kneeY + SHIN_LEN * Math.cos(shinRad)

    ctx.beginPath()
    ctx.moveTo(hipX, hipY)
    ctx.lineTo(kneeX, kneeY)
    ctx.lineTo(footX, footY)
    ctx.stroke()
  }

  // 몸통 — 정면(큰 앞면) + 위/오른쪽 슬리버로 살짝 입체감을 주는 3/4 정면
  // 각도(설계문서 §2). 바닥 타일은 여전히 엄격한 아이소메트릭이지만, 로봇
  // 몸체만 이렇게 그려야 정면 실루엣이 뚜렷해진다.
  const bodyGradient = ctx.createLinearGradient(-BODY_WIDTH / 2, bodyTopY, BODY_WIDTH / 2, bodyBottomY)
  bodyGradient.addColorStop(0, '#6b7480')
  bodyGradient.addColorStop(1, '#5a636e')
  ctx.fillStyle = bodyGradient
  ctx.fillRect(-BODY_WIDTH / 2, bodyTopY, BODY_WIDTH, BODY_HEIGHT)

  ctx.fillStyle = '#454c54'
  ctx.beginPath()
  ctx.moveTo(BODY_WIDTH / 2, bodyTopY)
  ctx.lineTo(BODY_WIDTH / 2 + BODY_DEPTH_X, bodyTopY - BODY_DEPTH_Y)
  ctx.lineTo(BODY_WIDTH / 2 + BODY_DEPTH_X, bodyBottomY - BODY_DEPTH_Y)
  ctx.lineTo(BODY_WIDTH / 2, bodyBottomY)
  ctx.closePath()
  ctx.fill()

  ctx.fillStyle = '#6b7480'
  ctx.beginPath()
  ctx.moveTo(-BODY_WIDTH / 2, bodyTopY)
  ctx.lineTo(BODY_WIDTH / 2, bodyTopY)
  ctx.lineTo(BODY_WIDTH / 2 + BODY_DEPTH_X, bodyTopY - BODY_DEPTH_Y)
  ctx.lineTo(-BODY_WIDTH / 2 + BODY_DEPTH_X, bodyTopY - BODY_DEPTH_Y)
  ctx.closePath()
  ctx.fill()

  // 패널 이음선 강조 스트라이프(정면 상단)
  ctx.fillStyle = '#e8823a'
  ctx.fillRect(-BODY_WIDTH / 2, bodyTopY, BODY_WIDTH, 4)

  if (selected) {
    ctx.strokeStyle = '#ffffff'
    ctx.lineWidth = 2
    ctx.strokeRect(-BODY_WIDTH / 2, bodyTopY, BODY_WIDTH, BODY_HEIGHT)
  }

  // 센서 헤드 — 눈 색으로 "지금 팔이 작업 중인가"만 나타낸다(로봇의
  // Failed/Repairing 여부는 이미 정지 여부로 구분되므로 여기서는 안 다룸
  // — 설계문서 §5).
  ctx.fillStyle = '#3a4048'
  ctx.fillRect(-6, bodyTopY - 2, 12, 7)
  ctx.fillStyle = robot.task === 'Idle' ? '#8a8f96' : '#ffd23a'
  ctx.beginPath()
  ctx.arc(0, bodyTopY + 1.5, 2.5, 0, Math.PI * 2)
  ctx.fill()

  // 어깨 장착 블록 + 팔 — 어깨→팔꿈치→손목을 하나의 stroke path로 이어서
  // (다리와 같은 이유로) 이음매 없이 매끈하게 굽어 보이게 한다. 실제 서버
  // IK가 계산한 shoulder/elbow_angle(`arm_pose`)은 그대로 쓰고, 그 결과를
  // 그리는 원점만 몸통 중앙에서 오른쪽 슬리버 위 모서리(어깨 위치)로
  // 옮긴다.
  const shoulderX = BODY_WIDTH / 2 - 2
  const shoulderY = bodyTopY + 4
  ctx.fillStyle = '#3a4048'
  ctx.fillRect(shoulderX - SHOULDER_BLOCK_SIZE / 2, shoulderY - SHOULDER_BLOCK_SIZE / 2, SHOULDER_BLOCK_SIZE, SHOULDER_BLOCK_SIZE)

  const elbowDx = elbowScreen.x - screen.x
  const elbowDy = elbowScreen.y - screen.y
  const wristDx = wristScreen.x - screen.x
  const wristDy = wristScreen.y - screen.y

  ctx.strokeStyle = '#8b95a0'
  ctx.lineWidth = 4
  ctx.lineCap = 'round'
  ctx.lineJoin = 'round'
  ctx.beginPath()
  ctx.moveTo(shoulderX, shoulderY)
  ctx.lineTo(shoulderX + elbowDx, shoulderY + elbowDy)
  ctx.lineTo(shoulderX + wristDx, shoulderY + wristDy)
  ctx.stroke()

  ctx.restore()
}
```

- [ ] **Step 2: 타입체크 + 기존 단위테스트 확인**

Run: `cd client && npm run typecheck && npm test`
Expected: 타입 에러 없음. `canvas.test.ts`의 기존 3개 describe 블록(`isConveyorCell`/`conveyorFlowDirection`/`sortRobotsForDrawing`)과 새 `gait.test.ts` 전부 PASS — `drawRobot`은 순수 함수가 아니라 기존에도 직접 단위테스트가 없었으므로(캔버스 부수효과) 이 스텝에서 새로 깨지는 테스트가 없어야 정상이다.

- [ ] **Step 3: 로컬에서 육안 확인**

Run(터미널 1): `cargo run --manifest-path server/Cargo.toml` — 뜬 `LISTENING_PORT` 값 확인.
Run(터미널 2): `cd client && npm run dev`
브라우저에서 `http://localhost:5173/?ws=ws://127.0.0.1:<포트>/ws` 열고 로봇 수를 1~2로 늘려서, 다리 4개가 몸통에서 안 떨어져 보이는지, 걸을 때 무릎이 흔듦 구간에서 굽는지, 로봇을 선택해 팔 동작(Picking/Placing)을 트리거했을 때 센서 눈이 노란색으로 바뀌는지 확인한다.

- [ ] **Step 4: 커밋**

```bash
git add client/src/render/canvas.ts
git commit -m "feat: redraw robot as Spot-style quadruped with seamless attached legs"
```

---

### Task 3: E2E 몸체색 테스트를 새 팔레트에 맞게 수정

**Files:**
- Modify: `client/tests/e2e/render.spec.ts:100-213`

- [ ] **Step 1: `isBodyColor` 판정 함수와 상수를 새 몸체 색(`#6b7480`~`#5a636e`)에 맞게 교체**

`client/tests/e2e/render.spec.ts`에서 (107행 부근) `MIN_BODY_PIXELS` 선언 바로 아래 주석을 이렇게 갱신:

```typescript
// isBodyColor는 canvas.ts::drawRobot의 새 몸체 그라디언트(#6b7480~#5a636e,
// 슬레이트 그레이 — 2026-07-21 로봇 외형 리디자인)와 대조 확인한다. 팔
// 스트로크(#8b95a0)나 다리(#454c54)는 R/G/B가 이 범위 밖이라 안 걸린다.
const MIN_BODY_PIXELS = 80
```

`isBodyColor` 정의(131행 부근)를 교체:

```typescript
const isBodyColor = (r: number, g: number, b: number) =>
  r >= 80 && r <= 130 && g >= 90 && g <= 140 && b >= 95 && b <= 145 && b >= r
```

박스 크기(188~189행 부근, `const w = 22`/`const h = 16`)를 새 몸체 크기로 교체:

```typescript
const w = 26
const h = 20
```

마지막 단언(212행 부근)을 교체:

```typescript
// 몸체 채우기 색(그라디언트: #6b7480 ~ #5a636e, 슬레이트 그레이로 B가
// R보다 항상 크거나 같음)이 26x20 박스 평균에 지배적으로 반영되는지 확인.
expect(result.avgR).toBeGreaterThan(70)
expect(result.avgR).toBeLessThan(140)
```

- [ ] **Step 2: E2E 테스트 실행해서 통과 확인**

Run: `cd client && npm run build && npx playwright test render.spec.ts`
Expected: PASS (`draws a spawned robot with body-colored pixels somewhere on the canvas` 포함 전체 통과)

- [ ] **Step 3: 뮤테이션 테스트 — 색 판정이 실제로 몸체를 구분하는지 확인**

`isBodyColor`의 `b >= r` 조건을 잠시 지우고 같은 명령을 재실행 — 팔/다리 색 일부가 몸체로 오인돼 무게중심이 흔들리면서 테스트가 불안정해지거나 실패하는지 확인한다(안 흔들리면 이 조건이 실제로 필요한 변별력을 갖는지 재검토). 확인 후 조건을 되돌리고 다시 PASS 확인.

- [ ] **Step 4: 커밋**

```bash
git add client/tests/e2e/render.spec.ts
git commit -m "test: update E2E body-color assertion for the redesigned slate-gray robot palette"
```

---

### Task 4: 전체 검증 + 문서 갱신

**Files:**
- Modify: `README.md`
- Modify: `docs/KANBAN.md`

- [ ] **Step 1: 서버+클라이언트 전체 테스트 스위트 재확인**

Run:
```bash
cargo test --manifest-path server/Cargo.toml
cargo clippy --manifest-path server/Cargo.toml --all-targets
cd client && npm run typecheck && npm test && npm run build && npx playwright test
```
Expected: 전부 PASS, clippy 경고 0개(서버는 이 플랜에서 안 건드렸으므로 회귀만 확인).

- [ ] **Step 2: README.md의 "지금까지 만든 것" 절에 이번 작업 추가**

`README.md`에서 "## 지금까지 만든 것" 절의 Plan 5 항목 바로 아래(그 다음 줄)에 새 항목 추가:

```markdown
- **로봇 외형 리디자인**: 라이브 데모를 직접 확인한 사용자 피드백("로봇 같지 않다")을 반영해 클라이언트 렌더링을 평평한 사각형+일자 다리에서 Spot 스타일 4족 로봇으로 다시 그렸다. 바닥 타일은 기존 아이소메트릭 투영을 유지하되 로봇 캐릭터만 3/4 정면 각도로 그려 실루엣을 뚜렷하게 하고, 다리 4개는 엉덩이→무릎→발을 하나의 연속된 stroke path로 그려 이음매 없이 몸통에 부착시켰다(비주얼 컴패니언 목업 반복 중 뒷다리가 몸통에서 떨어져 보이는 실제 버그를 발견해 수정). 걸음은 디딤(60%, 무릎 편 상태)/흔듦(40%, 무릎 굽힘)을 비대칭 타이밍으로 나눈 순수 함수(`client/src/render/gait.ts`)로 매핑해 "미끄러지듯 이동한다"는 인상을 줄였다. 서버/프로토콜 변경 없음.
```

- [ ] **Step 3: docs/KANBAN.md 갱신**

`docs/KANBAN.md`의 "## Done" 절, Plan 5 항목 바로 다음에 새 섹션 추가:

```markdown
### 로봇 외형 리디자인 (`docs/superpowers/specs/2026-07-21-robot-visual-redesign-design.md`, `docs/superpowers/plans/2026-07-21-robot-visual-redesign-plan.md`)
라이브 데모 실사용 피드백("로봇 같지 않다") → 비주얼 컴패니언으로 다수 목업 반복(정면 각도 → 아이소메트릭 박스 → 3/4 정면 각도 → 애니메이션 → 관절 이음매/부착 버그 수정) → 설계 → 4개 태스크 전부 완료.
- **Task 1** — 위상→다리각도 매핑 순수 함수(`client/src/render/gait.ts`, 디딤/흔듦 비대칭 타이밍).
- **Task 2** — `drawRobot` 재작성(Spot 스타일 몸통+다리, 엉덩이→무릎→발 단일 stroke path로 이음매 없이 부착, 센서 눈 상태 색).
- **Task 3** — E2E 몸체색 테스트를 새 슬레이트 그레이 팔레트에 맞게 갱신.
- **Task 4** — 전체 검증 + 문서 갱신.
```

- [ ] **Step 4: 커밋**

```bash
git add README.md docs/KANBAN.md
git commit -m "docs: record robot visual redesign completion in README and KANBAN"
```
