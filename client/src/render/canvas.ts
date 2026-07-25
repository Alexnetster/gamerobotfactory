import { gridToScreen, zOrderKey, wristWorldOffset, elbowWorldOffset, TILE_WIDTH, TILE_HEIGHT, RENDER_SCALE } from './projection'
import { legAnglesForPhase } from './gait'
import { BELT_ROW, BELT_START_X, BELT_END_X, WAREHOUSE_ROWS } from './layout'
import type { InterpolatedRobot, InterpolatedProduct } from '../state/interpolation'
import type { ConveyorView, StationView } from '../net/protocol'

export interface GridSize {
  width: number
  height: number
}

/** 일자형 벨트가 차지하는 칸(설계문서 §1) — 서버 `sim_core::sim`의
 * `BELT_ROW`/`BELT_START_X`/`BELT_END_X`와 정확히 같은 정의를 여기 다시
 * 써서 유지한다(서버는 자기 좌표만 보내지 이 상수들을 와이어로 보내지
 * 않는다 — 안 바뀌는 레이아웃 상수라 매 메시지에 실을 이유가 없다는
 * 판단, 기존 U자 벨트 때와 같은 트레이드오프). 지난번 U자 벨트에서 이
 * 정의가 서버/클라이언트 사이에 어긋나(그리드 크기 불일치) 실제 버그가
 * 났으므로, 이 값이 서버 쪽과 일치하는지 검증하는 테스트를 반드시 둔다
 * (Task 10). */
export function isConveyorCell(_grid: GridSize, x: number, y: number): boolean {
  return y === BELT_ROW && x >= BELT_START_X && x <= BELT_END_X
}

export function isWarehouseCell(_grid: GridSize, _x: number, y: number): boolean {
  return WAREHOUSE_ROWS.includes(y)
}

/** 벨트는 이제 항상 오른쪽(+x)으로만 흐른다 — U자 순환이 없어졌으니
 * 방향 계산도 훨씬 단순해졌다. */
export function conveyorFlowDirection(grid: GridSize, x: number, y: number): { dx: number; dy: number } | null {
  if (!isConveyorCell(grid, x, y)) {
    return null
  }
  return { dx: 1, dy: 0 }
}

/** z-order 오름차순 — 화면 안쪽(작은 x+y)부터 그려서, 앞쪽(큰 x+y) 로봇이
 * 나중에 그려져 위에 겹치게 한다. */
export function sortRobotsForDrawing(robots: InterpolatedRobot[]): InterpolatedRobot[] {
  return [...robots].sort((a, b) => {
    const keyA = zOrderKey({
      pos: a.renderPos, facing: a.facing, shoulderAngle: a.arm_pose.shoulder_angle, elbowAngle: a.arm_pose.elbow_angle,
    })
    const keyB = zOrderKey({
      pos: b.renderPos, facing: b.facing, shoulderAngle: b.arm_pose.shoulder_angle, elbowAngle: b.arm_pose.elbow_angle,
    })
    return keyA - keyB
  })
}

export interface DrawSceneInput {
  grid: GridSize
  conveyor: ConveyorView
  robots: InterpolatedRobot[]
  products: InterpolatedProduct[]
  stations: StationView[]
  showPaths: boolean
  animationTimeMs: number
  selectedRobotId: number | null
}

