import { describe, it, expect } from 'vitest'
import type { Issue } from '~/types/issue'
import {
  buildActionCenterGitHubPullRequestState,
  buildActionCenterIssuePrompt,
  buildActionCenterLinearIssueState,
  buildActionCenterProjectActionState,
  buildActionCenterProjectIdleState,
  buildActionCenterRunActionItem,
  getActionCenterRunNextActions,
  buildActionCenterReconciledActions,
  countActionCenterInProgressIssues,
  pickVisibleActionCenterItems,
} from '~/utils/action-center'
import type { ActionCenterGitHubPullRequest, ActionCenterLinearIssue } from '~/utils/bd-api'

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

function makePullRequest(overrides: Partial<ActionCenterGitHubPullRequest> = {}): ActionCenterGitHubPullRequest {
  return {
    repoFullName: 'entrc/entrc-backend',
    owner: 'entrc',
    repo: 'entrc-backend',
    number: 42,
    title: 'ENG-123 Add UAT action',
    url: 'https://github.com/entrc/entrc-backend/pull/42',
    state: 'open',
    branch: 'ENG-123-uat-action',
    author: 'aryrabelo',
    isDraft: false,
    reviewState: 'approved',
    comments: 0,
    reviewComments: 0,
    requestedReviewers: 0,
    createdAt: '2026-05-01T10:00:00Z',
    updatedAt: '2026-05-01T12:00:00Z',
    actionTimestamp: Date.parse('2026-05-01T10:00:00Z'),
    ...overrides,
  }
}

function makeLinearIssue(overrides: Partial<ActionCenterLinearIssue> = {}): ActionCenterLinearIssue {
  return {
    identifier: 'ENG-123',
    title: 'Move approved PR to UAT',
    url: 'https://linear.app/canix/issue/ENG-123/move-approved-pr-to-uat',
    status: 'In Progress',
    stateType: 'started',
    isUat: false,
    assignee: 'Ary Rabelo',
    labels: ['backend'],
    pullRequestUrls: [],
    unackedComments: 0,
    updatedAt: '2026-05-01T12:00:00Z',
    actionTimestamp: Date.parse('2026-05-01T12:00:00Z'),
    ...overrides,
  }
}

