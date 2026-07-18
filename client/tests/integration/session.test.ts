import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import WebSocket from 'ws'
import { spawnServer, stopServer } from '../helpers/spawn-server'
import type { SpawnedServer } from '../helpers/spawn-server'
import { parseServerMessage, encodeClientCommand } from '../../src/net/protocol'
import type { ServerMessage } from '../../src/net/protocol'
import { applyServerMessage, createEmptyMirror } from '../../src/state/mirror'
import type { MirrorState } from '../../src/state/mirror'

let server: SpawnedServer

beforeAll(async () => {
  server = await spawnServer()
})

afterAll(() => {
  stopServer(server)
})

function connect(port: number): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`)
    ws.once('open', () => resolve(ws))
    ws.once('error', reject)
  })
}

function nextMessage(ws: WebSocket): Promise<ServerMessage> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out waiting for a message')), 5000)
    ws.once('message', (data) => {
      clearTimeout(timeout)
      const parsed = parseServerMessage(data.toString())
      if (!parsed) {
        reject(new Error(`failed to parse message: ${data.toString()}`))
        return
      }
      resolve(parsed)
    })
  })
}

describe('client state layer against a real running server', () => {
  it('mirrors an initial empty Snapshot from the real server', async () => {
    const ws = await connect(server.port)
    const first = await nextMessage(ws)
    ws.close()

    expect(first.kind).toBe('Snapshot')
    const mirror: MirrorState = applyServerMessage(createEmptyMirror(), first)
    expect(mirror.robots.size).toBe(0)
  })

  it('reflects SetRobotCount into the local mirror, including facing/path/arm_pose', async () => {
    const ws = await connect(server.port)
    await nextMessage(ws) // 초기 스냅샷 소비

    ws.send(encodeClientCommand({ type: 'SetRobotCount', count: 2 }))

    let mirror: MirrorState = createEmptyMirror()
    const deadline = Date.now() + 5000
    while (mirror.robots.size < 2 && Date.now() < deadline) {
      const msg = await nextMessage(ws)
      mirror = applyServerMessage(mirror, msg)
    }
    ws.close()

    expect(mirror.robots.size).toBe(2)
    for (const robot of mirror.robots.values()) {
      expect(['North', 'East', 'South', 'West']).toContain(robot.facing)
      expect(Array.isArray(robot.path)).toBe(true)
      expect(typeof robot.arm_pose.shoulder_angle).toBe('number')
    }
  })

  it('resyncs after Resume with a valid session id', async () => {
    const ws1 = await connect(server.port)
    const snapshot = await nextMessage(ws1)
    if (snapshot.kind !== 'Snapshot') throw new Error('expected Snapshot')
    const sessionId = snapshot.session_id
    ws1.close()

    const ws2 = await connect(server.port)
    await nextMessage(ws2) // 재접속도 항상 새 Snapshot을 먼저 보낸다
    ws2.send(encodeClientCommand({ type: 'Resume', session_id: sessionId }))
    const ack = await nextMessage(ws2)
    ws2.close()

    expect(ack.kind).toBe('ResumeAck')
    if (ack.kind === 'ResumeAck') {
      expect(ack.resumed).toBe(true)
    }
  })
})
