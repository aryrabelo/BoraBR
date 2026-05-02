import { describe, expect, it } from 'vitest'
import type { Issue } from '~/types/issue'
import {
  hasAutoModeInProgressTask,
  pickAutoModeIssue,
  type AutoModeTask,
} from '~/composables/useAutoMode'

function makeIssue(overrides: Partial<Issue> = {}): Issue {
  return {
    id: 'borabr-unf.1',
    title: 'Test task',
    description: '',
    type: 'task',
    status: 'open',
    priority: 'p2',
    labels: [],
    createdAt: '2026-05-01T12:00:00.000Z',
    updatedAt: '2026-05-01T12:00:00.000Z',
    comments: [],
    ...overrides,
  } as Issue
}

function makeTask(overrides: Partial<AutoModeTask> = {}): AutoModeTask {
  return {
    issueId: 'borabr-unf.1',
    title: 'Test task',
    status: 'running',
    startedAt: Date.parse('2026-05-01T12:00:00.000Z'),
    ...overrides,
  }
}

describe('pickAutoModeIssue', () => {
  it('picks the highest-priority ready task instead of trusting ready list order', () => {
    const issues = [
      makeIssue({ id: 'borabr-unf.2', priority: 'p2' }),
      makeIssue({ id: 'borabr-unf.3', priority: 'p0' }),
      makeIssue({ id: 'borabr-unf.4', priority: 'p1' }),
    ]

    expect(pickAutoModeIssue(issues)?.id).toBe('borabr-unf.3')
  })

  it('does not dispatch epics or tasks that already have auto-mode state', () => {
    const activeTasks = new Map([
      ['borabr-unf.2', makeTask({ issueId: 'borabr-unf.2' })],
    ])
    const issues = [
      makeIssue({ id: 'borabr-unf', type: 'epic', priority: 'p0' }),
      makeIssue({ id: 'borabr-unf.2', priority: 'p0' }),
      makeIssue({ id: 'borabr-unf.3', priority: 'p1' }),
    ]

    expect(pickAutoModeIssue(issues, activeTasks)?.id).toBe('borabr-unf.3')
  })
})

describe('hasAutoModeInProgressTask', () => {
  it('blocks auto dispatch only for in-progress non-epic work', () => {
    expect(hasAutoModeInProgressTask([
      makeIssue({ id: 'borabr-unf', type: 'epic', status: 'in_progress' }),
    ])).toBe(false)

    expect(hasAutoModeInProgressTask([
      makeIssue({ id: 'borabr-unf.1', type: 'task', status: 'in_progress' }),
    ])).toBe(true)
  })
})
