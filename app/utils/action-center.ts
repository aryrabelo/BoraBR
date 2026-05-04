import type { Issue } from '~/types/issue'
import type {
  ActionCenterGitHubPullRequest,
  ActionCenterGitHubPullRequestResponse,
  ActionCenterLinearIssue,
  ActionCenterLinearIssueResponse,
} from '~/utils/bd-api'

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

export interface ActionCenterGitHubPullRequestState {
  projectPath: string
  projectName: string
  repoFullName: string | null
  error: string | null
  pullRequests: ActionCenterGitHubPullRequest[]
}

export interface ActionCenterLinearIssueState {
  teamKey: string
  assignee: string | null
  error: string | null
  issues: ActionCenterLinearIssue[]
}

export type ActionCenterReconciledActionKind =
  | 'move_linear_to_uat'
  | 'linear_missing_pr'
  | 'github_pr_attention'
  | 'linear_comments_pending'
  | 'github_pr_unlinked'

export type ActionCenterReconciledActionTarget = 'github' | 'linear'

export type ActionCenterAutoModeRunPhase =
  | 'workspace_ready'
  | 'executing'
  | 'executor_complete'
  | 'reviewing'
  | 'review_approved'
  | 'review_changes_requested'
  | 'creating_pr'
  | 'done'
  | 'failed'
  | 'cancelled'

export type ActionCenterAutoModeProvider = 'gitWorktree' | 'wt'

export type ActionCenterAutoModeRunNextAction =
  | 'continue'
  | 'open_worktree'
  | 'dispatch_review'
  | 'create_pr'
  | 'retry'
  | 'cancel'
  | 'cleanup'

export interface ActionCenterAutoModeRunState {
  runId?: string | null
  projectPath: string
  projectName?: string | null
  baseBranch?: string | null
  issueId: string
  issueTitle: string
  epicId?: string | null
  epicTitle?: string | null
  provider: ActionCenterAutoModeProvider
  branch: string
  worktreePath: string
  phase: ActionCenterAutoModeRunPhase
  lastEvent?: string | null
  error?: string | null
  attempts: number
  updatedAt?: string | null
}

export interface ActionCenterAutoModeRunItem extends ActionCenterAutoModeRunState {
  actionKind: 'auto_mode_run'
  actionId: string
  id: string
  title: string
  description: string
  projectPath: string
  projectName: string
  actionSource: 'auto_mode'
  actionSourceLabel: string
  actionTimestamp: number
  nextActions: ActionCenterAutoModeRunNextAction[]
}

export interface ActionCenterReconciledAction {
  actionKey: string
  nextActionKind: ActionCenterReconciledActionKind
  title: string
  description: string
  originSources: ActionCenterReconciledActionTarget[]
  primaryTarget: ActionCenterReconciledActionTarget
  primaryUrl: string
  githubPullRequest?: ActionCenterGitHubPullRequest
  linearIssue?: ActionCenterLinearIssue
  actionTimestamp: number
}

export function getActionCenterRunNextActions(
  run: Pick<ActionCenterAutoModeRunState, 'phase'>,
): ActionCenterAutoModeRunNextAction[] {
  switch (run.phase) {
    case 'workspace_ready':
    case 'executing':
    case 'reviewing':
    case 'review_changes_requested':
    case 'creating_pr':
      return ['continue', 'cancel']
    case 'executor_complete':
      return ['dispatch_review', 'continue', 'cancel']
    case 'review_approved':
      return ['create_pr', 'continue', 'cancel']
    case 'failed':
      return ['retry', 'cancel']
    case 'done':
    case 'cancelled':
      return ['cleanup']
  }
}

export function getActionCenterRunPhasePriority(
  run: Pick<ActionCenterAutoModeRunState, 'phase'>,
): number {
  switch (run.phase) {
    case 'failed':
      return 0
    case 'executor_complete':
    case 'review_approved':
    case 'review_changes_requested':
      return 1
    case 'workspace_ready':
    case 'executing':
    case 'reviewing':
    case 'creating_pr':
      return 2
    case 'done':
    case 'cancelled':
      return 3
  }
}

