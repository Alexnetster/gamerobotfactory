import { describe, it, expect } from 'vitest'
import { resolveWsUrl } from '../../src/net/resolve-ws-url'

describe('resolveWsUrl', () => {
  it('uses the ?ws= override when present, regardless of protocol', () => {
    expect(resolveWsUrl('?ws=ws://127.0.0.1:54321/ws', 'http:', 'localhost:5173')).toBe('ws://127.0.0.1:54321/ws')
  })

  it('derives wss:// from the same origin when protocol is https and no override is given', () => {
    expect(resolveWsUrl('', 'https:', 'gamerobotfactory.fly.dev')).toBe('wss://gamerobotfactory.fly.dev/ws')
  })

  it('derives ws:// from the same origin when protocol is http and no override is given', () => {
    expect(resolveWsUrl('', 'http:', 'localhost:8080')).toBe('ws://localhost:8080/ws')
  })
})
