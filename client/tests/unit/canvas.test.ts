import { describe, it, expect } from 'vitest'
import { isConveyorCell, sortRobotsForDrawing, conveyorFlowDirection, sensorEyeColor } from '../../src/render/canvas'
import type { InterpolatedRobot } from '../../src/state/interpolation'

function robotAt(id: number, x: number, y: number): InterpolatedRobot {
  return {
    id,
    pos: { x, y },
    renderPos: { x, y },
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

describe('isConveyorCell', () => {
  const grid = { width: 7, height: 6 }

  it('marks the top row, left column, and bottom row as belt', () => {
    expect(isConveyorCell(grid, 3, 0)).toBe(true)
    expect(isConveyorCell(grid, 0, 3)).toBe(true)
    expect(isConveyorCell(grid, 3, 5)).toBe(true)
  })

  it('leaves the right column and interior open (not belt) — U opens toward the sidebar', () => {
    expect(isConveyorCell(grid, 6, 3)).toBe(false)
    expect(isConveyorCell(grid, 3, 3)).toBe(false)
  })
})

describe('conveyorFlowDirection', () => {
  const grid = { width: 7, height: 6 }

  it('returns null for non-belt cells', () => {
    expect(conveyorFlowDirection(grid, 6, 3)).toBeNull()
    expect(conveyorFlowDirection(grid, 3, 3)).toBeNull()
  })

  it('flows left along the top row (toward the left column)', () => {
    expect(conveyorFlowDirection(grid, 3, 0)).toEqual({ dx: -1, dy: 0 })
  })

  it('flows down along the left column', () => {
    expect(conveyorFlowDirection(grid, 0, 3)).toEqual({ dx: 0, dy: 1 })
  })

  it('flows right along the bottom row (toward the open sidebar-facing end)', () => {
    expect(conveyorFlowDirection(grid, 3, 5)).toEqual({ dx: 1, dy: 0 })
  })

  it('resolves both corners consistently with a single continuous loop', () => {
    // (0,0): 위쪽 변과 왼쪽 변이 만나는 모서리 — 왼쪽 변 방향(아래)을 따른다.
    expect(conveyorFlowDirection(grid, 0, 0)).toEqual({ dx: 0, dy: 1 })
    // (0, height-1): 왼쪽 변과 아래쪽 변이 만나는 모서리 — 아래쪽 변 방향(오른쪽)을 따른다.
    expect(conveyorFlowDirection(grid, 0, 5)).toEqual({ dx: 1, dy: 0 })
  })
})

describe('sensorEyeColor', () => {
  it('고장(Failed) 로봇은 task와 무관하게 항상 빨강', () => {
    expect(sensorEyeColor({ status: { kind: 'Failed' }, task: 'Idle' })).toBe('#e04b3f')
    expect(sensorEyeColor({ status: { kind: 'Failed' }, task: 'Picking' })).toBe('#e04b3f')
  })

  it('수리 중(Repairing) 로봇은 task와 무관하게 항상 하늘색', () => {
    expect(sensorEyeColor({ status: { kind: 'Repairing', remaining_ticks: 50 }, task: 'Idle' })).toBe('#4bc0e0')
  })

  it('정상(Operational) + Idle은 회색', () => {
    expect(sensorEyeColor({ status: { kind: 'Operational' }, task: 'Idle' })).toBe('#8a8f96')
  })

  it('정상(Operational) + 작업 중(Picking/Placing)은 노랑', () => {
    expect(sensorEyeColor({ status: { kind: 'Operational' }, task: 'Picking' })).toBe('#ffd23a')
    expect(sensorEyeColor({ status: { kind: 'Operational' }, task: 'Placing' })).toBe('#ffd23a')
  })
})

describe('sortRobotsForDrawing', () => {
  it('orders robots from smallest to largest z-order key so nearer robots draw last (on top)', () => {
    const far = robotAt(1, 5, 5)
    const near = robotAt(2, 0, 0)
    const mid = robotAt(3, 2, 2)

    const sorted = sortRobotsForDrawing([far, near, mid])

    expect(sorted.map((r) => r.id)).toEqual([2, 3, 1])
  })
})
