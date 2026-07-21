// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { Sidebar } from '../../src/ui/sidebar'
import type { SidebarCallbacks } from '../../src/ui/sidebar'
import type { RobotView } from '../../src/net/protocol'

function robot(overrides: Partial<RobotView> = {}): RobotView {
  return {
    id: 1,
    pos: { x: 0, y: 0 },
    pose: 'Standing',
    leg_cycle_progress: 0,
    task: 'Idle',
    status: { kind: 'Operational' },
    durability_remaining: 0.8,
    path: [],
    facing: 'East',
    arm_pose: { shoulder_angle: 0, elbow_angle: 0 },
    carrying: false,
    ...overrides,
  }
}

function makeSidebar(container: HTMLElement, overrides: Partial<SidebarCallbacks> = {}): Sidebar {
  const callbacks: SidebarCallbacks = {
    onToggleConveyor: vi.fn(),
    onChangeRobotCount: vi.fn(),
    onSelectArmAction: vi.fn(),
    onRepair: vi.fn(),
    onTogglePathDebug: vi.fn(),
    ...overrides,
  }
  return new Sidebar(container, callbacks)
}

describe('Sidebar', () => {
  it('shows "선택된 로봇 없음" when nothing is selected', () => {
    const container = document.createElement('div')
    const sidebar = makeSidebar(container)

    sidebar.update({ connection: { kind: 'open' }, conveyor: { running: true }, robotCount: 3, selectedRobot: null, pathDebugEnabled: false })

    expect(container.textContent).toContain('선택된 로봇 없음')
  })

  it('renders the selected robot battery/task', () => {
    const container = document.createElement('div')
    const sidebar = makeSidebar(container)

    sidebar.update({
      connection: { kind: 'open' }, conveyor: { running: true }, robotCount: 1,
      selectedRobot: robot({ durability_remaining: 0.45, task: 'Picking' }), pathDebugEnabled: false,
    })

    expect(container.textContent).toContain('45%')
    expect(container.textContent).toContain('Picking')
  })

  it('disables the repair button unless the robot is Failed', () => {
    const container = document.createElement('div')
    const sidebar = makeSidebar(container)

    sidebar.update({ connection: { kind: 'open' }, conveyor: { running: true }, robotCount: 1, selectedRobot: robot(), pathDebugEnabled: false })
    const repairButtonWhileOperational = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === '수리') as HTMLButtonElement
    expect(repairButtonWhileOperational.disabled).toBe(true)

    sidebar.update({
      connection: { kind: 'open' }, conveyor: { running: true }, robotCount: 1,
      selectedRobot: robot({ status: { kind: 'Failed' } }), pathDebugEnabled: false,
    })
    const repairButtonWhileFailed = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === '수리') as HTMLButtonElement
    expect(repairButtonWhileFailed.disabled).toBe(false)
  })

  it('calls onRepair when the repair button is clicked', () => {
    const container = document.createElement('div')
    const onRepair = vi.fn()
    const sidebar = makeSidebar(container, { onRepair })

    sidebar.update({
      connection: { kind: 'open' }, conveyor: { running: true }, robotCount: 1,
      selectedRobot: robot({ status: { kind: 'Failed' } }), pathDebugEnabled: false,
    })
    const repairButton = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === '수리') as HTMLButtonElement
    repairButton.click()

    expect(onRepair).toHaveBeenCalledOnce()
  })

  it('shows the reconnecting attempt count', () => {
    const container = document.createElement('div')
    const sidebar = makeSidebar(container)

    sidebar.update({
      connection: { kind: 'reconnecting', attempt: 3 }, conveyor: { running: true }, robotCount: 0, selectedRobot: null, pathDebugEnabled: false,
    })

    expect(container.textContent).toContain('3')
  })

  it('calls onToggleConveyor when the conveyor button is clicked', () => {
    const container = document.createElement('div')
    const onToggleConveyor = vi.fn()
    const sidebar = makeSidebar(container, { onToggleConveyor })

    sidebar.update({ connection: { kind: 'open' }, conveyor: { running: true }, robotCount: 0, selectedRobot: null, pathDebugEnabled: false })
    const button = Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.includes('컨베이어')) as HTMLButtonElement
    button.click()

    expect(onToggleConveyor).toHaveBeenCalledOnce()
  })
})
