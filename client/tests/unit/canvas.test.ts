import { describe, it, test, expect } from 'vitest'
import { isConveyorCell, isWarehouseCell, sortRobotsForDrawing, conveyorFlowDirection, sensorEyeColor } from '../../src/render/canvas'
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
    role: { kind: 'Helper' },
  }
}

test('isConveyorCell is true only along the straight belt row within its span', () => {
  const grid = { width: 9, height: 7 }
  expect(isConveyorCell(grid, 1, 3)).toBe(true)
  expect(isConveyorCell(grid, 7, 3)).toBe(true)
  expect(isConveyorCell(grid, 0, 3)).toBe(false)
  expect(isConveyorCell(grid, 8, 3)).toBe(false)
  expect(isConveyorCell(grid, 4, 2)).toBe(false)
})

test('conveyorFlowDirection always points right on the belt', () => {
  const grid = { width: 9, height: 7 }
  expect(conveyorFlowDirection(grid, 4, 3)).toEqual({ dx: 1, dy: 0 })
  expect(conveyorFlowDirection(grid, 4, 2)).toBeNull()
})

test('isWarehouseCell is true for the top two rows', () => {
  const grid = { width: 9, height: 7 }
  expect(isWarehouseCell(grid, 3, 0)).toBe(true)
  expect(isWarehouseCell(grid, 3, 1)).toBe(true)
  expect(isWarehouseCell(grid, 3, 2)).toBe(false)
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
