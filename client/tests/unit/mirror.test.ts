import { describe, it, expect } from 'vitest'
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
    })

    expect(next.conveyor).toEqual({ running: true })
    expect(next.robots.size).toBe(2)
    expect(next.robots.get(1)?.pos).toEqual({ x: 0, y: 0 })
  })

  it('overwrites changed robots on Delta', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [robot(1, 0)],
    })

    mirror = applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: null, changed_robots: [robot(1, 3)], removed_robot_ids: [],
    })

    expect(mirror.robots.get(1)?.pos).toEqual({ x: 3, y: 0 })
  })

  it('removes robots listed in removed_robot_ids', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [robot(1, 0), robot(2, 1)],
    })

    mirror = applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: null, changed_robots: [], removed_robot_ids: [2],
    })

    expect(mirror.robots.has(2)).toBe(false)
    expect(mirror.robots.has(1)).toBe(true)
  })

  it('keeps the previous conveyor state when Delta.conveyor is null', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [],
    })

    mirror = applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: null, changed_robots: [], removed_robot_ids: [],
    })

    expect(mirror.conveyor).toEqual({ running: true })
  })

  it('adopts the new conveyor state when Delta.conveyor is present', () => {
    let mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [],
    })

    mirror = applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: { running: false }, changed_robots: [], removed_robot_ids: [],
    })

    expect(mirror.conveyor).toEqual({ running: false })
  })

  it('leaves the mirror untouched on ResumeAck', () => {
    const mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [robot(1, 0)],
    })

    const next = applyServerMessage(mirror, { kind: 'ResumeAck', v: 1, session_id: 'abc', resumed: true })

    expect(next).toBe(mirror)
  })

  it('does not mutate the previous mirror object (pure function)', () => {
    const mirror = applyServerMessage(createEmptyMirror(), {
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'abc', conveyor: { running: true }, robots: [robot(1, 0)],
    })
    const robotsBefore = mirror.robots

    applyServerMessage(mirror, {
      kind: 'Delta', v: 1, tick: 2, conveyor: null, changed_robots: [robot(1, 9)], removed_robot_ids: [],
    })

    expect(mirror.robots).toBe(robotsBefore)
    expect(mirror.robots.get(1)?.pos).toEqual({ x: 0, y: 0 })
  })
})
