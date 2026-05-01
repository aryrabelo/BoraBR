import type { Issue } from '~/types/issue'

export type TaskTerminalSource =
  | {
      origin: 'external-cmux'
      surfaceId: string
      label: string
      command: string[]
    }
  | {
      origin: 'embedded'
      label: string
    }
  | {
      origin: 'unknown'
      label: string
    }

export interface TaskTerminalAssigneeDisplay {
  primary: string
  secondary: string | null
  title: string
}

export interface FocusTaskTerminalSourceActions {
  focusCmuxSurface: (surfaceId: string) => Promise<void>
  openEmbedded: () => void
  showError?: (message: string) => void
}

export type FocusTaskTerminalSourceResult = 'external-focused' | 'external-error' | 'embedded-opened' | 'unknown'

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function extractCmuxSurfaceId(assignee: string): string | null {
  const match = assignee.match(/(?:^|\s)cmux:\s*\{?([A-Za-z0-9:_-]+)/)
  return match?.[1]?.replace(/\}+$/, '') ?? null
}

export function resolveTaskTerminalSource(issue: Pick<Issue, 'assignee'>): TaskTerminalSource {
  const assignee = issue.assignee?.trim()
  if (!assignee) {
    return { origin: 'embedded', label: 'embedded' }
  }

  if (!assignee.includes('cmux:')) {
    return { origin: 'embedded', label: 'embedded' }
  }

  const surfaceId = extractCmuxSurfaceId(assignee)
  if (!surfaceId) {
    return { origin: 'unknown', label: 'cmux' }
  }

  return {
    origin: 'external-cmux',
    surfaceId,
    label: `cmux:${surfaceId.slice(0, 8)}`,
    command: ['cmux', 'focus-surface', '--surface', surfaceId],
  }
}

export function buildTaskTerminalAssigneeDisplay(issue: Pick<Issue, 'assignee'>): TaskTerminalAssigneeDisplay {
  const source = resolveTaskTerminalSource(issue)
  if (source.origin === 'external-cmux') {
    return {
      primary: 'cmux',
      secondary: source.surfaceId.slice(0, 8),
      title: `External cmux surface ${source.surfaceId}`,
    }
  }

  const assignee = issue.assignee?.trim() || '-'
  return {
    primary: assignee,
    secondary: null,
    title: assignee,
  }
}

export async function focusTaskTerminalSource(
  source: TaskTerminalSource,
  actions: FocusTaskTerminalSourceActions,
): Promise<FocusTaskTerminalSourceResult> {
  if (source.origin === 'external-cmux') {
    try {
      await actions.focusCmuxSurface(source.surfaceId)
      return 'external-focused'
    } catch (error) {
      actions.showError?.(`Could not focus cmux surface ${source.surfaceId}: ${errorMessage(error)}`)
      return 'external-error'
    }
  }

  if (source.origin === 'embedded') {
    actions.openEmbedded()
    return 'embedded-opened'
  }

  return 'unknown'
}
