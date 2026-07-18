import { Connection } from './net/connection'
import type { ConnectionStatus } from './net/connection'
import { createEmptyMirror, applyServerMessage } from './state/mirror'
import type { MirrorState } from './state/mirror'
import { computeRenderRobots } from './state/interpolation'
import type { TickSnapshot } from './state/interpolation'
import { drawScene } from './render/canvas'
import { gridToScreen } from './render/projection'
import { Sidebar } from './ui/sidebar'
import type { ServerMessage } from './net/protocol'
import type { WebSocketLike } from './net/connection'
import { resolveWsUrl } from './net/resolve-ws-url'

// U자 컨베이어가 이소메트릭 각도에서 알아볼 수 있는 최소 크기(브레인스토밍
// 목업 기준 7x6 이상 권장).
const GRID_SIZE = { width: 9, height: 7 }

function setupLayout(): { canvas: HTMLCanvasElement; sidebarContainer: HTMLElement } {
  const app = document.getElementById('app')
  if (!app) {
    throw new Error('#app element not found')
  }
  app.innerHTML = ''

  const canvas = document.createElement('canvas')
  canvas.style.flex = '1'
  // <canvas>는 대체 요소(replaced element)라 기본 width/height 속성(300x150,
  // 2:1 비율)이 flex 아이템의 자동 min-width 계산에 그대로 들어간다. 세로
  // 축(cross-axis)이 컨테이너 높이만큼 stretch되면, 그 비율을 유지하려는
  // 자동 최소 너비가 (stretch된 높이 × 2)로 튀어올라 flex-basis:0/grow와
  // 무관하게 캔버스가 훨씬 넓게 잡히고 사이드바가 화면 밖으로 밀려난다
  // (Playwright E2E로 실측: 뷰포트 1000px인데 캔버스가 1400px로 렌더링됨).
  // min-width:0으로 이 자동 최소값을 꺼야 flex-grow/shrink가 실제 남는
  // 공간(뷰포트 - 사이드바)에 맞춰 캔버스 크기를 정상적으로 계산한다.
  canvas.style.minWidth = '0'
  app.appendChild(canvas)

  const sidebarContainer = document.createElement('div')
  sidebarContainer.style.width = '280px'
  sidebarContainer.style.flexShrink = '0'
  app.appendChild(sidebarContainer)

  function resizeCanvas(): void {
    canvas.width = canvas.clientWidth
    canvas.height = canvas.clientHeight
  }
  window.addEventListener('resize', resizeCanvas)
  resizeCanvas()

  return { canvas, sidebarContainer }
}

function main(): void {
  const wsUrl = resolveWsUrl(location.search, location.protocol, location.host)
  const { canvas, sidebarContainer } = setupLayout()
  const ctx2d = canvas.getContext('2d')
  if (!ctx2d) {
    throw new Error('2D canvas context unavailable')
  }
  const ctx: CanvasRenderingContext2D = ctx2d

  let mirror: MirrorState = createEmptyMirror()
  let prevSnapshot: TickSnapshot | null = null
  let currSnapshot: TickSnapshot | null = null
  let connectionStatus: ConnectionStatus = { kind: 'connecting' }
  let selectedRobotId: number | null = null
  let pathDebugEnabled = false

  const sidebar = new Sidebar(sidebarContainer, {
    onToggleConveyor: () => connection.send({ type: 'ToggleConveyor' }),
    onChangeRobotCount: (delta) => {
      const nextCount = Math.max(0, mirror.robots.size + delta)
      connection.send({ type: 'SetRobotCount', count: nextCount })
    },
    onSelectArmAction: (task) => {
      if (selectedRobotId !== null) {
        connection.send({ type: 'TriggerArmAction', robot_id: selectedRobotId, task })
      }
    },
    onRepair: () => {
      if (selectedRobotId !== null) {
        connection.send({ type: 'RepairRobot', robot_id: selectedRobotId })
      }
    },
    onTogglePathDebug: (enabled) => {
      pathDebugEnabled = enabled
      renderSidebar()
    },
  })

  // 사이드바는 서버 틱(applyServerMessage)이나 로컬 상태(연결 상태/선택/토글)가
  // 실제로 바뀔 때만 다시 그린다 — requestAnimationFrame 루프(60fps)마다
  // 선택 패널의 DOM을 통째로 재생성하면, 사람이 버튼을 클릭하는 동안(mousedown~
  // mouseup 수십~수백ms) 그 버튼이 매 프레임 교체되면서 클릭이 누락될 수
  // 있다(Playwright 자동화 클릭에서 "element was detached from the DOM,
  // retrying"으로 실제 재현됨). 캔버스 애니메이션(drawScene)은 여전히
  // frame()에서 매 프레임 그린다.
  function renderSidebar(): void {
    sidebar.update({
      connection: connectionStatus,
      conveyor: mirror.conveyor,
      robotCount: mirror.robots.size,
      selectedRobot: selectedRobotId !== null ? (mirror.robots.get(selectedRobotId) ?? null) : null,
      pathDebugEnabled,
    })
  }

  function handleMessage(message: ServerMessage): void {
    mirror = applyServerMessage(mirror, message)
    if (message.kind === 'Snapshot' || message.kind === 'Delta') {
      prevSnapshot = currSnapshot
      currSnapshot = { mirror, receivedAtMs: performance.now() }
    }
    renderSidebar()
  }

  const connection = new Connection(wsUrl, (url) => new WebSocket(url) as unknown as WebSocketLike, window.sessionStorage, {
    onMessage: handleMessage,
    onStatusChange: (status) => {
      connectionStatus = status
      renderSidebar()
    },
  })
  connection.connect()
  renderSidebar()

  canvas.addEventListener('click', (ev) => {
    if (!currSnapshot) return
    const rect = canvas.getBoundingClientRect()
    const clickX = ev.clientX - rect.left - canvas.width / 2
    const clickY = ev.clientY - rect.top - 40
    const rendered = computeRenderRobots(prevSnapshot, currSnapshot, performance.now())

    // 가장 가까운 로봇을 선택한다 — 로봇 수가 적은 v1 스코프에서는 정밀한
    // 폴리곤 히트테스트 없이 화면 좌표 거리만으로 충분히 정확하다.
    let closestId: number | null = null
    let closestDist = Infinity
    for (const robot of rendered) {
      const screen = gridToScreen(robot.renderPos.x, robot.renderPos.y)
      const dist = Math.hypot(screen.x - clickX, screen.y - clickY)
      if (dist < closestDist) {
        closestDist = dist
        closestId = robot.id
      }
    }
    if (closestId !== null && closestDist < 24) {
      selectedRobotId = closestId
      connection.send({ type: 'SelectRobot', robot_id: closestId })
      renderSidebar()
    }
  })

  function frame(): void {
    const now = performance.now()
    const rendered = currSnapshot ? computeRenderRobots(prevSnapshot, currSnapshot, now) : []
    drawScene(ctx, canvas.width, canvas.height, {
      grid: GRID_SIZE,
      conveyor: mirror.conveyor,
      robots: rendered,
      showPaths: pathDebugEnabled,
      animationTimeMs: now,
      selectedRobotId,
    })
    requestAnimationFrame(frame)
  }
  requestAnimationFrame(frame)
}

main()