export function compareActionCenterRunItems(
  a: Pick<ActionCenterAutoModeRunItem, 'phase' | 'actionTimestamp' | 'id'>,
  b: Pick<ActionCenterAutoModeRunItem, 'phase' | 'actionTimestamp' | 'id'>,
): number {
  const priorityDelta = getActionCenterRunPhasePriority(a) - getActionCenterRunPhasePriority(b)
  if (priorityDelta !== 0) return priorityDelta

  if (a.actionTimestamp !== b.actionTimestamp) {
    return b.actionTimestamp - a.actionTimestamp
  }

  return a.id.localeCompare(b.id)
}

function parseActionCenterRunTimestamp(value?: string | null): number {
  const timestamp = Date.parse(value || '')
  if (!Number.isNaN(timestamp)) return timestamp

  const numericTimestamp = Number(value)
  if (Number.isFinite(numericTimestamp)) return numericTimestamp

  return Date.now()
}

export function buildActionCenterRunActionItem(
  run: ActionCenterAutoModeRunState,
): ActionCenterAutoModeRunItem {
  const actionTimestamp = parseActionCenterRunTimestamp(run.updatedAt)
  const projectName = run.projectName || run.projectPath.split('/').filter(Boolean).at(-1) || 'Project'
  const actionRevision = run.runId || run.updatedAt || run.phase
  const descriptionParts = [
    `Project root: ${run.projectPath}`,
    run.baseBranch ? `Base branch: ${run.baseBranch}` : '',
    run.epicId ? `Epic: ${run.epicId}${run.epicTitle ? ` - ${run.epicTitle}` : ''}` : '',
    run.runId ? `Run: ${run.runId}` : '',
    `Provider: ${run.provider}`,
    `Branch: ${run.branch}`,
    `Worktree: ${run.worktreePath}`,
    `Phase: ${run.phase}`,
    run.lastEvent ? `Last event: ${run.lastEvent}` : '',
    run.error ? `Error: ${run.error}` : '',
  ].filter(Boolean)

  return {
    ...run,
    actionKind: 'auto_mode_run',
    actionId: `auto-mode:${normalizeActionCenterProjectPath(run.projectPath)}:${run.issueId}:${actionRevision}:${run.phase}`,
    id: run.issueId,
    title: `${run.issueId}: ${run.issueTitle}`,
    description: descriptionParts.join(' | '),
    projectName,
    actionSource: 'auto_mode',
    actionSourceLabel: 'Auto-Mode',
    actionTimestamp,
    nextActions: getActionCenterRunNextActions(run),
  }
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

interface BuildActionCenterGitHubPullRequestStateOptions {
  projectPath: string
  projectName: string
  response: ActionCenterGitHubPullRequestResponse
}

interface BuildActionCenterLinearIssueStateOptions {
  response: ActionCenterLinearIssueResponse
}

interface BuildActionCenterReconciledActionsOptions {
  githubPullRequestStates: ActionCenterGitHubPullRequestState[]
  linearIssueState?: ActionCenterLinearIssueState | null
}

export const normalizeActionCenterProjectPath = (path: string) => path.replace(/\/+$/, '')

export function buildActionCenterIssuePrompt(issue: Pick<Issue, 'id'>): string {
  return `Continuar a tarefa ${issue.id} usando a skill BR`
}

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

export function buildActionCenterGitHubPullRequestState(
  options: BuildActionCenterGitHubPullRequestStateOptions,
): ActionCenterGitHubPullRequestState {
  const repoFullName = options.response.repoFullName ?? null

  return {
    projectPath: options.projectPath,
    projectName: options.projectName,
    repoFullName,
    error: options.response.error ?? null,
    pullRequests: options.response.pullRequests
      .filter(pr => pr.state === 'open')
      .map(pr => ({
        ...pr,
        repoFullName: pr.repoFullName || repoFullName || '',
      }))
      .sort((a, b) => a.actionTimestamp - b.actionTimestamp || a.number - b.number),
  }
}

export function buildActionCenterLinearIssueState(
  options: BuildActionCenterLinearIssueStateOptions,
): ActionCenterLinearIssueState {
  return {
    teamKey: options.response.teamKey,
    assignee: options.response.assignee ?? null,
    error: options.response.error ?? null,
    issues: options.response.issues
      .sort((a, b) => a.actionTimestamp - b.actionTimestamp || a.identifier.localeCompare(b.identifier)),
  }
}

function normalizeActionCenterPullRequestUrl(url: string): string {
  return url.trim().replace(/\/+$/, '').toLowerCase()
}

function extractActionCenterTicketIdentifiers(...values: Array<string | null | undefined>): string[] {
  const identifiers = new Set<string>()
  for (const value of values) {
    const matches = value?.match(/\b[A-Z]+-\d+\b/gi) ?? []
    for (const match of matches) {
      identifiers.add(match.toUpperCase())
    }
  }
  return Array.from(identifiers)
}

function formatActionCenterReviewState(state: string): string {
  const labels: Record<string, string> = {
    approved: 'PR aprovado',
    changes_requested: 'mudanças pedidas no PR',
    review_requested: 'review solicitado',
    commented: 'PR com comentários',
    draft: 'PR em draft',
    pending_review: 'PR aguardando review',
  }
  return labels[state] ?? state
}

function describeActionCenterLink(
  pr?: ActionCenterGitHubPullRequest,
  issue?: ActionCenterLinearIssue,
): string {
  return [
    pr ? `GitHub ${pr.repoFullName} #${pr.number}` : '',
    issue ? `Linear ${issue.identifier} (${issue.status || 'sem status'})` : '',
  ].filter(Boolean).join(' • ')
}

function buildMatchedActionTimestamp(
  pr?: ActionCenterGitHubPullRequest,
  issue?: ActionCenterLinearIssue,
): number {
  return Math.min(
    pr?.actionTimestamp ?? Number.MAX_SAFE_INTEGER,
    issue?.actionTimestamp ?? Number.MAX_SAFE_INTEGER,
  )
}

export function buildActionCenterReconciledActions(
  options: BuildActionCenterReconciledActionsOptions,
): ActionCenterReconciledAction[] {
  const pullRequests = options.githubPullRequestStates.flatMap(state => state.pullRequests)
  const linearIssues = options.linearIssueState?.issues ?? []
  const pullRequestsByUrl = new Map<string, ActionCenterGitHubPullRequest>()
  const pullRequestsByIdentifier = new Map<string, ActionCenterGitHubPullRequest[]>()
  const matchedPullRequestUrls = new Set<string>()
  const actions: ActionCenterReconciledAction[] = []

  for (const pr of pullRequests) {
    pullRequestsByUrl.set(normalizeActionCenterPullRequestUrl(pr.url), pr)
    for (const identifier of extractActionCenterTicketIdentifiers(pr.branch, pr.title)) {
      const matches = pullRequestsByIdentifier.get(identifier) ?? []
      matches.push(pr)
      pullRequestsByIdentifier.set(identifier, matches)
    }
  }

  const findPullRequestForIssue = (issue: ActionCenterLinearIssue) => {
    for (const url of issue.pullRequestUrls) {
      const pr = pullRequestsByUrl.get(normalizeActionCenterPullRequestUrl(url))
      if (pr) return pr
    }
    return pullRequestsByIdentifier.get(issue.identifier.toUpperCase())?.[0]
  }

  for (const issue of linearIssues) {
    const pr = findPullRequestForIssue(issue)
    if (pr) {
      matchedPullRequestUrls.add(normalizeActionCenterPullRequestUrl(pr.url))
    }

    if (!pr) {
      actions.push({
        actionKey: `linear-missing-pr:${issue.identifier}:${issue.updatedAt || issue.actionTimestamp}`,
        nextActionKind: 'linear_missing_pr',
        title: `${issue.identifier}: criar ou vincular PR`,
        description: [
          describeActionCenterLink(undefined, issue),
          issue.assignee ? `assignee ${issue.assignee}` : '',
        ].filter(Boolean).join(' • '),
        originSources: ['linear'],
        primaryTarget: 'linear',
        primaryUrl: issue.url,
        linearIssue: issue,
        actionTimestamp: issue.actionTimestamp,
      })
      continue
    }

    const linkedDescription = describeActionCenterLink(pr, issue)
    if (pr.reviewState === 'approved' && !issue.isUat) {
      actions.push({
        actionKey: `move-linear-to-uat:${issue.identifier}:${pr.repoFullName}:${pr.number}:${issue.updatedAt || issue.actionTimestamp}`,
        nextActionKind: 'move_linear_to_uat',
        title: `${issue.identifier}: mover Linear para UAT`,
        description: `${linkedDescription} • ${formatActionCenterReviewState(pr.reviewState)}`,
        originSources: ['github', 'linear'],
        primaryTarget: 'linear',
        primaryUrl: issue.url,
        githubPullRequest: pr,
        linearIssue: issue,
        actionTimestamp: buildMatchedActionTimestamp(pr, issue),
      })
      continue
    }

    if (
      pr.reviewState === 'changes_requested'
      || pr.reviewState === 'commented'
      || pr.reviewState === 'review_requested'
      || pr.comments > 0
      || pr.reviewComments > 0
      || pr.requestedReviewers > 0
    ) {
      actions.push({
        actionKey: `github-pr-attention:${pr.repoFullName}:${pr.number}:${pr.updatedAt || pr.actionTimestamp}`,
        nextActionKind: 'github_pr_attention',
        title: `#${pr.number}: tratar pendência do PR`,
        description: `${linkedDescription} • ${formatActionCenterReviewState(pr.reviewState)}`,
        originSources: ['github', 'linear'],
        primaryTarget: 'github',
        primaryUrl: pr.url,
        githubPullRequest: pr,
        linearIssue: issue,
        actionTimestamp: buildMatchedActionTimestamp(pr, issue),
      })
      continue
    }

    if (issue.unackedComments > 0) {
      actions.push({
        actionKey: `linear-comments-pending:${issue.identifier}:${issue.updatedAt || issue.actionTimestamp}`,
        nextActionKind: 'linear_comments_pending',
        title: `${issue.identifier}: responder comentários no Linear`,
        description: `${linkedDescription} • ${issue.unackedComments} comentário(s) pendente(s)`,
        originSources: ['github', 'linear'],
        primaryTarget: 'linear',
        primaryUrl: issue.url,
        githubPullRequest: pr,
        linearIssue: issue,
        actionTimestamp: buildMatchedActionTimestamp(pr, issue),
      })
    }
  }

  for (const pr of pullRequests) {
    if (matchedPullRequestUrls.has(normalizeActionCenterPullRequestUrl(pr.url))) {
      continue
    }

    actions.push({
      actionKey: `github-pr-unlinked:${pr.repoFullName}:${pr.number}:${pr.updatedAt || pr.actionTimestamp}`,
      nextActionKind: 'github_pr_unlinked',
      title: `#${pr.number}: vincular PR ao Linear`,
      description: `${describeActionCenterLink(pr)} • ${formatActionCenterReviewState(pr.reviewState)}`,
      originSources: ['github'],
      primaryTarget: 'github',
      primaryUrl: pr.url,
      githubPullRequest: pr,
      actionTimestamp: pr.actionTimestamp,
    })
  }

  return actions.sort((a, b) =>
    a.actionTimestamp - b.actionTimestamp || a.actionKey.localeCompare(b.actionKey),
  )
}
