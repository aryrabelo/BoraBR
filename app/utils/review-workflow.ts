import type { Issue, UpdateIssuePayload } from '~/types/issue'

export type ReviewWorkflowKind = 'not_review' | 'queued' | 'running' | 'stale' | 'unknown'
export type ReviewProcessKind = 'running' | 'not_running' | 'unknown'

export interface ReviewProcessStatus {
  status: ReviewProcessKind
  pid?: number
}

export interface ReviewWorkflowOptions {
  now?: string | Date
  staleAfterMinutes?: number
  processStatus?: ReviewProcessStatus
}

export interface ReviewWorkflowState {
  kind: ReviewWorkflowKind
  label: string
  tool?: string
  pid?: number
  sessionId?: string
  heartbeatAt?: string
}

interface ReviewMetadata {
  tool?: string
  pid?: number
  sessionId?: string
  startedAt?: string
  heartbeatAt?: string
}

interface ReviewCommentInput {
  at: string
}

export interface ReviewStartedInput extends ReviewCommentInput {
  tool: string
  pid?: number
  sessionId?: string
}

export interface ReviewFailedInput extends ReviewCommentInput {
  reason: string
}

export interface ReviewPassedInput extends ReviewCommentInput {
  summary: string
}

export interface ReviewNeedsAnswerInput extends ReviewCommentInput {
  question: string
}

export interface ReviewWorkflowUpdate {
  payload: UpdateIssuePayload
  comment: string
}

const STALE_AFTER_MINUTES = 30
const REVIEW_CHANGES_REQUESTED = 'review:changes_requested'
const BLOCKED_NEEDS_ANSWER = 'blocked:needs_answer'

function parseJsonObject(value: string | undefined): Record<string, unknown> | null {
  if (!value) return null
  try {
    const parsed = JSON.parse(value)
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as Record<string, unknown> : null
  } catch {
    return null
  }
}

function stringValue(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value : undefined
}

function numberValue(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

function normalizeReviewMetadata(raw: unknown): ReviewMetadata {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {}
  const data = raw as Record<string, unknown>
  return {
    tool: stringValue(data.tool),
    pid: numberValue(data.pid),
    sessionId: stringValue(data.session_id) ?? stringValue(data.sessionId),
    startedAt: stringValue(data.started_at) ?? stringValue(data.startedAt),
    heartbeatAt: stringValue(data.heartbeat_at) ?? stringValue(data.heartbeatAt) ?? stringValue(data.at),
  }
}

function parseMetadata(issue: Issue): ReviewMetadata {
  const parsed = parseJsonObject(issue.metadata)
  if (!parsed) return {}
  return normalizeReviewMetadata(parsed.review ?? parsed)
}

function parseReviewStartedComment(content: string): ReviewMetadata {
  const marker = 'review:started '
  if (!content.startsWith(marker)) return {}
  const parsed = parseJsonObject(content.slice(marker.length))
  return normalizeReviewMetadata(parsed)
}

function parseCommentMetadata(issue: Issue): ReviewMetadata {
  for (const comment of [...(issue.comments ?? [])].reverse()) {
    const parsed = parseReviewStartedComment(comment.content)
    if (parsed.tool || parsed.pid || parsed.sessionId || parsed.startedAt || parsed.heartbeatAt) {
      return parsed
    }
  }
  return {}
}

function getReviewMetadata(issue: Issue): ReviewMetadata {
  return {
    ...parseCommentMetadata(issue),
    ...parseMetadata(issue),
  }
}

function toDate(value: string | Date | undefined): Date | null {
  if (!value) return null
  const date = value instanceof Date ? value : new Date(value)
  return Number.isNaN(date.getTime()) ? null : date
}

function isStale(heartbeat: string | undefined, now: string | Date | undefined, staleAfterMinutes: number): boolean {
  const heartbeatDate = toDate(heartbeat)
  const nowDate = toDate(now) ?? new Date()
  if (!heartbeatDate) return false
  return nowDate.getTime() - heartbeatDate.getTime() > staleAfterMinutes * 60 * 1000
}

function state(kind: ReviewWorkflowKind, overrides: Omit<ReviewWorkflowState, 'kind' | 'label'> = {}): ReviewWorkflowState {
  const labels: Record<ReviewWorkflowKind, string> = {
    not_review: 'Not in review',
    queued: 'Queued',
    running: 'Running',
    stale: 'Stale',
    unknown: 'Unknown',
  }
  return { kind, label: labels[kind], ...overrides }
}

export function getReviewWorkflowState(issue: Issue, options: ReviewWorkflowOptions = {}): ReviewWorkflowState {
  if (issue.status !== 'in_review') return state('not_review')

  const metadata = getReviewMetadata(issue)
  const heartbeatAt = metadata.heartbeatAt ?? metadata.startedAt ?? issue.updatedAt
  const hasClaim = !!issue.assignee || !!metadata.tool || !!metadata.pid || !!metadata.sessionId
  const details = {
    tool: metadata.tool ?? issue.assignee,
    pid: metadata.pid ?? options.processStatus?.pid,
    sessionId: metadata.sessionId,
    heartbeatAt,
  }

  if (!hasClaim) return state('queued', details)

  const staleAfterMinutes = options.staleAfterMinutes ?? STALE_AFTER_MINUTES
  if (isStale(heartbeatAt, options.now, staleAfterMinutes)) return state('stale', details)
  if (options.processStatus?.status === 'not_running') return state('stale', details)
  if (options.processStatus?.status === 'running') return state('running', details)
  if (metadata.pid || metadata.sessionId || metadata.startedAt) return state('running', details)

  return state('unknown', details)
}

function jsonComment(prefix: string, body: Record<string, unknown>): string {
  return `${prefix} ${JSON.stringify(body)}`
}

export function buildReviewStartedComment(input: ReviewStartedInput): string {
  const body: Record<string, unknown> = {
    tool: input.tool,
  }
  if (input.pid !== undefined) body.pid = input.pid
  if (input.sessionId) body.session_id = input.sessionId
  body.started_at = input.at
  return jsonComment('review:started', body)
}

export function buildReviewFailedComment(input: ReviewFailedInput): string {
  return jsonComment('review:failed', {
    reason: input.reason,
    at: input.at,
  })
}

export function buildReviewPassedComment(input: ReviewPassedInput): string {
  return jsonComment('review:passed', {
    summary: input.summary,
    at: input.at,
  })
}

export function buildReviewNeedsAnswerComment(input: ReviewNeedsAnswerInput): string {
  return jsonComment('review:needs_answer', {
    question: input.question,
    at: input.at,
  })
}

function appendLabel(labels: string[], label: string): string[] {
  return labels.some(existing => existing.toLowerCase() === label) ? labels : [...labels, label]
}

export function buildReviewFailureUpdate(issue: Issue, input: ReviewFailedInput): ReviewWorkflowUpdate {
  return {
    payload: {
      status: 'open',
      assignee: '',
      labels: appendLabel(issue.labels ?? [], REVIEW_CHANGES_REQUESTED),
    },
    comment: buildReviewFailedComment(input),
  }
}

export function buildReviewNeedsAnswerUpdate(issue: Issue, input: ReviewNeedsAnswerInput): ReviewWorkflowUpdate {
  return {
    payload: {
      status: 'blocked',
      labels: appendLabel(issue.labels ?? [], BLOCKED_NEEDS_ANSWER),
    },
    comment: buildReviewNeedsAnswerComment(input),
  }
}
