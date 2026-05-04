import type { Comment } from '~/types/issue'

export interface WorkflowHandoffPayload {
  branch?: string
  commit?: string
  files: string[]
  prUrl?: string
  [key: string]: unknown
}

export interface WorkflowHandoffRecord {
  stepId: string
  payload: WorkflowHandoffPayload
  comment: Comment
}

const STEP_HANDOFF_PATTERN = /^step:([a-z0-9._-]+)\s+(\{.*\})\s*$/is

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim().length > 0 ? value.trim() : undefined
}

function asFiles(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  return value
    .filter((item): item is string => typeof item === 'string' && item.trim().length > 0)
    .map(item => item.trim())
}

export function parseWorkflowHandoffComment(comment: Comment): WorkflowHandoffRecord | null {
  const match = comment.content.trim().match(STEP_HANDOFF_PATTERN)
  if (!match) return null

  let parsed: Record<string, unknown>
  try {
    const value = JSON.parse(match[2]!)
    if (!value || typeof value !== 'object' || Array.isArray(value)) return null
    parsed = value as Record<string, unknown>
  } catch {
    return null
  }

  const payload: WorkflowHandoffPayload = {
    ...parsed,
    branch: asString(parsed.branch),
    commit: asString(parsed.commit),
    files: asFiles(parsed.files),
    prUrl: asString(parsed.prUrl) ?? asString(parsed.pr_url),
  }

  return {
    stepId: match[1]!,
    payload,
    comment,
  }
}

export function parseWorkflowHandoffs(comments: Comment[] = []): WorkflowHandoffRecord[] {
  return comments
    .map(parseWorkflowHandoffComment)
    .filter((record): record is WorkflowHandoffRecord => record !== null)
    .sort((a, b) => Date.parse(a.comment.createdAt) - Date.parse(b.comment.createdAt))
}

export function latestWorkflowHandoff(comments: Comment[] = []): WorkflowHandoffRecord | null {
  const handoffs = parseWorkflowHandoffs(comments)
  return handoffs.at(-1) ?? null
}

export function resolveWorkflowBranch(pattern: string, parentId: string): string {
  return pattern.replaceAll('{parent-id}', parentId)
}

export function buildWorkflowHandoffComment(stepId: string, payload: WorkflowHandoffPayload): string {
  return `step:${stepId} ${JSON.stringify({
    ...payload,
    files: payload.files ?? [],
  })}`
}

export function buildWorkflowPullRequestCommand(options: {
  branch: string
  baseBranch?: string
  title: string
  body: string
}): string {
  const base = options.baseBranch ?? 'master'
  return [
    'gh pr create',
    `--base ${shellQuote(base)}`,
    `--head ${shellQuote(options.branch)}`,
    `--title ${shellQuote(options.title)}`,
    `--body ${shellQuote(options.body)}`,
  ].join(' ')
}

function shellQuote(value: string): string {
  return `'${value.replaceAll('\'', '\'\\\'\'')}'`
}
