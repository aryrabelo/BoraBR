import { describe, it, expect } from 'vitest'
import type { Issue } from '~/types/issue'
import {
  buildActionCenterProjectActionState,
  buildActionCenterProjectIdleState,
  countActionCenterInProgressIssues,
  pickVisibleActionCenterItems,
} from '~/utils/action-center'

function makeIssue(overrides: Partial<Issue> = {}): Issue {
  return {
    id: 'issue-1',
    title: 'Test issue',
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

describe('Action Center project state', () => {
  it('counts in-progress issues for a saved project snapshot', () => {
    const issues = [
      makeIssue({ id: 'active', status: 'in_progress' }),
      makeIssue({ id: 'queued', status: 'open' }),
    ]

    expect(countActionCenterInProgressIssues(issues)).toBe(1)
  })

  it('suppresses ready issue actions while the same project has in-progress work', () => {
    const readyIssue = makeIssue({ id: 'ready' })
    const state = buildActionCenterProjectActionState({
      projectPath: '/repo',
      projectName: 'Repo',
      projectIssues: [makeIssue({ id: 'active', status: 'in_progress' })],
      readyIssues: [readyIssue],
    })

    expect(state.inProgressCount).toBe(1)
    expect(state.readyIssues).toEqual([])
  })

  it('keeps ready issue actions when the project has no in-progress work', () => {
    const readyIssue = makeIssue({ id: 'ready' })
    const state = buildActionCenterProjectActionState({
      projectPath: '/repo',
      projectName: 'Repo',
      projectIssues: [makeIssue({ id: 'open', status: 'open' })],
      readyIssues: [readyIssue],
    })

    expect(state.inProgressCount).toBe(0)
    expect(state.readyIssues).toEqual([readyIssue])
  })

  it('does not expose epics as executable Action Center items', () => {
    const epic = makeIssue({ id: 'beads_rust-hhw1', type: 'epic', children: [] })
    const childTask = makeIssue({
      id: 'beads_rust-child1',
      type: 'task',
      parent: {
        id: 'beads_rust-hhw1',
        title: 'Parent epic',
        status: 'open',
        priority: 'p0',
      },
    })

    const state = buildActionCenterProjectActionState({
      projectPath: '/repo',
      projectName: 'Repo',
      projectIssues: [epic, childTask],
      readyIssues: [epic, childTask],
    })

    expect(state.readyIssues).toEqual([childTask])
  })

  it('shows at most three visible actions and only one per project', () => {
    const items = [
      { id: 'nuran-1', projectPath: '/repos/nuran' },
      { id: 'nuran-2', projectPath: '/repos/nuran/' },
      { id: 'br-1', projectPath: '/repos/br' },
      { id: 'cmux-1', projectPath: '/repos/cmux' },
      { id: 'extra-1', projectPath: '/repos/extra' },
    ]

    expect(pickVisibleActionCenterItems(items, 3).map(item => item.id)).toEqual([
      'nuran-1',
      'br-1',
      'cmux-1',
    ])
  })

  it('does not create an idle notification while the project has in-progress work', () => {
    const state = buildActionCenterProjectIdleState({
      projectPath: '/repo',
      projectName: 'Repo',
      projectIssues: [makeIssue({ id: 'active', status: 'in_progress' })],
      fallbackTimestamp: Date.parse('2026-05-01T12:10:00.000Z'),
    })

    expect(state).toBeNull()
  })
})
