import { describe, expect, it } from 'vitest'
import type { Issue } from '~/types/issue'
import {
  buildReviewFailedComment,
  buildReviewFailureUpdate,
  buildReviewNeedsAnswerUpdate,
  buildReviewPassedComment,
  buildReviewStartedComment,
  getReviewWorkflowState,
} from '~/utils/review-workflow'

function makeIssue(overrides: Partial<Issue> = {}): Issue {
  return {
    id: 'borabr-m0z.7',
    title: 'Support review workflow',
    description: '',
    type: 'task',
    status: 'in_review',
    priority: 'p1',
    assignee: '',
    labels: [],
    createdAt: '2026-05-01T10:00:00Z',
    updatedAt: '2026-05-01T10:05:00Z',
    comments: [],
    ...overrides,
  } as Issue
}

describe('review workflow state', () => {
  it('treats in_review without assignee or session metadata as queued', () => {
    const state = getReviewWorkflowState(makeIssue(), {
      now: '2026-05-01T10:10:00Z',
      staleAfterMinutes: 30,
    })

    expect(state.kind).toBe('queued')
    expect(state.label).toBe('Queued')
  })

  it('shows a running review when process evidence is alive and heartbeat is fresh', () => {
    const issue = makeIssue({
      assignee: 'codex',
      metadata: JSON.stringify({
        review: {
          tool: 'codex',
          pid: 4242,
          session_id: 'sess-1',
          started_at: '2026-05-01T10:00:00Z',
        },
      }),
      updatedAt: '2026-05-01T10:06:00Z',
    })

    const state = getReviewWorkflowState(issue, {
      now: '2026-05-01T10:10:00Z',
      staleAfterMinutes: 30,
      processStatus: { status: 'running', pid: 4242 },
    })

    expect(state.kind).toBe('running')
    expect(state.tool).toBe('codex')
    expect(state.pid).toBe(4242)
  })

  it('marks claimed reviews stale when the heartbeat is too old even with process metadata', () => {
    const issue = makeIssue({
      assignee: 'claude',
      metadata: JSON.stringify({
        review: {
          tool: 'claude',
          pid: 5151,
          session_id: 'sess-old',
          started_at: '2026-05-01T09:00:00Z',
        },
      }),
      updatedAt: '2026-05-01T09:05:00Z',
    })

    const state = getReviewWorkflowState(issue, {
      now: '2026-05-01T10:10:00Z',
      staleAfterMinutes: 30,
      processStatus: { status: 'running', pid: 5151 },
    })

    expect(state.kind).toBe('stale')
  })

  it('shows unknown when review is claimed but process state cannot be trusted', () => {
    const state = getReviewWorkflowState(makeIssue({ assignee: 'codex' }), {
      now: '2026-05-01T10:10:00Z',
      staleAfterMinutes: 30,
      processStatus: { status: 'unknown' },
    })

    expect(state.kind).toBe('unknown')
  })
})

describe('review workflow comments and transitions', () => {
  it('builds structured comments for review lifecycle events', () => {
    expect(buildReviewStartedComment({
      tool: 'codex',
      pid: 4242,
      sessionId: 'sess-1',
      at: '2026-05-01T10:00:00Z',
    })).toBe('review:started {"tool":"codex","pid":4242,"session_id":"sess-1","started_at":"2026-05-01T10:00:00Z"}')

    expect(buildReviewFailedComment({
      reason: 'Missing regression test',
      at: '2026-05-01T10:30:00Z',
    })).toBe('review:failed {"reason":"Missing regression test","at":"2026-05-01T10:30:00Z"}')

    expect(buildReviewPassedComment({
      summary: 'No actionable findings',
      at: '2026-05-01T10:40:00Z',
    })).toBe('review:passed {"summary":"No actionable findings","at":"2026-05-01T10:40:00Z"}')
  })

  it('returns review failure update data that reopens and labels the issue', () => {
    const update = buildReviewFailureUpdate(makeIssue({
      assignee: 'codex',
      labels: ['terminal'],
    }), {
      reason: 'Missing regression test',
      at: '2026-05-01T10:30:00Z',
    })

    expect(update.payload.status).toBe('open')
    expect(update.payload.assignee).toBe('')
    expect(update.payload.labels).toEqual(['terminal', 'review:changes_requested'])
    expect(update.comment).toContain('review:failed')
  })

  it('returns needs-answer update data that blocks with a concrete question', () => {
    const update = buildReviewNeedsAnswerUpdate(makeIssue({ labels: ['terminal'] }), {
      question: 'Which agent should own the review?',
      at: '2026-05-01T10:35:00Z',
    })

    expect(update.payload.status).toBe('blocked')
    expect(update.payload.labels).toEqual(['terminal', 'blocked:needs_answer'])
    expect(update.comment).toBe('review:needs_answer {"question":"Which agent should own the review?","at":"2026-05-01T10:35:00Z"}')
  })
})
