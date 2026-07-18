import { describe, it, expect } from 'vitest'
import { isConveyorCell, sortRobotsForDrawing } from '../../src/render/canvas'
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

describe('sortRobotsForDrawing', () => {
  it('orders robots from smallest to largest z-order key so nearer robots draw last (on top)', () => {
    const far = robotAt(1, 5, 5)
    const near = robotAt(2, 0, 0)
    const mid = robotAt(3, 2, 2)

    const sorted = sortRobotsForDrawing([far, near, mid])

    expect(sorted.map((r) => r.id)).toEqual([2, 3, 1])
  })
})
