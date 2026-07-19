export const TILE_WIDTH = 64
export const TILE_HEIGHT = 32

// 전체 씬(타일+로봇)을 한 번에 키우는 배율. 로봇 몸체/다리는 절대 픽셀
// 상수(canvas.ts::drawRobot)라 TILE_WIDTH/HEIGHT만 키우면 타일 사이 간격만
// 넓어지고 로봇 자체는 그대로 작게 남는다 — drawScene에서 ctx.scale로 씬
// 전체에 곱해야 타일과 로봇이 함께 커진다. 클릭 히트테스트(main.ts)와 E2E
// 테스트(render.spec.ts)의 클릭 좌표 계산도 이 값을 역보정으로 써야
// 화면에 보이는 위치와 클릭 판정이 어긋나지 않는다.
export const RENDER_SCALE = 1.8

// server/src/protocol.rs의 UPPER_ARM_LEN/LOWER_ARM_LEN과 반드시 같은
// 값으로 유지해야 한다 — 와이어로 안 보내는 튜닝 상수.
export const UPPER_ARM_LEN = 0.7
export const LOWER_ARM_LEN = 0.6

export interface ScreenPoint {
  x: number
  y: number
}

export function gridToScreen(x: number, y: number): ScreenPoint {
  return {
    x: (x - y) * (TILE_WIDTH / 2),
    y: (x + y) * (TILE_HEIGHT / 2),
  }
}

export interface LocalPoint {
  x: number // 전방
  y: number // 높이
}

/** 서버 ik.rs::forward_kinematics와 동일한 공식 — 각도를 몸체-로컬
 * (전방, 높이) 좌표로 되돌린다. */
export function forwardKinematics(upperLen: number, lowerLen: number, shoulderAngle: number, elbowAngle: number): LocalPoint {
  const elbowX = upperLen * Math.cos(shoulderAngle)
  const elbowY = upperLen * Math.sin(shoulderAngle)
  const wristAngle = shoulderAngle + elbowAngle
  return {
    x: elbowX + lowerLen * Math.cos(wristAngle),
    y: elbowY + lowerLen * Math.sin(wristAngle),
  }
}

export type Facing = 'North' | 'East' | 'South' | 'West'

/** facing은 그리드의 4방향과 1:1 대응하므로(대각선 없음), 회전은 항상
 * 축정렬된 단위 벡터를 곱하는 것으로 충분하다. */
export function forwardDirectionVector(facing: Facing): { dx: number; dy: number } {
  switch (facing) {
    case 'East':
      return { dx: 1, dy: 0 }
    case 'West':
      return { dx: -1, dy: 0 }
    case 'North':
      return { dx: 0, dy: 1 }
    case 'South':
      return { dx: 0, dy: -1 }
  }
}

export interface RobotPoseInput {
  pos: { x: number; y: number }
  facing: Facing
  shoulderAngle: number
  elbowAngle: number
}

/** 로봇 손목의 월드 그리드 좌표 — z-order와 팔 드로잉이 공유해서 쓴다. */
export function wristWorldOffset(input: RobotPoseInput): { x: number; y: number; height: number } {
  const local = forwardKinematics(UPPER_ARM_LEN, LOWER_ARM_LEN, input.shoulderAngle, input.elbowAngle)
  const dir = forwardDirectionVector(input.facing)
  return {
    x: input.pos.x + dir.dx * local.x,
    y: input.pos.y + dir.dy * local.x,
    height: local.y,
  }
}

/** z-order 정렬 키 — 몸체 칸이 아니라 (몸체+팔이 차지하는) 바운딩 박스의
 * 가장 먼 안쪽 모서리 기준 x+y. 팔이 몸체 뒤쪽으로 접히는 경우(로컬 x가
 * 음수)엔 몸체 칸 자체가 최댓값이 된다. */
export function zOrderKey(input: RobotPoseInput): number {
  const wrist = wristWorldOffset(input)
  return Math.max(input.pos.x, wrist.x) + Math.max(input.pos.y, wrist.y)
}
