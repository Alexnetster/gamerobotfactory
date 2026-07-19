import { describe, it, expect } from 'vitest'
import {
  gridToScreen,
  forwardKinematics,
  forwardDirectionVector,
  wristWorldOffset,
  elbowWorldOffset,
  zOrderKey,
  UPPER_ARM_LEN,
  LOWER_ARM_LEN,
} from '../../src/render/projection'

describe('gridToScreen', () => {
  it('maps the origin to the screen origin', () => {
    expect(gridToScreen(0, 0)).toEqual({ x: 0, y: 0 })
  })

  it('maps grid axes to the expected diamond offsets', () => {
    expect(gridToScreen(1, 0)).toEqual({ x: 32, y: 16 })
    expect(gridToScreen(0, 1)).toEqual({ x: -32, y: 16 })
    expect(gridToScreen(1, 1)).toEqual({ x: 0, y: 32 })
  })
})

describe('forwardKinematics', () => {
  it('points straight forward when both angles are zero', () => {
    const p = forwardKinematics(UPPER_ARM_LEN, LOWER_ARM_LEN, 0, 0)
    expect(p.x).toBeCloseTo(UPPER_ARM_LEN + LOWER_ARM_LEN)
    expect(p.y).toBeCloseTo(0)
  })

  it('matches manual trigonometry for a 90-degree shoulder angle', () => {
    const p = forwardKinematics(UPPER_ARM_LEN, LOWER_ARM_LEN, Math.PI / 2, 0)
    expect(p.x).toBeCloseTo(0, 5)
    expect(p.y).toBeCloseTo(UPPER_ARM_LEN + LOWER_ARM_LEN)
  })
})

describe('forwardDirectionVector', () => {
  it('maps all four facings to axis-aligned unit vectors', () => {
    expect(forwardDirectionVector('East')).toEqual({ dx: 1, dy: 0 })
    expect(forwardDirectionVector('West')).toEqual({ dx: -1, dy: 0 })
    expect(forwardDirectionVector('North')).toEqual({ dx: 0, dy: 1 })
    expect(forwardDirectionVector('South')).toEqual({ dx: 0, dy: -1 })
  })
})

describe('wristWorldOffset', () => {
  it('extends the wrist forward of the body in the facing direction', () => {
    const wrist = wristWorldOffset({ pos: { x: 2, y: 3 }, facing: 'East', shoulderAngle: 0, elbowAngle: 0 })
    expect(wrist.x).toBeCloseTo(2 + UPPER_ARM_LEN + LOWER_ARM_LEN)
    expect(wrist.y).toBeCloseTo(3)
  })
})

describe('elbowWorldOffset', () => {
  it('extends only the upper-arm segment forward, stopping short of the wrist', () => {
    const elbow = elbowWorldOffset({ pos: { x: 2, y: 3 }, facing: 'East', shoulderAngle: 0, elbowAngle: 0 })
    expect(elbow.x).toBeCloseTo(2 + UPPER_ARM_LEN)
    expect(elbow.y).toBeCloseTo(3)
  })

  it('matches manual trigonometry for a 90-degree shoulder angle', () => {
    const elbow = elbowWorldOffset({ pos: { x: 2, y: 3 }, facing: 'East', shoulderAngle: Math.PI / 2, elbowAngle: 0 })
    expect(elbow.x).toBeCloseTo(2, 5)
  })
})

describe('zOrderKey', () => {
  it('extends past the body cell when the arm reaches forward', () => {
    const key = zOrderKey({ pos: { x: 2, y: 3 }, facing: 'East', shoulderAngle: 0, elbowAngle: 0 })
    expect(key).toBeCloseTo(2 + UPPER_ARM_LEN + LOWER_ARM_LEN + 3)
  })

  it('falls back to the body cell key when the arm folds backward past the body', () => {
    // shoulderAngle = PI (뒤쪽을 향함) -> 로컬 x가 음수 -> 월드 좌표로는
    // facing 방향의 "뒤"로 접히므로, max()가 몸체 칸 자체를 골라야 한다
    // (음수 방향으로 바운딩박스가 넓어지면 안 된다).
    const key = zOrderKey({ pos: { x: 2, y: 3 }, facing: 'East', shoulderAngle: Math.PI, elbowAngle: 0 })
    expect(key).toBeCloseTo(2 + 3)
  })
})
