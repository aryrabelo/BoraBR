import { computed, ref } from 'vue'

export const TERMINAL_PANEL_MIN_HEIGHT = 160
export const TERMINAL_PANEL_DEFAULT_HEIGHT = 280
export const TERMINAL_PANEL_MAX_HEIGHT = 560

export type TerminalSessionStatus = 'starting' | 'running' | 'exited' | 'error'

export interface TerminalPanelIssueContext {
  id: string
  title?: string
}

export interface CreateTerminalPanelSessionInput {
  projectPath: string
  projectName?: string
  issue?: TerminalPanelIssueContext | null
  label?: string
  backendSessionId?: string
}

export interface TerminalPanelSession {
  id: string
  backendSessionId?: string
  projectPath: string
  projectName: string
  issueId?: string
  issueTitle?: string
  label: string
  buffer: string
  status: TerminalSessionStatus
  restartKey: number
  createdAt: number
  updatedAt: number
  error?: string
}

interface UseTerminalPanelOptions {
  initialOpen?: boolean
  initialHeight?: number
  idFactory?: () => string
}

let terminalSessionCounter = 0

function clampHeight(height: number, maxHeight = TERMINAL_PANEL_MAX_HEIGHT): number {
  const effectiveMax = Math.max(TERMINAL_PANEL_MIN_HEIGHT, Math.min(maxHeight, TERMINAL_PANEL_MAX_HEIGHT))
  return Math.min(Math.max(height, TERMINAL_PANEL_MIN_HEIGHT), effectiveMax)
}

function projectNameFromPath(path: string): string {
  const normalized = path.replace(/\\/g, '/').replace(/\/$/, '')
  return normalized.split('/').filter(Boolean).pop() || 'Project'
}

function replaceSession(
  sessions: TerminalPanelSession[],
  id: string,
  update: (session: TerminalPanelSession) => TerminalPanelSession,
): TerminalPanelSession[] {
  return sessions.map(session => session.id === id ? update(session) : session)
}

export function useTerminalPanel(options: UseTerminalPanelOptions = {}) {
  const idFactory = options.idFactory ?? (() => `terminal-${++terminalSessionCounter}`)
  const isOpen = ref(options.initialOpen ?? false)
  const height = ref(clampHeight(options.initialHeight ?? TERMINAL_PANEL_DEFAULT_HEIGHT))
  const sessions = ref<TerminalPanelSession[]>([])
  const activeSessionId = ref<string | null>(null)

  const activeSession = computed(() => {
    if (!activeSessionId.value) return null
    return sessions.value.find(session => session.id === activeSessionId.value) ?? null
  })

  const setHeight = (nextHeight: number, maxHeight?: number) => {
    height.value = clampHeight(nextHeight, maxHeight)
  }

  const openPanel = () => {
    isOpen.value = true
  }

  const closePanel = () => {
    isOpen.value = false
  }

  const togglePanel = () => {
    isOpen.value = !isOpen.value
  }

  const setActiveSession = (id: string | null) => {
    if (id === null) {
      activeSessionId.value = null
      return
    }
    if (sessions.value.some(session => session.id === id)) {
      activeSessionId.value = id
      isOpen.value = true
    }
  }

  const createSession = (input: CreateTerminalPanelSessionInput): TerminalPanelSession => {
    const projectName = input.projectName || projectNameFromPath(input.projectPath)
    const label = input.label || (input.issue?.id ? `${projectName} / ${input.issue.id}` : `${projectName} / session ${sessions.value.length + 1}`)
    const now = Date.now()
    const session: TerminalPanelSession = {
      id: idFactory(),
      backendSessionId: input.backendSessionId,
      projectPath: input.projectPath,
      projectName,
      issueId: input.issue?.id,
      issueTitle: input.issue?.title,
      label,
      buffer: '',
      status: input.backendSessionId ? 'running' : 'starting',
      restartKey: 0,
      createdAt: now,
      updatedAt: now,
    }

    sessions.value = [...sessions.value, session]
    activeSessionId.value = session.id
    isOpen.value = true
    return session
  }

  const closeSession = (id: string) => {
    const closingIndex = sessions.value.findIndex(session => session.id === id)
    if (closingIndex === -1) return

    const wasActive = activeSessionId.value === id
    sessions.value = sessions.value.filter(session => session.id !== id)

    if (sessions.value.length === 0) {
      activeSessionId.value = null
      isOpen.value = false
      return
    }

    if (wasActive) {
      activeSessionId.value = sessions.value[closingIndex]?.id ?? sessions.value[closingIndex - 1]?.id ?? sessions.value[0]!.id
    }
  }

  const appendOutput = (id: string, output: string) => {
    const now = Date.now()
    sessions.value = replaceSession(sessions.value, id, session => ({
      ...session,
      buffer: session.buffer + output,
      updatedAt: now,
    }))
  }

  const clearSession = (id: string) => {
    const now = Date.now()
    sessions.value = replaceSession(sessions.value, id, session => ({
      ...session,
      buffer: '',
      updatedAt: now,
    }))
  }

  const markRunning = (id: string, backendSessionId?: string) => {
    const now = Date.now()
    sessions.value = replaceSession(sessions.value, id, session => ({
      ...session,
      backendSessionId: backendSessionId ?? session.backendSessionId,
      status: 'running',
      error: undefined,
      updatedAt: now,
    }))
  }

  const markExited = (id: string) => {
    const now = Date.now()
    sessions.value = replaceSession(sessions.value, id, session => ({
      ...session,
      status: 'exited',
      updatedAt: now,
    }))
  }

  const markError = (id: string, error: string) => {
    const now = Date.now()
    sessions.value = replaceSession(sessions.value, id, session => ({
      ...session,
      status: 'error',
      error,
      updatedAt: now,
    }))
  }

  const restartSession = (id: string, backendSessionId?: string): TerminalPanelSession | null => {
    let restarted: TerminalPanelSession | null = null
    const now = Date.now()
    sessions.value = replaceSession(sessions.value, id, session => {
      restarted = {
        ...session,
        backendSessionId: backendSessionId ?? session.backendSessionId,
        buffer: '',
        status: 'starting',
        error: undefined,
        restartKey: session.restartKey + 1,
        updatedAt: now,
      }
      return restarted
    })
    return restarted
  }

  return {
    isOpen,
    height,
    sessions,
    activeSessionId,
    activeSession,
    setHeight,
    openPanel,
    closePanel,
    togglePanel,
    setActiveSession,
    createSession,
    closeSession,
    appendOutput,
    clearSession,
    markRunning,
    markExited,
    markError,
    restartSession,
  }
}