export function drawScene(ctx: CanvasRenderingContext2D, canvasWidth: number, canvasHeight: number, input: DrawSceneInput): void {
  ctx.clearRect(0, 0, canvasWidth, canvasHeight)
  ctx.save()
  ctx.translate(canvasWidth / 2, 40)
  ctx.scale(RENDER_SCALE, RENDER_SCALE)

  drawFloor(ctx, input.grid, input.conveyor, input.animationTimeMs)

  for (const product of input.products) {
    drawProduct(ctx, product)
  }

  for (const robot of sortRobotsForDrawing(input.robots)) {
    if (input.showPaths) {
      drawPath(ctx, robot)
    }
    drawRobot(ctx, robot, robot.id === input.selectedRobotId)
  }

  // 로봇 루프 뒤에 그린다 — 이 경고는 그 로봇 위에 얹히는 오버레이라
  // 로봇의 센서 헤드 블록(drawRobot, bodyTopY-2~+5)보다 항상 위(나중)에
  // 그려져야 한다. 예전엔 로봇 루프보다 먼저 그렸는데, 로봇 몸통이 서
  // 있을 때 헤드 블록이 이 삼각형의 밑변 ~4px를 덮어써 뾰족한 끝만 보이는
  // 시각 버그가 있었다(코드 리뷰에서 실측 재현: 재고 0은 흔한 정상
  // 상태라 매 프레임 발생). `Failed` 로봇 경고 삼각형(drawRobot 내부,
  // warnY = bodyTopY - 10)처럼 오프셋을 더 키워 여유를 주는 방법도
  // 있었지만, 그러면 BODY_HEIGHT/bodyLift가 바뀔 때마다 또 손으로
  // 맞춰야 한다 — 그리기 순서를 뒤로 미는 쪽이 기하학적 상수에
  // 의존하지 않아 더 견고하다.
  for (const station of input.stations) {
    drawStationInventoryWarning(ctx, station)
  }

  ctx.restore()
}

function drawFloor(ctx: CanvasRenderingContext2D, grid: GridSize, conveyor: ConveyorView, animationTimeMs: number): void {
  for (let y = 0; y < grid.height; y++) {
    for (let x = 0; x < grid.width; x++) {
      const screen = gridToScreen(x, y)
      const direction = conveyorFlowDirection(grid, x, y)
      const warehouse = isWarehouseCell(grid, x, y)
      drawTile(ctx, screen.x, screen.y, direction, warehouse, conveyor.running, animationTimeMs)
    }
  }
}

function drawTile(
  ctx: CanvasRenderingContext2D,
  sx: number,
  sy: number,
  direction: { dx: number; dy: number } | null,
  warehouse: boolean,
  running: boolean,
  animationTimeMs: number,
): void {
  const isBelt = direction !== null
  ctx.save()
  ctx.translate(sx, sy)
  ctx.beginPath()
  ctx.moveTo(0, -TILE_HEIGHT / 2)
  ctx.lineTo(TILE_WIDTH / 2, 0)
  ctx.lineTo(0, TILE_HEIGHT / 2)
  ctx.lineTo(-TILE_WIDTH / 2, 0)
  ctx.closePath()

  const gradient = ctx.createLinearGradient(-TILE_WIDTH / 2, 0, TILE_WIDTH / 2, 0)
  if (isBelt) {
    gradient.addColorStop(0, '#5b84c9')
    gradient.addColorStop(1, '#33538f')
  } else if (warehouse) {
    gradient.addColorStop(0, '#8f5fc9')
    gradient.addColorStop(1, '#5f3d8f')
  } else {
    gradient.addColorStop(0, '#4a9d6f')
    gradient.addColorStop(1, '#2c6b47')
  }
  ctx.fillStyle = gradient
  ctx.fill()
  ctx.strokeStyle = 'rgba(0,0,0,0.3)'
  ctx.stroke()

  if (direction) {
    drawConveyorChevrons(ctx, direction, running, animationTimeMs)
  }
  ctx.restore()
}

/** 벨트 칸 위에 흐르는 방향을 가리키는 화살표(셰브런)를 그린다 — 꺼져 있을
 * 때도 "이건 방향성 있는 기계"라는 걸 알아볼 수 있게 정적으로 하나 그리고,
 * 켜져 있으면 시간에 따라 이동하는 화살표 두 개로 흐름을 표현한다. 색만
 * 다른 평범한 floor 타일처럼 보인다는 피드백(사용자 실측)으로 추가됨. */
