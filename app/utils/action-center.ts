import type { Issue } from '~/types/issue'

export interface ActionCenterProjectActionState {
  projectPath: string
  projectName: string
  inProgressCount: number
  cmuxSurfaceId?: string
  readyIssues: Issue[]
}

export interface ActionCenterProjectIdleState {
  projectPath: string
  projectName: string
  idleSince: number
}

interface BuildActionCenterProjectActionStateOptions {
  projectPath: string
  projectName: string
  projectIssues: Pick<Issue, 'status'>[]
  readyIssues: Issue[]
  cmuxSurfaceId?: string
}

interface BuildActionCenterProjectIdleStateOptions {
  projectPath: string
  projectName: string
  projectIssues: Pick<Issue, 'status' | 'createdAt' | 'updatedAt'>[]
  fallbackTimestamp: number
  existingIdleSince?: number
}

export const normalizeActionCenterProjectPath = (path: string) => path.replace(/\/+$/, '')

export function pickVisibleActionCenterItems<T extends { projectPath: string }>(
  items: T[],
  limit: number,
): T[] {
  const selected: T[] = []
  const selectedProjectPaths = new Set<string>()

  for (const item of items) {
    const projectKey = normalizeActionCenterProjectPath(item.projectPath)
    if (selectedProjectPaths.has(projectKey)) continue

    selected.push(item)
    selectedProjectPaths.add(projectKey)

    if (selected.length >= limit) break
  }

  return selected
}

export function countActionCenterInProgressIssues(issues: Pick<Issue, 'status'>[]): number {
  return issues.filter(issue => issue.status === 'in_progress').length
}

export function isActionCenterExecutableIssue(issue: Pick<Issue, 'type'>): boolean {
  return issue.type !== 'epic'
}

export function getActionCenterProjectIdleSince(
  projectIssues: Pick<Issue, 'createdAt' | 'updatedAt'>[],
  fallbackTimestamp: number,
): number {
  const latestIssueUpdate = projectIssues.reduce((latest, issue) => {
    const timestamp = Date.parse(issue.updatedAt || issue.createdAt || '')
    return Number.isNaN(timestamp) ? latest : Math.max(latest, timestamp)
  }, 0)

  return latestIssueUpdate > 0 && latestIssueUpdate <= fallbackTimestamp
    ? latestIssueUpdate
    : fallbackTimestamp
}

export function buildActionCenterProjectActionState(
  options: BuildActionCenterProjectActionStateOptions,
): ActionCenterProjectActionState {
  const inProgressCount = countActionCenterInProgressIssues(options.projectIssues)

  return {
    projectPath: options.projectPath,
    projectName: options.projectName,
    inProgressCount,
    cmuxSurfaceId: options.cmuxSurfaceId,
    readyIssues: inProgressCount > 0
      ? []
      : options.readyIssues.filter(isActionCenterExecutableIssue),
  }
}

export function buildActionCenterProjectIdleState(
  options: BuildActionCenterProjectIdleStateOptions,
): ActionCenterProjectIdleState | null {
  if (countActionCenterInProgressIssues(options.projectIssues) > 0) {
    return null
  }

  return {
    projectPath: options.projectPath,
    projectName: options.projectName,
    idleSince: options.existingIdleSince
      ?? getActionCenterProjectIdleSince(options.projectIssues, options.fallbackTimestamp),
  }
}
