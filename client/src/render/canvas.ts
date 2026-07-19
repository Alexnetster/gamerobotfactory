import { gridToScreen, zOrderKey, wristWorldOffset, elbowWorldOffset, TILE_WIDTH, TILE_HEIGHT, RENDER_SCALE } from './projection'
import type { InterpolatedRobot } from '../state/interpolation'
import type { ConveyorView } from '../net/protocol'

export interface GridSize {
  width: number
  height: number
}

/** U자형 컨베이어 장식이 차지하는 칸 — 위/왼쪽/아래 세 변, 오른쪽(사이드바
 * 쪽) 개방. 서버는 이 개념을 전혀 모른다 — 순수 클라이언트 배경 장식이고
 * 로봇 이동/작업에 아무 영향도 주지 않는다. */
export function isConveyorCell(grid: GridSize, x: number, y: number): boolean {
  return y === 0 || y === grid.height - 1 || x === 0
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

  for (const robot of sortRobotsForDrawing(input.robots)) {
    if (input.showPaths) {
      drawPath(ctx, robot)
    }
    drawRobot(ctx, robot, robot.id === input.selectedRobotId)
  }

  ctx.restore()
}

function drawFloor(ctx: CanvasRenderingContext2D, grid: GridSize, conveyor: ConveyorView, animationTimeMs: number): void {
  for (let y = 0; y < grid.height; y++) {
    for (let x = 0; x < grid.width; x++) {
      const screen = gridToScreen(x, y)
      drawTile(ctx, screen.x, screen.y, isConveyorCell(grid, x, y), conveyor.running, animationTimeMs)
    }
  }
}

function drawTile(ctx: CanvasRenderingContext2D, sx: number, sy: number, isBelt: boolean, running: boolean, animationTimeMs: number): void {
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
  } else {
    gradient.addColorStop(0, '#4a9d6f')
    gradient.addColorStop(1, '#2c6b47')
  }
  ctx.fillStyle = gradient
  ctx.fill()
  ctx.strokeStyle = 'rgba(0,0,0,0.3)'
  ctx.stroke()

  if (isBelt && running) {
    // 흐르는 느낌의 대각선 스트라이프 — 시간에 따라 위치가 이동한다.
    // 정지 상태(running=false)면 스트라이프를 그리지 않아 정적으로 보인다.
    ctx.save()
    ctx.clip()
    const stripeOffset = (animationTimeMs / 20) % 12
    ctx.strokeStyle = 'rgba(255,255,255,0.35)'
    ctx.lineWidth = 3
    for (let i = -TILE_WIDTH; i < TILE_WIDTH; i += 12) {
      ctx.beginPath()
      ctx.moveTo(i + stripeOffset, -TILE_HEIGHT / 2)
      ctx.lineTo(i + stripeOffset + TILE_HEIGHT / 2, TILE_HEIGHT / 2)
      ctx.stroke()
    }
    ctx.restore()
  }
  ctx.restore()
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

  // 다리 — 몸체 바깥으로 뚜렷하게 뻗어 나오는 4족 자세가 보이도록 몸체
  // 폭(22px)보다 더 벌리고(±14), 발끝에 작은 원을 찍어 다리 끝이 어디서
  // 끝나는지 명확히 한다. 이렇게 안 하면 짧은 세로선만 보여서 몸체 밑에
  // 거의 안 보이고, 로봇 전체가 가오리처럼 미끄러지는 것처럼 보인다.
  ctx.strokeStyle = '#6b4810'
  ctx.fillStyle = '#6b4810'
  ctx.lineWidth = 3
  for (let i = 0; i < 4; i++) {
    const phase = (robot.leg_cycle_progress + i * 0.25) % 1
    const legX = (i < 2 ? -14 : 14) + (phase < 0.5 ? -4 : 4)
    const footY = -bodyLift + 14
    ctx.beginPath()
    ctx.moveTo(legX, -bodyLift)
    ctx.lineTo(legX, footY)
    ctx.stroke()
    ctx.beginPath()
    ctx.arc(legX, footY, 2, 0, Math.PI * 2)
    ctx.fill()
  }

  const bodyGradient = ctx.createLinearGradient(-11, -bodyLift - 8, 11, -bodyLift + 8)
  bodyGradient.addColorStop(0, '#ffd27a')
  bodyGradient.addColorStop(1, '#d99a2e')
  ctx.fillStyle = bodyGradient
  ctx.fillRect(-11, -bodyLift - 8, 22, 16)
  if (selected) {
    ctx.strokeStyle = '#ffffff'
    ctx.lineWidth = 2
    ctx.strokeRect(-11, -bodyLift - 8, 22, 16)
  }

  // 팔 — 어깨-팔꿈치-손목 두 세그먼트로 그려야 elbow_angle에 따른 실제
  // 굽힘이 보인다. 어깨에서 손목까지 직선 하나로만 이으면(예전 방식)
  // 팔이 항상 뻣뻣한 막대처럼 보여서 팔꿈치 각도가 있어도 티가 안 났다.
  ctx.strokeStyle = '#a06f1a'
  ctx.lineWidth = 3
  ctx.beginPath()
  ctx.moveTo(0, -bodyLift)
  ctx.lineTo(elbowScreen.x - screen.x, elbowScreen.y - screen.y - bodyLift)
  ctx.lineTo(wristScreen.x - screen.x, wristScreen.y - screen.y - bodyLift)
  ctx.stroke()

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