function drawConveyorChevrons(
  ctx: CanvasRenderingContext2D,
  direction: { dx: number; dy: number },
  running: boolean,
  animationTimeMs: number,
): void {
  const rawX = (direction.dx - direction.dy) * (TILE_WIDTH / 2)
  const rawY = (direction.dx + direction.dy) * (TILE_HEIGHT / 2)
  const len = Math.hypot(rawX, rawY)
  const fx = rawX / len
  const fy = rawY / len
  const px = -fy // 진행 방향에 수직인 벡터 — 셰브런의 "날개" 폭에 쓴다
  const py = fx

  ctx.strokeStyle = running ? 'rgba(255,255,255,0.85)' : 'rgba(255,255,255,0.4)'
  ctx.lineWidth = 2.5
  ctx.lineJoin = 'round'
  ctx.lineCap = 'round'

  const drawChevronAt = (centerOffset: number) => {
    const cx = fx * centerOffset
    const cy = fy * centerOffset
    const tip = { x: cx + fx * 6, y: cy + fy * 6 }
    const wingA = { x: cx - fx * 4 + px * 5, y: cy - fy * 4 + py * 5 }
    const wingB = { x: cx - fx * 4 - px * 5, y: cy - fy * 4 - py * 5 }
    ctx.beginPath()
    ctx.moveTo(wingA.x, wingA.y)
    ctx.lineTo(tip.x, tip.y)
    ctx.lineTo(wingB.x, wingB.y)
    ctx.stroke()
  }

  if (!running) {
    drawChevronAt(0)
    return
  }

  // 타일 폭(진행 방향 기준)을 따라 두 화살표가 시간에 맞춰 미끄러지며
  // 순환한다 — 기존 대각선 스트라이프와 같은 20ms/px 속도 감각을 유지.
  const cycle = TILE_WIDTH / 2
  const offset = (animationTimeMs / 20) % cycle
  drawChevronAt(offset - cycle / 2)
  drawChevronAt(offset)
}

/** 제품(드론) 단계별 스프라이트 — 브레인스토밍 목업(product-progression.html)의
 * SVG 모양을 캔버스 그리기로 그대로 옮긴 것. stage가 오를수록 프레임 위에
 * 부품이 하나씩 늘어난다. */
function drawProduct(ctx: CanvasRenderingContext2D, product: InterpolatedProduct): void {
  const screen = gridToScreen(product.renderPos.x, product.renderPos.y)
  ctx.save()
  ctx.translate(screen.x, screen.y - 6) // 바닥보다 살짝 띄워서 로봇 팔 높이와 겹치지 않게

  // 프레임(X자 팔) — 0단계부터 항상 보인다.
  ctx.strokeStyle = '#565e68'
  ctx.lineWidth = 3
  ctx.lineCap = 'round'
  ctx.beginPath()
  ctx.moveTo(-7, -4)
  ctx.lineTo(7, 4)
  ctx.moveTo(7, -4)
  ctx.lineTo(-7, 4)
  ctx.stroke()

  // 배터리(stage >= 1)
  if (product.stage >= 1) {
    ctx.fillStyle = '#c9762f'
    ctx.strokeStyle = '#1c2024'
    ctx.lineWidth = 1
    ctx.fillRect(-4, -3, 8, 6)
    ctx.strokeRect(-4, -3, 8, 6)
  }

  // 프로펠러 4개(stage >= 2)
  if (product.stage >= 2) {
    ctx.strokeStyle = '#8b95a0'
    ctx.lineWidth = 1.5
    for (const [cx, cy] of [
      [-7, -4],
      [7, -4],
      [-7, 4],
      [7, 4],
    ] as const) {
      ctx.beginPath()
      ctx.arc(cx, cy, 3, 0, Math.PI * 2)
      ctx.stroke()
    }
  }

  // 완성 체크(stage >= 3)
  if (product.stage >= 3) {
    ctx.strokeStyle = '#ffd23a'
    ctx.lineWidth = 1.5
    ctx.beginPath()
    ctx.arc(0, 0, 6, 0, Math.PI * 2)
    ctx.stroke()
  }

  ctx.restore()
}

/** 재고 0인 스테이션은 조립 로봇 위에 경고색 표시(기존 고장 표시와 같은
 * 관례 — 빨강 경고 삼각형). 재고가 있으면 아무것도 그리지 않는다. */
