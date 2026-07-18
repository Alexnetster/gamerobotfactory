export const TILE_WIDTH = 64
export const TILE_HEIGHT = 32

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
