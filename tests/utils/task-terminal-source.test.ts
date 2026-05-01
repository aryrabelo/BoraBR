import { describe, expect, it, vi } from 'vitest'
import {
  buildTaskTerminalAssigneeDisplay,
  focusTaskTerminalSource,
  resolveTaskTerminalSource,
} from '~/utils/task-terminal-source'
import type { Issue } from '~/types/issue'

const surfaceId = '7DCCBE94-C09F-4E40-80D6-23FAEFD7D116'

function makeIssue(input: Partial<Issue> = {}): Issue {
  return {
    id: 'borabr-m0z.11',
    title: 'External cmux task terminal',
    description: '',
    type: 'feature',
    status: 'in_progress',
    priority: 'p1',
    assignee: '',
    labels: [],
    comments: [],
    createdAt: '2026-05-01T00:00:00Z',
    updatedAt: '2026-05-01T00:00:00Z',
    ...input,
  }
}

describe('task terminal source', () => {
  it('detects external cmux surface ids from the assignee field', () => {
    expect(resolveTaskTerminalSource(makeIssue({ assignee: `cmux:${surfaceId}` }))).toEqual({
      origin: 'external-cmux',
      surfaceId,
      label: 'cmux:7DCCBE94',
      command: ['cmux', 'focus-surface', '--surface', surfaceId],
    })
  })

  it('accepts brace-wrapped cmux assignee ids from quick task annotations', () => {
    expect(resolveTaskTerminalSource(makeIssue({ assignee: `cmux:{${surfaceId}}` }))).toMatchObject({
      origin: 'external-cmux',
      surfaceId,
    })
  })

  it('treats malformed cmux assignee values as unknown terminal source', () => {
    expect(resolveTaskTerminalSource(makeIssue({ assignee: 'cmux:' }))).toEqual({
      origin: 'unknown',
      label: 'cmux',
    })
  })

  it('uses embedded terminal fallback when no external terminal source is present', () => {
    expect(resolveTaskTerminalSource(makeIssue({ assignee: 'assistant' }))).toEqual({
      origin: 'embedded',
      label: 'embedded',
    })
  })

  it('builds assignee-column display without overwriting durable assignee semantics', () => {
    expect(buildTaskTerminalAssigneeDisplay(makeIssue({ assignee: `cmux:${surfaceId}` }))).toEqual({
      primary: 'cmux',
      secondary: '7DCCBE94',
      title: `External cmux surface ${surfaceId}`,
    })

    expect(buildTaskTerminalAssigneeDisplay(makeIssue({ assignee: 'assistant' }))).toEqual({
      primary: 'assistant',
      secondary: null,
      title: 'assistant',
    })
  })

  it('focuses an external cmux source instead of opening the embedded terminal', async () => {
    const focusCmuxSurface = vi.fn().mockResolvedValue(undefined)
    const openEmbedded = vi.fn()
    const source = resolveTaskTerminalSource(makeIssue({ assignee: `cmux:${surfaceId}` }))

    const result = await focusTaskTerminalSource(source, { focusCmuxSurface, openEmbedded })

    expect(result).toBe('external-focused')
    expect(focusCmuxSurface).toHaveBeenCalledWith(surfaceId)
    expect(openEmbedded).not.toHaveBeenCalled()
  })

  it('opens the embedded terminal when the issue has no external cmux source', async () => {
    const focusCmuxSurface = vi.fn()
    const openEmbedded = vi.fn()
    const source = resolveTaskTerminalSource(makeIssue({ assignee: 'assistant' }))

    const result = await focusTaskTerminalSource(source, { focusCmuxSurface, openEmbedded })

    expect(result).toBe('embedded-opened')
    expect(openEmbedded).toHaveBeenCalledOnce()
    expect(focusCmuxSurface).not.toHaveBeenCalled()
  })
})
