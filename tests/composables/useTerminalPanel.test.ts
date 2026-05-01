import { describe, expect, it } from 'vitest'
import {
  TERMINAL_PANEL_MAX_HEIGHT,
  TERMINAL_PANEL_MIN_HEIGHT,
  useTerminalPanel,
} from '~/composables/useTerminalPanel'

describe('useTerminalPanel', () => {
  it('opens the panel and labels the first project issue session', () => {
    const panel = useTerminalPanel({ idFactory: () => 'session-1' })

    const session = panel.createSession({
      projectPath: '/work/beads',
      projectName: 'beads',
      issue: { id: 'borabr-m0z.3', title: 'Build terminal panel' },
    })

    expect(panel.isOpen.value).toBe(true)
    expect(panel.sessions.value).toHaveLength(1)
    expect(panel.activeSession.value?.id).toBe('session-1')
    expect(session.label).toBe('beads / borabr-m0z.3')
    expect(session.projectPath).toBe('/work/beads')
    expect(session.issueTitle).toBe('Build terminal panel')
  })

  it('clamps resize height between panel limits', () => {
    const panel = useTerminalPanel({ initialHeight: 300 })

    panel.setHeight(TERMINAL_PANEL_MIN_HEIGHT - 100)
    expect(panel.height.value).toBe(TERMINAL_PANEL_MIN_HEIGHT)

    panel.setHeight(420)
    expect(panel.height.value).toBe(420)

    panel.setHeight(TERMINAL_PANEL_MAX_HEIGHT + 100)
    expect(panel.height.value).toBe(TERMINAL_PANEL_MAX_HEIGHT)
  })

  it('closing the active tab selects a neighbor and hides after the last tab closes', () => {
    let nextId = 0
    const panel = useTerminalPanel({ idFactory: () => `session-${++nextId}` })

    const first = panel.createSession({ projectPath: '/repo', projectName: 'repo' })
    const second = panel.createSession({ projectPath: '/repo', projectName: 'repo' })
    const third = panel.createSession({ projectPath: '/repo', projectName: 'repo' })

    expect(panel.activeSession.value?.id).toBe(third.id)

    panel.setActiveSession(second.id)
    panel.closeSession(second.id)
    expect(panel.activeSession.value?.id).toBe(third.id)

    panel.closeSession(third.id)
    expect(panel.activeSession.value?.id).toBe(first.id)

    panel.closeSession(first.id)
    expect(panel.sessions.value).toHaveLength(0)
    expect(panel.activeSession.value).toBeNull()
    expect(panel.isOpen.value).toBe(false)
  })

  it('clears output without closing a session', () => {
    const panel = useTerminalPanel({ idFactory: () => 'session-1' })
    const session = panel.createSession({ projectPath: '/repo', projectName: 'repo' })

    panel.appendOutput(session.id, 'hello')
    panel.appendOutput(session.id, ' world')
    expect(panel.activeSession.value?.buffer).toBe('hello world')

    panel.clearSession(session.id)
    expect(panel.sessions.value).toHaveLength(1)
    expect(panel.activeSession.value?.buffer).toBe('')
  })

  it('restarting a session preserves the tab id and marks a new restart generation', () => {
    const panel = useTerminalPanel({ idFactory: () => 'session-1' })
    const session = panel.createSession({ projectPath: '/repo', projectName: 'repo' })

    panel.markRunning(session.id, 'backend-1')
    const restarted = panel.restartSession(session.id, 'backend-2')

    expect(restarted?.id).toBe(session.id)
    expect(restarted?.backendSessionId).toBe('backend-2')
    expect(restarted?.restartKey).toBe(1)
    expect(restarted?.status).toBe('starting')
  })
})
