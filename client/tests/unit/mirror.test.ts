import { describe, it, expect, test } from 'vitest'
import { applyServerMessage, createEmptyMirror } from '../../src/state/mirror'
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
    role: { kind: 'Helper' },
  }
}

describe('applyServerMessage', () => {
  it('replaces the whole robot map on Snapshot', () => {
    const mirror = createEmptyMirror()
    const next = applyServerMessage(mirror, {
      kind: 'Snapshot',
      v: 1,
      tick: 1,
      session_id: 'abc',
      conveyor: { running: true },
      robots: [robot(1, 0), robot(2, 5)],
      stations: [],
      products: [],
    })

    expect(next.conveyor).toEqual({ running: true })
    expect(next.robots.size).toBe(2)
    expect(next.robots.get(1)?.pos).toEqual({ x: 0, y: 0 })
  })

  it('overwrites changed robots on Delta', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [robot(1, 0)], stations: [], products: [],
    })

    mirror = applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: null, changed_robots: [robot(1, 3)], removed_robot_ids: [],
      stations: [], changed_products: [], removed_product_ids: [],
    })

    expect(mirror.robots.get(1)?.pos).toEqual({ x: 3, y: 0 })
  })

  it('adds a brand-new robot introduced via Delta', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [robot(1, 0)], stations: [], products: [],
    })
    mirror = applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: null, changed_robots: [robot(2, 5)], removed_robot_ids: [],
      stations: [], changed_products: [], removed_product_ids: [],
    })
    expect(mirror.robots.size).toBe(2)
    expect(mirror.robots.get(2)?.pos).toEqual({ x: 5, y: 0 })
  })

  it('removes robots listed in removed_robot_ids', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [robot(1, 0), robot(2, 1)], stations: [], products: [],
    })

    mirror = applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: null, changed_robots: [], removed_robot_ids: [2],
      stations: [], changed_products: [], removed_product_ids: [],
    })

    expect(mirror.robots.has(2)).toBe(false)
    expect(mirror.robots.has(1)).toBe(true)
  })

  it('keeps the previous conveyor state when Delta.conveyor is null', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [], stations: [], products: [],
    })

    mirror = applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: null, changed_robots: [], removed_robot_ids: [],
      stations: [], changed_products: [], removed_product_ids: [],
    })

    expect(mirror.conveyor).toEqual({ running: true })
  })

  it('adopts the new conveyor state when Delta.conveyor is present', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [], stations: [], products: [],
    })

    mirror = applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: { running: false }, changed_robots: [], removed_robot_ids: [],
      stations: [], changed_products: [], removed_product_ids: [],
    })

    expect(mirror.conveyor).toEqual({ running: false })
  })

  it('leaves the mirror untouched on ResumeAck', () => {
    const mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [robot(1, 0)], stations: [], products: [],
    })

    const next = applyServerMessage(mirror, { kind: 'ResumeAck', v: 1, session_id: 'abc', resumed: true })

    expect(next).toBe(mirror)
  })

  it('does not mutate the previous mirror object (pure function)', () => {
    const mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [robot(1, 0)], stations: [], products: [],
    })
    const robotsBefore = mirror.robots

    applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: null, changed_robots: [robot(1, 9)], removed_robot_ids: [],
      stations: [], changed_products: [], removed_product_ids: [],
    })

    expect(mirror.robots).toBe(robotsBefore)
    expect(mirror.robots.get(1)?.pos).toEqual({ x: 0, y: 0 })
  })

  test('snapshot populates stations and products', () => {
    const mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot',
      v: 1,
      tick: 0,
      session_id: 'x',
      conveyor: { running: true },
      robots: [],
      stations: [{ index: 0, robot_cell: { x: 2, y: 2 }, part_inventory: 5 }],
      products: [{ id: 1, stage: 0, pos: { x: 1, y: 3 } }],
    })

    expect(mirror.stations).toHaveLength(1)
    expect(mirror.products.get(1)?.stage).toBe(0)
  })

  test('delta removes a product by id', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot',
      v: 1,
      tick: 0,
      session_id: 'x',
      conveyor: { running: true },
      robots: [],
      stations: [],
      products: [{ id: 1, stage: 0, pos: { x: 1, y: 3 } }],
    })

    mirror = applyServerMessage(mirror, {
      kind: 'Delta',
      v: 1,
      tick: 1,
      conveyor: null,
      changed_robots: [],
      removed_robot_ids: [],
      stations: [],
      changed_products: [],
      removed_product_ids: [1],
    })

    expect(mirror.products.has(1)).toBe(false)
  })
})
