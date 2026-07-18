import { describe, it, expect } from 'vitest'
import { parseServerMessage, encodeClientCommand } from '../../src/net/protocol'

describe('parseServerMessage', () => {
  it('parses a Snapshot message', () => {
    const raw = JSON.stringify({
      kind: 'Snapshot',
      v: 1,
      tick: 5,
      session_id: '00000000-0000-0000-0000-000000000000',
      conveyor: { running: true },
      robots: [],
    })

    const msg = parseServerMessage(raw)

    expect(msg).not.toBeNull()
    expect(msg?.kind).toBe('Snapshot')
  })

  it('parses a Delta message with a null conveyor (unchanged)', () => {
    const raw = JSON.stringify({
      kind: 'Delta',
      v: 1,
      tick: 6,
      conveyor: null,
      changed_robots: [],
      removed_robot_ids: [],
    })

    const msg = parseServerMessage(raw)

    expect(msg).not.toBeNull()
    if (msg?.kind === 'Delta') {
      expect(msg.conveyor).toBeNull()
    } else {
      throw new Error('expected Delta')
    }
  })

  it('returns null for invalid JSON instead of throwing', () => {
    expect(parseServerMessage('not valid json')).toBeNull()
  })

  it('returns null for JSON missing a kind field', () => {
    expect(parseServerMessage(JSON.stringify({ foo: 'bar' }))).toBeNull()
  })
})

describe('encodeClientCommand', () => {
  it('encodes SelectRobot matching the server tagged-union shape', () => {
    const json = encodeClientCommand({ type: 'SelectRobot', robot_id: 7 })
    expect(JSON.parse(json)).toEqual({ type: 'SelectRobot', robot_id: 7 })
  })

  it('encodes TriggerArmAction with a nested task string', () => {
    const json = encodeClientCommand({ type: 'TriggerArmAction', robot_id: 3, task: 'Picking' })
    expect(JSON.parse(json)).toEqual({ type: 'TriggerArmAction', robot_id: 3, task: 'Picking' })
  })
})