describe('Action Center project state', () => {
  it('maps durable auto-mode phases to deterministic next actions', () => {
    expect(getActionCenterRunNextActions({ phase: 'executor_complete' })).toEqual(['dispatch_review', 'continue', 'cancel'])
    expect(getActionCenterRunNextActions({ phase: 'review_approved' })).toEqual(['create_pr', 'continue', 'cancel'])
    expect(getActionCenterRunNextActions({ phase: 'failed' })).toEqual(['retry', 'cancel'])
    expect(getActionCenterRunNextActions({ phase: 'done' })).toEqual(['cleanup'])
    expect(getActionCenterRunNextActions({ phase: 'cancelled' })).toEqual(['cleanup'])
  })

  it('builds auto-mode Action Center items from durable run state', () => {
    const item = buildActionCenterRunActionItem({
      projectPath: '/Users/aryrabelo/Sites/entrc-backend',
      projectName: 'entrc-backend',
      baseBranch: 'main',
      issueId: 'ENG-559',
      issueTitle: 'Ship deterministic auto-mode',
      epicId: 'ENG',
      provider: 'wt',
      branch: 'ary/ENG-559-auto-mode',
      worktreePath: '/Users/aryrabelo/Sites/entrc-backend/.worktrees/ENG-559',
      phase: 'executing',
      lastEvent: 'Executor running',
      attempts: 1,
      updatedAt: '2026-05-04T12:00:00.000Z',
    })

    expect(item).toMatchObject({
      actionKind: 'auto_mode_run',
      actionSource: 'auto_mode',
      actionSourceLabel: 'Auto-Mode',
      id: 'ENG-559',
      title: 'ENG-559: Ship deterministic auto-mode',
      projectPath: '/Users/aryrabelo/Sites/entrc-backend',
      provider: 'wt',
      branch: 'ary/ENG-559-auto-mode',
      worktreePath: '/Users/aryrabelo/Sites/entrc-backend/.worktrees/ENG-559',
      phase: 'executing',
      lastEvent: 'Executor running',
      nextActions: ['continue', 'cancel'],
    })
    expect(item.description).toContain('Project root: /Users/aryrabelo/Sites/entrc-backend')
    expect(item.description).toContain('Base branch: main')
    expect(item.description).toContain('Provider: wt')
    expect(item.description).toContain('Worktree: /Users/aryrabelo/Sites/entrc-backend/.worktrees/ENG-559')
  })

  it('builds the prompt used by Action Center issue actions', () => {
    expect(buildActionCenterIssuePrompt(makeIssue({ id: 'borabr-vox.3' }))).toBe(
      'Continuar a tarefa borabr-vox.3 usando a skill BR',
    )
  })

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

  it('normalizes open GitHub PRs for Action Center ordering', () => {
    const state = buildActionCenterGitHubPullRequestState({
      projectPath: '/Users/aryrabelo/Sites/entrc-backend',
      projectName: 'entrc-backend',
      response: {
        projectPath: '/Users/aryrabelo/Sites/entrc-backend',
        repoFullName: 'entrc/entrc-backend',
        error: null,
        pullRequests: [
          {
            repoFullName: 'entrc/entrc-backend',
            owner: 'entrc',
            repo: 'entrc-backend',
            number: 42,
            title: 'ENG-123 Add UAT action',
            url: 'https://github.com/entrc/entrc-backend/pull/42',
            state: 'open',
            branch: 'ENG-123-uat-action',
            author: 'aryrabelo',
            isDraft: false,
            reviewState: 'approved',
            comments: 2,
            reviewComments: 1,
            requestedReviewers: 0,
            createdAt: '2026-05-01T10:00:00Z',
            updatedAt: '2026-05-01T12:00:00Z',
            actionTimestamp: Date.parse('2026-05-01T10:00:00Z'),
          },
          {
            repoFullName: 'entrc/entrc-backend',
            owner: 'entrc',
            repo: 'entrc-backend',
            number: 41,
            title: 'Closed PR',
            url: 'https://github.com/entrc/entrc-backend/pull/41',
            state: 'closed',
            branch: 'ENG-122-closed',
            author: 'aryrabelo',
            isDraft: false,
            reviewState: 'pending_review',
            comments: 0,
            reviewComments: 0,
            requestedReviewers: 0,
            createdAt: '2026-05-01T09:00:00Z',
            updatedAt: '2026-05-01T09:30:00Z',
            actionTimestamp: Date.parse('2026-05-01T09:00:00Z'),
          },
        ],
      },
    })

    expect(state.error).toBeNull()
    expect(state.pullRequests).toHaveLength(1)
    const firstPullRequest = state.pullRequests[0]!
    expect(firstPullRequest).toMatchObject({
      repoFullName: 'entrc/entrc-backend',
      number: 42,
      branch: 'ENG-123-uat-action',
      author: 'aryrabelo',
      reviewState: 'approved',
    })
    expect(firstPullRequest.actionTimestamp).toBe(Date.parse('2026-05-01T10:00:00Z'))
  })

  it('keeps GitHub API errors as recoverable Action Center state', () => {
    const state = buildActionCenterGitHubPullRequestState({
      projectPath: '/repo',
      projectName: 'Repo',
      response: {
        projectPath: '/repo',
        repoFullName: 'acme/repo',
        error: 'GitHub PR request returned status: 401 Unauthorized',
        pullRequests: [],
      },
    })

    expect(state.error).toBe('GitHub PR request returned status: 401 Unauthorized')
    expect(state.pullRequests).toEqual([])
  })

  it('normalizes Linear issues with status and PR links', () => {
    const state = buildActionCenterLinearIssueState({
      response: {
        teamKey: 'ENG',
        assignee: 'Ary Rabelo',
        error: null,
        issues: [
          {
            identifier: 'ENG-123',
            title: 'Move approved PR to UAT',
            url: 'https://linear.app/canix/issue/ENG-123/move-approved-pr-to-uat',
            status: 'UAT',
            stateType: 'started',
            isUat: true,
            assignee: 'Ary Rabelo',
            labels: ['backend'],
            pullRequestUrls: ['https://github.com/entrc/entrc-backend/pull/42'],
            unackedComments: 1,
            updatedAt: '2026-05-01T12:00:00Z',
            actionTimestamp: Date.parse('2026-05-01T12:00:00Z'),
          },
          {
            identifier: 'ENG-124',
            title: 'Needs implementation',
            url: 'https://linear.app/canix/issue/ENG-124/needs-implementation',
            status: 'In Progress',
            stateType: 'started',
            isUat: false,
            assignee: 'Ary Rabelo',
            labels: [],
            pullRequestUrls: [],
            unackedComments: 0,
            updatedAt: '2026-05-01T11:00:00Z',
            actionTimestamp: Date.parse('2026-05-01T11:00:00Z'),
          },
        ],
      },
    })

    expect(state.error).toBeNull()
    expect(state.issues.map(issue => issue.identifier)).toEqual(['ENG-124', 'ENG-123'])
    expect(state.issues[1]!.isUat).toBe(true)
    expect(state.issues[1]!.pullRequestUrls).toEqual(['https://github.com/entrc/entrc-backend/pull/42'])
  })

  it('keeps Linear API errors as recoverable Action Center state', () => {
    const state = buildActionCenterLinearIssueState({
      response: {
        teamKey: 'ENG',
        assignee: null,
        error: 'Linear API key not configured',
        issues: [],
      },
    })

    expect(state.error).toBe('Linear API key not configured')
    expect(state.issues).toEqual([])
  })

  it('reconciles an approved GitHub PR with a non-UAT Linear ticket', () => {
    const actions = buildActionCenterReconciledActions({
      githubPullRequestStates: [{
        projectPath: '/Users/aryrabelo/Sites/entrc-backend',
        projectName: 'entrc-backend',
        repoFullName: 'entrc/entrc-backend',
        error: null,
        pullRequests: [makePullRequest()],
      }],
      linearIssueState: {
        teamKey: 'ENG',
        assignee: 'Ary Rabelo',
        error: null,
        issues: [makeLinearIssue()],
      },
    })

    expect(actions).toHaveLength(1)
    expect(actions[0]).toMatchObject({
      nextActionKind: 'move_linear_to_uat',
      primaryTarget: 'linear',
      title: 'ENG-123: mover Linear para UAT',
      originSources: ['github', 'linear'],
    })
  })

  it('creates follow-up actions for Linear tickets without matching PRs', () => {
    const actions = buildActionCenterReconciledActions({
      githubPullRequestStates: [],
      linearIssueState: {
        teamKey: 'ENG',
        assignee: 'Ary Rabelo',
        error: null,
        issues: [makeLinearIssue({ identifier: 'ENG-124', pullRequestUrls: [] })],
      },
    })

    expect(actions).toHaveLength(1)
    expect(actions[0]).toMatchObject({
      nextActionKind: 'linear_missing_pr',
      primaryTarget: 'linear',
      title: 'ENG-124: criar ou vincular PR',
      originSources: ['linear'],
    })
  })

  it('uses Linear PR URLs to reconcile GitHub actions back to tickets', () => {
    const pullRequest = makePullRequest({
      number: 77,
      title: 'No ticket id in title',
      branch: 'feature/no-ticket-id',
      url: 'https://github.com/entrc/entrc-backend/pull/77',
      reviewState: 'changes_requested',
      comments: 1,
      actionTimestamp: Date.parse('2026-05-01T09:00:00Z'),
    })
    const actions = buildActionCenterReconciledActions({
      githubPullRequestStates: [{
        projectPath: '/Users/aryrabelo/Sites/entrc-backend',
        projectName: 'entrc-backend',
        repoFullName: 'entrc/entrc-backend',
        error: null,
        pullRequests: [pullRequest],
      }],
      linearIssueState: {
        teamKey: 'ENG',
        assignee: 'Ary Rabelo',
        error: null,
        issues: [makeLinearIssue({
          identifier: 'ENG-125',
          pullRequestUrls: ['https://github.com/entrc/entrc-backend/pull/77/'],
        })],
      },
    })

    expect(actions).toHaveLength(1)
    expect(actions[0]).toMatchObject({
      nextActionKind: 'github_pr_attention',
      primaryTarget: 'github',
      title: '#77: tratar pendência do PR',
      originSources: ['github', 'linear'],
    })
  })

  it('keeps GitHub PRs without Linear matches as link actions', () => {
    const actions = buildActionCenterReconciledActions({
      githubPullRequestStates: [{
        projectPath: '/Users/aryrabelo/Sites/entrc-backend',
        projectName: 'entrc-backend',
        repoFullName: 'entrc/entrc-backend',
        error: null,
        pullRequests: [makePullRequest({ branch: 'no-ticket-id', title: 'Small fix' })],
      }],
      linearIssueState: {
        teamKey: 'ENG',
        assignee: 'Ary Rabelo',
        error: null,
        issues: [],
      },
    })

    expect(actions).toHaveLength(1)
    expect(actions[0]).toMatchObject({
      nextActionKind: 'github_pr_unlinked',
      primaryTarget: 'github',
      title: '#42: vincular PR ao Linear',
      originSources: ['github'],
    })
  })
})
