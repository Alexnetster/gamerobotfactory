import { describe, it, expect } from 'vitest'
import { computeRenderFactor, computeRenderRobots, computeRenderProducts, TICK_DURATION_MS } from '../../src/state/interpolation'
import { createEmptyMirror, applyServerMessage } from '../../src/state/mirror'
import type { ProductView, RobotView } from '../../src/net/protocol'

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
    role: { kind: 'Helper' },
  }
}

function product(id: number, x: number): ProductView {
  return { id, stage: 0, pos: { x, y: 0 } }
}

function mirrorWith(...robots: RobotView[]) {
  return applyServerMessage(createEmptyMirror(), {
    kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots, stations: [], products: [],
  })
}

function mirrorWithProducts(...products: ProductView[]) {
  return applyServerMessage(createEmptyMirror(), {
    kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [], stations: [], products,
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

describe('computeRenderProducts', () => {
  it('interpolates halfway between prev and curr positions', () => {
    const prev = { mirror: mirrorWithProducts(product(1, 0)), receivedAtMs: 1000 }
    const curr = { mirror: mirrorWithProducts(product(1, 2)), receivedAtMs: 1050 }

    const rendered = computeRenderProducts(prev, curr, 1075) // 25ms into the 50ms window

    expect(rendered[0].renderPos.x).toBeCloseTo(1)
  })

  it('shows a newly-appeared product at its curr position with no interpolation partner', () => {
    const curr = { mirror: mirrorWithProducts(product(1, 3)), receivedAtMs: 1000 }

    const rendered = computeRenderProducts(null, curr, 1000)

    expect(rendered[0].renderPos).toEqual({ x: 3, y: 0 })
  })

  it('renders at exactly the prev position when factor is 0', () => {
    const prev = { mirror: mirrorWithProducts(product(1, 0)), receivedAtMs: 1000 }
    const curr = { mirror: mirrorWithProducts(product(1, 2)), receivedAtMs: 1050 }

    const rendered = computeRenderProducts(prev, curr, 1050) // curr just received, elapsed = 0

    expect(rendered[0].renderPos.x).toBeCloseTo(0)
  })

  it('renders at exactly the curr position when factor is 1', () => {
    const prev = { mirror: mirrorWithProducts(product(1, 0)), receivedAtMs: 1000 }
    const curr = { mirror: mirrorWithProducts(product(1, 2)), receivedAtMs: 1050 }

    const rendered = computeRenderProducts(prev, curr, 1050 + TICK_DURATION_MS) // elapsed = TICK_DURATION_MS -> factor 1

    expect(rendered[0].renderPos.x).toBeCloseTo(2)
  })

  it('extrapolates beyond curr when the next tick is late', () => {
    const prev = { mirror: mirrorWithProducts(product(1, 0)), receivedAtMs: 1000 }
    const curr = { mirror: mirrorWithProducts(product(1, 2)), receivedAtMs: 1050 }

    // curr로부터 25ms 지남(=elapsed 75ms, factor 1.5) -> 2 + (2-0)*0.5 = 3
    const rendered = computeRenderProducts(prev, curr, 1125)

    expect(rendered[0].renderPos.x).toBeCloseTo(3)
  })
})
