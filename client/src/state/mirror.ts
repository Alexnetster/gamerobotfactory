import type { ConveyorView, ProductView, RobotView, ServerMessage, StationView } from '../net/protocol'

export interface MirrorState {
  conveyor: ConveyorView
  robots: Map<number, RobotView>
  stations: StationView[]
  products: Map<number, ProductView>
}

export function createEmptyMirror(): MirrorState {
  return { conveyor: { running: false }, robots: new Map(), stations: [], products: new Map() }
}

/** 서버의 Snapshot/Delta 프로토콜을 그대로 재생하는 순수 함수. 입력
 * `mirror`를 절대 제자리에서 고치지 않는다 — 항상 새 객체를 반환한다. */
export function applyServerMessage(mirror: MirrorState, message: ServerMessage): MirrorState {
  switch (message.kind) {
    case 'Snapshot':
      return {
        conveyor: message.conveyor,
        robots: new Map(message.robots.map((r) => [r.id, r])),
        stations: message.stations,
        products: new Map(message.products.map((p) => [p.id, p])),
      }
    case 'Delta': {
      const robots = new Map(mirror.robots)
      for (const robot of message.changed_robots) {
        robots.set(robot.id, robot)
      }
      for (const id of message.removed_robot_ids) {
        robots.delete(id)
      }
      const products = new Map(mirror.products)
      for (const product of message.changed_products) {
        products.set(product.id, product)
      }
      for (const id of message.removed_product_ids) {
        products.delete(id)
      }
      return {
        conveyor: message.conveyor ?? mirror.conveyor,
        robots,
        // 서버가 매 Delta마다 항상 전체 스테이션 목록(3개)을 보내므로 보통 이 조건은
        // 항상 참이지만, 빈 배열이 오는 예외적인 경우 기존 데이터를 잃지 않도록
        // 방어적으로 유지한다.
        stations: message.stations.length > 0 ? message.stations : mirror.stations,
        products,
      }
    }
    case 'ResumeAck':
      return mirror
  }
}
