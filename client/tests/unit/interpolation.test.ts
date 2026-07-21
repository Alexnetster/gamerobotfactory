import { describe, it, expect } from 'vitest'
import { computeRenderFactor, computeRenderRobots, TICK_DURATION_MS } from '../../src/state/interpolation'
import { createEmptyMirror, applyServerMessage } from '../../src/state/mirror'
import type { RobotView } from '../../src/net/protocol'

function robot(id: number, x: number): RobotView {
  return {
    id,
    pos: { x, y: 0 },
    pose: 'Standing',
    leg_cycle_progress: 0,
    task: 'Idle',
    status: { kind: 'Operational' },
    durability_remaining: 1,
    path: [],
    facing: 'East',
    arm_pose: { shoulder_angle: 0, elbow_angle: 0 },
    carrying: false,
  }
}

function mirrorWith(...robots: RobotView[]) {
  return applyServerMessage(createEmptyMirror(), {
    kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots,
  })
}

describe('computeRenderFactor', () => {
  it('is 0 at the moment curr was received', () => {
    expect(computeRenderFactor(0)).toBe(0)
  })

  it('is 0.5 halfway through the tick window', () => {
    expect(computeRenderFactor(TICK_DURATION_MS / 2)).toBeCloseTo(0.5)
  })

  it('is 1 exactly at the tick boundary', () => {
    expect(computeRenderFactor(TICK_DURATION_MS)).toBeCloseTo(1)
  })

  it('extrapolates past 1 when the next tick is late', () => {
    expect(computeRenderFactor(TICK_DURATION_MS + TICK_DURATION_MS / 2)).toBeCloseTo(1.5)
  })

  it('caps extrapolation instead of growing without bound', () => {
    const atCap = computeRenderFactor(TICK_DURATION_MS + 100)
    const wayPastCap = computeRenderFactor(TICK_DURATION_MS + 100_000)
    expect(atCap).toBeCloseTo(wayPastCap, 5)
  })
})

describe('computeRenderRobots', () => {
  it('interpolates halfway between prev and curr positions', () => {
    const prev = { mirror: mirrorWith(robot(1, 0)), receivedAtMs: 1000 }
    const curr = { mirror: mirrorWith(robot(1, 2)), receivedAtMs: 1050 }

    const rendered = computeRenderRobots(prev, curr, 1075) // 25ms into the 50ms window

    expect(rendered[0].renderPos.x).toBeCloseTo(1)
  })

  it('shows a newly-appeared robot at its curr position with no interpolation partner', () => {
    const curr = { mirror: mirrorWith(robot(1, 3)), receivedAtMs: 1000 }

    const rendered = computeRenderRobots(null, curr, 1000)

    expect(rendered[0].renderPos).toEqual({ x: 3, y: 0 })
  })

  it('extrapolates beyond curr when the next tick is late', () => {
    const prev = { mirror: mirrorWith(robot(1, 0)), receivedAtMs: 1000 }
    const curr = { mirror: mirrorWith(robot(1, 2)), receivedAtMs: 1050 }

    // curr로부터 25ms 지남(=elapsed 75ms, factor 1.5) -> 2 + (2-0)*0.5 = 3
    const rendered = computeRenderRobots(prev, curr, 1125)

    expect(rendered[0].renderPos.x).toBeCloseTo(3)
  })
})
