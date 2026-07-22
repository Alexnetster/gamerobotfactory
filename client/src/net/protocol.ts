// server/src/protocol.rs의 와이어 타입을 그대로 미러링한다. 서버가 이미
// 모든 커맨드를 검증하므로(존재하지 않는 로봇 거부 등) 여기엔 별도
// 런타임 스키마 검증을 두지 않는다.

export interface WireCellId {
  x: number
  y: number
}

export type WireTask = 'Idle' | 'Picking' | 'Placing'

export type WirePose = 'Standing' | 'Crouching'

export type WireDirection = 'North' | 'East' | 'South' | 'West'

export interface WireArmPose {
  shoulder_angle: number
  elbow_angle: number
}

export type WireStatus =
  | { kind: 'Operational' }
  | { kind: 'Failed' }
  | { kind: 'Repairing'; remaining_ticks: number }

export interface RobotView {
  id: number
  pos: WireCellId
  pose: WirePose
  leg_cycle_progress: number
  task: WireTask
  status: WireStatus
  durability_remaining: number
  path: WireCellId[]
  facing: WireDirection
  arm_pose: WireArmPose
  carrying: boolean
}

export interface ConveyorView {
  running: boolean
}

export type ServerMessage =
  | { kind: 'Snapshot'; v: number; tick: number; session_id: string; conveyor: ConveyorView; robots: RobotView[] }
  | {
      kind: 'Delta'
      v: number
      tick: number
      conveyor: ConveyorView | null
      changed_robots: RobotView[]
      removed_robot_ids: number[]
    }
  | { kind: 'ResumeAck'; v: number; session_id: string; resumed: boolean }

export type ClientCommand =
  | { type: 'SelectRobot'; robot_id: number }
  | { type: 'ReleaseRobot' }
  | { type: 'ToggleConveyor' }
  | { type: 'SetRobotCount'; count: number }
  | { type: 'TriggerArmAction'; robot_id: number; task: WireTask }
  | { type: 'RepairRobot'; robot_id: number }
  | { type: 'RepairAllRobots' }
  | { type: 'Resume'; session_id: string }

/** 파싱 실패(잘못된 JSON, `kind` 없음)는 예외 대신 `null` — 서버의
 * "잘못된 메시지는 로그만 남기고 연결 유지" 정책과 대칭. */
export function parseServerMessage(raw: string): ServerMessage | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return null
  }
  if (typeof parsed !== 'object' || parsed === null || typeof (parsed as { kind?: unknown }).kind !== 'string') {
    return null
  }
  return parsed as ServerMessage
}

export function encodeClientCommand(command: ClientCommand): string {
  return JSON.stringify(command)
}
