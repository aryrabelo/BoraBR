import { describe, expect, it } from 'vitest'
import type { Issue } from '~/types/issue'
import {
  detectWorkflowContracts,
  parseWorkflowCheckOutput,
  resolveWorkflowContractState,
} from '~/utils/workflow-contracts'

function makeIssue(overrides: Partial<Issue> = {}): Issue {
  return {
    id: 'borabr-m0z.9',
    title: 'Align workflow contracts',
    description: '',
    type: 'feature',
    status: 'open',
    priority: 'p0',
    assignee: '',
    labels: [],
    createdAt: '2026-05-01T10:00:00Z',
    updatedAt: '2026-05-01T10:10:00Z',
    comments: [],
    ...overrides,
  } as Issue
}

describe('workflow contracts', () => {
  it('detects workflow labels from issue labels', () => {
    const contracts = detectWorkflowContracts(makeIssue({
      labels: ['terminal', 'workflow:review', 'workflow:agent-review'],
    }))

    expect(contracts).toEqual([
      { id: 'review', label: 'workflow:review', source: 'issue' },
      { id: 'agent-review', label: 'workflow:agent-review', source: 'issue' },
    ])
  })

  it('uses br workflow output as authoritative state and next commands', () => {
    const parsed = parseWorkflowCheckOutput(JSON.stringify({
      state: 'step_ready',
      workflow_id: 'review',
      next_commands: [
        'br workflow next borabr-m0z.9',
        'br workflow steps borabr-m0z.9 --apply',
      ],
    }))

    const state = resolveWorkflowContractState(makeIssue({
      labels: ['workflow:review'],
    }), { check: parsed })

    expect(state.kind).toBe('step_ready')
    expect(state.workflowId).toBe('review')
    expect(state.nextCommands).toEqual([
      'br workflow next borabr-m0z.9',
      'br workflow steps borabr-m0z.9 --apply',
    ])
    expect(state.inferredPolicy).toBe(false)
  })

  it('shows missing setup guidance without creating workflow steps locally', () => {
    const state = resolveWorkflowContractState(makeIssue({
      labels: ['workflow:review'],
    }))

    expect(state.kind).toBe('uninitialized')
    expect(state.nextCommands).toEqual(['br workflow check borabr-m0z.9'])
    expect(state.inferredPolicy).toBe(false)
  })

  it('marks old in_review state as legacy when no workflow contract is present', () => {
    const state = resolveWorkflowContractState(makeIssue({
      status: 'in_review',
      labels: ['review'],
    }))

    expect(state.kind).toBe('legacy_in_review')
    expect(state.label).toBe('Legacy review')
    expect(state.nextCommands).toEqual(['br workflow check borabr-m0z.9'])
  })
})
