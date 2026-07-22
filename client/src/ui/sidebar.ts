import type { ConveyorView, RobotView } from '../net/protocol'
import type { ConnectionStatus } from '../net/connection'

export interface SidebarCallbacks {
  onToggleConveyor: () => void
  onChangeRobotCount: (delta: number) => void
  onSelectArmAction: (task: 'Idle' | 'Picking' | 'Placing') => void
  onRepair: () => void
  onRepairAll: () => void
  onTogglePathDebug: (enabled: boolean) => void
}

export interface SidebarState {
  connection: ConnectionStatus
  conveyor: ConveyorView
  robotCount: number
  selectedRobot: RobotView | null
  pathDebugEnabled: boolean
}

/** 선택 패널을 다시 그려야 하는지 판단하는 데 쓰는 값만 뽑는다 — 로봇의
 * 위치/leg_cycle_progress/arm_pose 등은 매 틱(20Hz) 바뀌지만 패널에는
 * 안 보이므로 이 값들이 바뀌었다고 다시 그릴 필요가 없다. */
function selectedPanelSignature(robot: RobotView | null): string {
  if (!robot) return 'none'
  return `${robot.id}|${robot.status.kind}|${robot.status.kind === 'Repairing' ? robot.status.remaining_ticks : ''}|${robot.task}|${robot.durability_remaining}`
}

export class Sidebar {
  private readonly connectionEl: HTMLElement
  private readonly conveyorButton: HTMLButtonElement
  private readonly robotCountEl: HTMLElement
  private readonly pathToggle: HTMLInputElement
  private readonly selectedPanel: HTMLElement
  private readonly callbacks: SidebarCallbacks
  private lastSelectedPanelSignature: string | null = null

  constructor(container: HTMLElement, callbacks: SidebarCallbacks) {
    this.callbacks = callbacks

    const root = document.createElement('div')
    root.className = 'sidebar'
    container.appendChild(root)

    const globalSection = document.createElement('section')
    this.connectionEl = document.createElement('p')
    this.connectionEl.className = 'connection-status'
    globalSection.appendChild(this.connectionEl)

    this.conveyorButton = document.createElement('button')
    this.conveyorButton.addEventListener('click', () => callbacks.onToggleConveyor())
    globalSection.appendChild(this.conveyorButton)

    const decButton = document.createElement('button')
    decButton.textContent = '-'
    decButton.addEventListener('click', () => callbacks.onChangeRobotCount(-1))
    const incButton = document.createElement('button')
    incButton.textContent = '+'
    incButton.addEventListener('click', () => callbacks.onChangeRobotCount(1))
    this.robotCountEl = document.createElement('span')
    this.robotCountEl.className = 'robot-count'
    globalSection.appendChild(decButton)
    globalSection.appendChild(this.robotCountEl)
    globalSection.appendChild(incButton)

    // 개별 로봇을 하나씩 선택해서 수리하는 게 번거로울 만큼 여러 대가
    // 동시에 고장나는 경우를 위한 일괄 수리 — 고장난 로봇이 하나도 없어도
    // 눌러서 해로울 게 없으므로(서버가 조용히 무시함) 항상 활성 상태로 둔다.
    const repairAllButton = document.createElement('button')
    repairAllButton.textContent = '전체 수리'
    repairAllButton.addEventListener('click', () => callbacks.onRepairAll())
    globalSection.appendChild(repairAllButton)

    const pathLabel = document.createElement('label')
    this.pathToggle = document.createElement('input')
    this.pathToggle.type = 'checkbox'
    this.pathToggle.addEventListener('change', () => callbacks.onTogglePathDebug(this.pathToggle.checked))
    pathLabel.appendChild(this.pathToggle)
    pathLabel.appendChild(document.createTextNode('경로 표시'))
    globalSection.appendChild(pathLabel)

    root.appendChild(globalSection)

    this.selectedPanel = document.createElement('section')
    this.selectedPanel.className = 'selected-robot-panel'
    root.appendChild(this.selectedPanel)
  }

  update(state: SidebarState): void {
    this.connectionEl.textContent = connectionStatusLabel(state.connection)
    this.conveyorButton.textContent = state.conveyor.running ? '컨베이어 끄기' : '컨베이어 켜기'
    this.robotCountEl.textContent = String(state.robotCount)
    this.pathToggle.checked = state.pathDebugEnabled

    // 선택 패널은 표시 내용이 실제로 바뀔 때만 다시 그린다 — 서버 메시지는
    // 20Hz(초당 20번)로 계속 오는데, 그때마다 버튼을 통째로 지우고 새로
    // 만들면 사람이 마우스로 누르는 동안(mousedown~mouseup 사이 100~300ms)
    // 그 버튼이 여러 번 교체되면서 클릭이 이미 사라진 버튼에 떨어져 무시될
    // 수 있다(실사용 중 "수리 버튼을 눌러도 반응이 없다"로 실제 발견됨).
    const signature = selectedPanelSignature(state.selectedRobot)
    if (signature === this.lastSelectedPanelSignature) {
      return
    }
    this.lastSelectedPanelSignature = signature

    this.selectedPanel.innerHTML = ''
    if (!state.selectedRobot) {
      const empty = document.createElement('p')
      empty.textContent = '선택된 로봇 없음'
      this.selectedPanel.appendChild(empty)
      return
    }

    const robot = state.selectedRobot
    const info = document.createElement('p')
    info.textContent = `로봇 #${robot.id} · 배터리 ${Math.round(robot.durability_remaining * 100)}% · ${robot.task} · ${statusLabel(robot.status)}`
    this.selectedPanel.appendChild(info)

    for (const task of ['Idle', 'Picking', 'Placing'] as const) {
      const button = document.createElement('button')
      button.textContent = task
      button.disabled = robot.status.kind !== 'Operational'
      button.addEventListener('click', () => this.callbacks.onSelectArmAction(task))
      this.selectedPanel.appendChild(button)
    }

    const repairButton = document.createElement('button')
    repairButton.textContent = '수리'
    repairButton.disabled = robot.status.kind !== 'Failed'
    repairButton.addEventListener('click', () => this.callbacks.onRepair())
    this.selectedPanel.appendChild(repairButton)
  }
}

function connectionStatusLabel(status: ConnectionStatus): string {
  switch (status.kind) {
    case 'connecting':
      return '연결 중...'
    case 'open':
      return '🟢 연결됨'
    case 'reconnecting':
      return `🔴 재연결 중... (시도 ${status.attempt})`
  }
}

function statusLabel(status: RobotView['status']): string {
  switch (status.kind) {
    case 'Operational':
      return 'Operational'
    case 'Failed':
      return 'Failed'
    case 'Repairing':
      return `Repairing (${status.remaining_ticks})`
  }
}
