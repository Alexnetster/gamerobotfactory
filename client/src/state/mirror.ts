import type { ConveyorView, RobotView, ServerMessage } from '../net/protocol'

export interface MirrorState {
  conveyor: ConveyorView
  robots: Map<number, RobotView>
}

export function createEmptyMirror(): MirrorState {
  return { conveyor: { running: false }, robots: new Map() }
}

/** 서버의 Snapshot/Delta 프로토콜을 그대로 재생하는 순수 함수. 입력
 * `mirror`를 절대 제자리에서 고치지 않는다 — 항상 새 객체를 반환한다. */
export function applyServerMessage(mirror: MirrorState, message: ServerMessage): MirrorState {
  switch (message.kind) {
    case 'Snapshot':
      return {
        conveyor: message.conveyor,
        robots: new Map(message.robots.map((r) => [r.id, r])),
      }
    case 'Delta': {
      const robots = new Map(mirror.robots)
      for (const robot of message.changed_robots) {
        robots.set(robot.id, robot)
      }
      for (const id of message.removed_robot_ids) {
        robots.delete(id)
      }
      return {
        conveyor: message.conveyor ?? mirror.conveyor,
        robots,
      }
    }
    case 'ResumeAck':
      return mirror
  }
}