function drawStationInventoryWarning(ctx: CanvasRenderingContext2D, station: StationView): void {
  if (station.part_inventory > 0) {
    return
  }
  const screen = gridToScreen(station.robot_cell.x, station.robot_cell.y)
  ctx.save()
  ctx.translate(screen.x, screen.y - 34)
  ctx.fillStyle = '#e04b3f'
  ctx.beginPath()
  ctx.moveTo(0, -6)
  ctx.lineTo(5, 4)
  ctx.lineTo(-5, 4)
  ctx.closePath()
  ctx.fill()
  ctx.strokeStyle = '#1c2024'
  ctx.lineWidth = 1
  ctx.stroke()
  ctx.restore()
}

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

/** 센서 눈 색 — 로봇 상태(status)가 task보다 우선한다. 고장/수리 중인
 * 로봇도 Idle 로봇과 똑같이 멈춰 있어서, "멈춰있음"만으로는 실제로
 * 구분이 안 됐다(라이브 데모 실사용 피드백으로 발견). */
export function sensorEyeColor(robot: Pick<InterpolatedRobot, 'status' | 'task'>): string {
  if (robot.status.kind === 'Failed') return '#e04b3f'
  if (robot.status.kind === 'Repairing') return '#4bc0e0'
  return robot.task === 'Idle' ? '#8a8f96' : '#ffd23a'
}

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

  // 센서 헤드 — 눈 색으로 로봇 상태를 나타낸다. 고장(Failed)/수리 중
  // (Repairing)은 "멈춰있다"는 사실만으로 구분하려 했으나(설계문서 §5),
  // Idle(작업 없이 대기 중)도 똑같이 멈춰 있어서 실제로는 구분이 안
  // 됐다 — 사용자가 라이브 데모에서 실측으로 지적해 우선순위를
  // status > task로 바꿨다: 고장은 빨강, 수리 중은 하늘색, 그 외엔
  // 기존 작업중/대기 로직 그대로.
  ctx.fillStyle = '#3a4048'
  ctx.fillRect(-6, bodyTopY - 2, 12, 7)
  ctx.fillStyle = sensorEyeColor(robot)
  ctx.beginPath()
  ctx.arc(0, bodyTopY + 1.5, 2.5, 0, Math.PI * 2)
  ctx.fill()

  // 고장 상태는 눈 색만으로는 눈에 잘 안 띄어서(작은 점 색 하나 차이),
  // 몸통 위에 빨간 경고 삼각형(느낌표)을 따로 그려 확실히 구분되게 한다.
  if (robot.status.kind === 'Failed') {
    const warnY = bodyTopY - 10
    ctx.fillStyle = '#e04b3f'
    ctx.beginPath()
    ctx.moveTo(0, warnY - 6)
    ctx.lineTo(5, warnY + 4)
    ctx.lineTo(-5, warnY + 4)
    ctx.closePath()
    ctx.fill()
    ctx.strokeStyle = '#1c2024'
    ctx.lineWidth = 1
    ctx.stroke()
    ctx.fillStyle = '#ffffff'
    ctx.fillRect(-0.75, warnY - 3, 1.5, 4)
    ctx.fillRect(-0.75, warnY + 2, 1.5, 1.5)
  }

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

  if (robot.carrying) {
    const cargoX = shoulderX + wristDx
    const cargoY = shoulderY + wristDy
    ctx.fillStyle = '#c9762f'
    ctx.strokeStyle = '#1c2024'
    ctx.lineWidth = 1.5
    ctx.fillRect(cargoX - 5, cargoY - 5, 10, 9)
    ctx.strokeRect(cargoX - 5, cargoY - 5, 10, 9)
  }

  ctx.restore()
}

function drawPath(ctx: CanvasRenderingContext2D, robot: InterpolatedRobot): void {
  if (robot.path.length === 0) {
    return
  }
  ctx.save()
  ctx.strokeStyle = 'rgba(93, 214, 255, 0.7)'
  ctx.lineWidth = 2
  ctx.setLineDash([4, 4])
  ctx.beginPath()
  const start = gridToScreen(robot.renderPos.x, robot.renderPos.y)
  ctx.moveTo(start.x, start.y)
  for (const cell of robot.path) {
    const p = gridToScreen(cell.x, cell.y)
    ctx.lineTo(p.x, p.y)
  }
  ctx.stroke()
  ctx.restore()
}
