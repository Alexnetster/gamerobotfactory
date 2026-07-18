import { parseServerMessage, encodeClientCommand } from './protocol'
import type { ClientCommand, ServerMessage } from './protocol'

export type ConnectionStatus =
  | { kind: 'connecting' }
  | { kind: 'open' }
  | { kind: 'reconnecting'; attempt: number }

export interface ConnectionCallbacks {
  onMessage: (message: ServerMessage) => void
  onStatusChange: (status: ConnectionStatus) => void
}

/** 브라우저 `WebSocket`과 이 인터페이스만 맞으면 되므로, 테스트에서는
 * 진짜 소켓 없이 이 형태만 흉내내는 가짜 객체를 쓴다. */
export interface WebSocketLike {
  send(data: string): void
  close(): void
  onopen: ((this: WebSocketLike, ev: unknown) => void) | null
  onmessage: ((this: WebSocketLike, ev: { data: string }) => void) | null
  onclose: ((this: WebSocketLike, ev: unknown) => void) | null
  onerror: ((this: WebSocketLike, ev: unknown) => void) | null
}

export type WebSocketFactory = (url: string) => WebSocketLike

export interface SessionStorageLike {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
}

const SESSION_STORAGE_KEY = 'gamerobotfactory.session_id'
const BASE_RECONNECT_DELAY_MS = 500
const MAX_RECONNECT_DELAY_MS = 8000

export class Connection {
  private socket: WebSocketLike | null = null
  private reconnectAttempt = 0
  private closedByUser = false
  private generation = 0

  constructor(
    private readonly url: string,
    private readonly factory: WebSocketFactory,
    private readonly storage: SessionStorageLike,
    private readonly callbacks: ConnectionCallbacks,
    private readonly scheduleReconnect: (delayMs: number, fn: () => void) => void = (delayMs, fn) => setTimeout(fn, delayMs),
  ) {}

  connect(): void {
    this.closedByUser = false
    this.generation += 1
    const myGeneration = this.generation
    this.callbacks.onStatusChange(
      this.reconnectAttempt === 0 ? { kind: 'connecting' } : { kind: 'reconnecting', attempt: this.reconnectAttempt },
    )
    const socket = this.factory(this.url)
    this.socket = socket

    socket.onopen = () => {
      this.reconnectAttempt = 0
      this.callbacks.onStatusChange({ kind: 'open' })
      const savedSessionId = this.storage.getItem(SESSION_STORAGE_KEY)
      if (savedSessionId) {
        this.send({ type: 'Resume', session_id: savedSessionId })
      }
    }

    socket.onmessage = (ev) => {
      const message = parseServerMessage(ev.data)
      if (!message) {
        return // 서버의 "잘못된 메시지는 로그만 남기고 연결 유지" 정책과 대칭
      }
      if (message.kind === 'Snapshot') {
        this.storage.setItem(SESSION_STORAGE_KEY, message.session_id)
      }
      this.callbacks.onMessage(message)
    }

    socket.onclose = () => {
      if (this.closedByUser) {
        return
      }
      this.reconnectAttempt += 1
      const delay = Math.min(BASE_RECONNECT_DELAY_MS * 2 ** (this.reconnectAttempt - 1), MAX_RECONNECT_DELAY_MS)
      this.callbacks.onStatusChange({ kind: 'reconnecting', attempt: this.reconnectAttempt })
      this.scheduleReconnect(delay, () => {
        if (this.generation === myGeneration) {
          this.connect()
        }
      })
    }

    socket.onerror = () => {
      // onclose가 뒤따라 호출되므로 재연결 스케줄링은 onclose에 맡긴다.
    }
  }

  send(command: ClientCommand): void {
    this.socket?.send(encodeClientCommand(command))
  }

  close(): void {
    this.closedByUser = true
    this.generation += 1
    this.reconnectAttempt = 0
    this.socket?.close()
  }
}
