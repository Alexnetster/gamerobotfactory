import type { ConveyorView, RobotView } from '../net/protocol'
import type { ConnectionStatus } from '../net/connection'

export interface SidebarCallbacks {
  onToggleConveyor: () => void
  onChangeRobotCount: (delta: number) => void
  onSelectArmAction: (task: 'Idle' | 'Picking' | 'Placing') => void
  onRepair: () => void
  onTogglePathDebug: (enabled: boolean) => void
}

export interface SidebarState {
  connection: ConnectionStatus
  conveyor: ConveyorView
  robotCount: number
  selectedRobot: RobotView | null
  pathDebugEnabled: boolean
}

export class Sidebar {
  private readonly connectionEl: HTMLElement
  private readonly conveyorButton: HTMLButtonElement
  private readonly robotCountEl: HTMLElement
  private readonly pathToggle: HTMLInputElement
  private readonly selectedPanel: HTMLElement
  private readonly callbacks: SidebarCallbacks

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
