import type { MirrorState } from './mirror'
import type { RobotView } from '../net/protocol'

export const TICK_DURATION_MS = 50
// 다음 틱이 지연되면 이 시간(ms)만큼만 짧게 외삽한 뒤 그 지점에서
// 정지한다 — 탭이 백그라운드에서 오래 스로틀돼도 무한정 앞서나가지
// 않는다. 튜닝 대상.
export const MAX_EXTRAPOLATION_MS = 100

export interface InterpolatedRobot extends RobotView {
  renderPos: { x: number; y: number }
}

export interface TickSnapshot {
  mirror: MirrorState
  receivedAtMs: number
}

/** prev->curr 진행 계수. [0,1]은 보간, 1보다 크면(최대
 * `1 + MAX_EXTRAPOLATION_MS/TICK_DURATION_MS`까지) curr 시점 속도로
 * 외삽한 값이고, 그 이상 지나도 더 커지지 않는다. */
export function computeRenderFactor(elapsedMs: number): number {
  const clampedElapsed = Math.max(elapsedMs, 0)
  if (clampedElapsed <= TICK_DURATION_MS) {
    return clampedElapsed / TICK_DURATION_MS
  }
  const overage = Math.min(clampedElapsed - TICK_DURATION_MS, MAX_EXTRAPOLATION_MS)
  return 1 + overage / TICK_DURATION_MS
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t
}

/** curr에 있는 로봇만 렌더링 대상이다(제거된 로봇은 mirror.robots에 이미
 * 없다). prev에 짝이 없는(새로 등장한) 로봇은 보간 없이 curr 위치 그대로
 * 표시한다. */
export function computeRenderRobots(prev: TickSnapshot | null, curr: TickSnapshot, nowMs: number): InterpolatedRobot[] {
  const factor = computeRenderFactor(nowMs - curr.receivedAtMs)
  const result: InterpolatedRobot[] = []

  for (const robot of curr.mirror.robots.values()) {
    const prevRobot = prev?.mirror.robots.get(robot.id)
    if (!prevRobot) {
      result.push({ ...robot, renderPos: { x: robot.pos.x, y: robot.pos.y } })
      continue
    }
    result.push({
      ...robot,
      renderPos: {
        x: lerp(prevRobot.pos.x, robot.pos.x, factor),
        y: lerp(prevRobot.pos.y, robot.pos.y, factor),
      },
    })
  }
  return result
}
