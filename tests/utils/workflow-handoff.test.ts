import { describe, expect, it } from 'vitest'
import type { Comment } from '~/types/issue'
import {
  buildWorkflowHandoffComment,
  buildWorkflowPullRequestCommand,
  latestWorkflowHandoff,
  parseWorkflowHandoffComment,
  parseWorkflowHandoffs,
  resolveWorkflowBranch,
} from '~/utils/workflow-handoff'

function comment(content: string, createdAt = '2026-05-04T10:00:00Z'): Comment {
  return {
    id: createdAt,
    author: 'assistant',
    content,
    createdAt,
  }
}

describe('workflow handoff', () => {
  it('parses structured step comments', () => {
    const record = parseWorkflowHandoffComment(comment(
      'step:implement-fix {"branch":"epic/borabr-lw4","commit":"abc123","files":["app/utils/foo.ts"]}',
    ))

    expect(record?.stepId).toBe('implement-fix')
    expect(record?.payload.branch).toBe('epic/borabr-lw4')
    expect(record?.payload.commit).toBe('abc123')
    expect(record?.payload.files).toEqual(['app/utils/foo.ts'])
  })

  it('ignores unstructured or malformed comments', () => {
    expect(parseWorkflowHandoffComment(comment('step:implement abc123 app/foo.ts'))).toBeNull()
    expect(parseWorkflowHandoffComment(comment('step:test {not-json'))).toBeNull()
    expect(parseWorkflowHandoffComment(comment('ordinary note'))).toBeNull()
  })

  it('returns handoffs in chronological order and exposes latest', () => {
    const comments = [
      comment('step:test {"branch":"epic/borabr-lw4","commit":"b","files":["b.ts"]}', '2026-05-04T10:05:00Z'),
      comment('step:implement {"branch":"epic/borabr-lw4","commit":"a","files":["a.ts"]}', '2026-05-04T10:01:00Z'),
    ]

    expect(parseWorkflowHandoffs(comments).map(record => record.stepId)).toEqual(['implement', 'test'])
    expect(latestWorkflowHandoff(comments)?.payload.commit).toBe('b')
  })

  it('formats machine-readable handoff comments', () => {
    expect(buildWorkflowHandoffComment('implement', {
      branch: 'epic/borabr-lw4',
      commit: 'abc123',
      files: ['src-tauri/src/lib.rs'],
    })).toBe('step:implement {"branch":"epic/borabr-lw4","commit":"abc123","files":["src-tauri/src/lib.rs"]}')
  })

  it('resolves shared epic branch patterns', () => {
    expect(resolveWorkflowBranch('epic/{parent-id}', 'borabr-lw4')).toBe('epic/borabr-lw4')
  })

  it('builds a single PR command from the shared branch', () => {
    expect(buildWorkflowPullRequestCommand({
      branch: 'epic/borabr-lw4',
      title: 'Fix auto-mode workflow',
      body: 'Shared branch evidence',
    })).toBe("gh pr create --base 'master' --head 'epic/borabr-lw4' --title 'Fix auto-mode workflow' --body 'Shared branch evidence'")
  })
})
