import { describe, it, expect } from 'vitest'
import { Connection, type WebSocketLike, type SessionStorageLike } from '../../src/net/connection'

class FakeWebSocket implements WebSocketLike {
  sent: string[] = []
  closed = false
  onopen: WebSocketLike['onopen'] = null
  onmessage: WebSocketLike['onmessage'] = null
  onclose: WebSocketLike['onclose'] = null
  onerror: WebSocketLike['onerror'] = null

  send(data: string) {
    this.sent.push(data)
  }
  close() {
    this.closed = true
  }
}

function memoryStorage(): SessionStorageLike {
  const map = new Map<string, string>()
  return {
    getItem: (key) => map.get(key) ?? null,
    setItem: (key, value) => {
      map.set(key, value)
    },
  }
}

describe('Connection', () => {
  it('reports connecting then open, and does not send Resume without a saved session', () => {
    const sockets: FakeWebSocket[] = []
    const statuses: string[] = []
    const conn = new Connection(
      'ws://x',
      () => {
        const s = new FakeWebSocket()
        sockets.push(s)
        return s
      },
      memoryStorage(),
      { onMessage: () => {}, onStatusChange: (s) => statuses.push(s.kind) },
    )

    conn.connect()
    expect(statuses).toEqual(['connecting'])

    sockets[0].onopen?.(undefined)
    expect(statuses).toEqual(['connecting', 'open'])
    expect(sockets[0].sent).toEqual([])
  })

  it('sends Resume with the saved session id once connected', () => {
    const sockets: FakeWebSocket[] = []
    const storage = memoryStorage()
    storage.setItem('gamerobotfactory.session_id', 'saved-session')
    const conn = new Connection(
      'ws://x',
      () => {
        const s = new FakeWebSocket()
        sockets.push(s)
        return s
      },
      storage,
      { onMessage: () => {}, onStatusChange: () => {} },
    )

    conn.connect()
    sockets[0].onopen?.(undefined)

    expect(sockets[0].sent).toEqual([JSON.stringify({ type: 'Resume', session_id: 'saved-session' })])
  })

  it('saves the session id from a Snapshot message and forwards the parsed message', () => {
    const sockets: FakeWebSocket[] = []
    const storage = memoryStorage()
    const messages: unknown[] = []
    const conn = new Connection(
      'ws://x',
      () => {
        const s = new FakeWebSocket()
        sockets.push(s)
        return s
      },
      storage,
      { onMessage: (m) => messages.push(m), onStatusChange: () => {} },
    )

    conn.connect()
    const raw = JSON.stringify({
      kind: 'Snapshot', v: 1, tick: 1, session_id: 'new-session', conveyor: { running: true }, robots: [],
    })
    sockets[0].onmessage?.({ data: raw })

    expect(storage.getItem('gamerobotfactory.session_id')).toBe('new-session')
    expect(messages).toHaveLength(1)
  })

  it('reconnects with exponential backoff after an unexpected close', () => {
    const sockets: FakeWebSocket[] = []
    const scheduled: Array<{ delayMs: number; fn: () => void }> = []
    const conn = new Connection(
      'ws://x',
      () => {
        const s = new FakeWebSocket()
        sockets.push(s)
        return s
      },
      memoryStorage(),
      { onMessage: () => {}, onStatusChange: () => {} },
      (delayMs, fn) => scheduled.push({ delayMs, fn }),
    )

    conn.connect()
    sockets[0].onclose?.(undefined)

    expect(scheduled).toHaveLength(1)
    expect(scheduled[0].delayMs).toBe(500)

    scheduled[0].fn() // 재연결 실행
    sockets[1].onclose?.(undefined) // 두 번째도 실패

    expect(scheduled).toHaveLength(2)
    expect(scheduled[1].delayMs).toBe(1000) // 지수 백오프: 500 -> 1000
  })

  it('does not reconnect after the user calls close()', () => {
    const sockets: FakeWebSocket[] = []
    const scheduled: Array<() => void> = []
    const conn = new Connection(
      'ws://x',
      () => {
        const s = new FakeWebSocket()
        sockets.push(s)
        return s
      },
      memoryStorage(),
      { onMessage: () => {}, onStatusChange: () => {} },
      (_delayMs, fn) => scheduled.push(fn),
    )

    conn.connect()
    conn.close()
    sockets[0].onclose?.(undefined)

    expect(scheduled).toHaveLength(0)
    expect(sockets[0].closed).toBe(true)
  })

  it('does not reconnect when close() lands during the backoff wait, even if the pending reconnect fires later', () => {
    const sockets: FakeWebSocket[] = []
    const scheduled: Array<() => void> = []
    const conn = new Connection(
      'ws://x',
      () => {
        const s = new FakeWebSocket()
        sockets.push(s)
        return s
      },
      memoryStorage(),
      { onMessage: () => {}, onStatusChange: () => {} },
      (_delayMs, fn) => scheduled.push(fn),
    )

    conn.connect()
    sockets[0].onclose?.(undefined) // 예기치 못한 종료 -> 재연결 타이머 예약 (소켓은 아직 null)

    expect(scheduled).toHaveLength(1)

    conn.close() // 대기 중에 사용자가 명시적으로 종료

    scheduled[0]() // 예약됐던 재연결 타이머가 뒤늦게 실행됨

    expect(sockets).toHaveLength(1) // 새 소켓이 만들어지면 안 된다
  })
})
