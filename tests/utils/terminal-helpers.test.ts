import { describe, expect, it } from 'vitest'
import { buildTerminalHelperCommands, shellQuote } from '~/utils/terminal-helpers'

describe('terminal helpers', () => {
  it('shell-quotes issue titles safely', () => {
    expect(shellQuote("can't ship")).toBe("'can'\"'\"'t ship'")
  })

  it('exposes project-level Beads commands without a selected issue', () => {
    const helpers = buildTerminalHelperCommands(null)

    expect(helpers.map(helper => helper.id)).toEqual(['create', 'ready', 'sync'])
    expect(helpers[0]?.command).toBe('br create "New issue title" --type task --priority p2')
  })

  it('exposes selected issue context and common mutation commands', () => {
    const helpers = buildTerminalHelperCommands({
      id: 'borabr-m0z.4',
      title: 'Add Beads workflow helpers',
    })

    expect(helpers.map(helper => helper.id)).toEqual([
      'issue-id',
      'show',
      'start',
      'review-start',
      'review-fail',
      'review-pass',
      'review-question',
      'close',
      'comment',
      'label',
      'blocker',
      'create',
      'ready',
      'sync',
    ])
    expect(helpers.find(helper => helper.id === 'issue-id')?.command).toBe('borabr-m0z.4')
    expect(helpers.find(helper => helper.id === 'show')?.command).toBe('br show borabr-m0z.4')
    expect(helpers.find(helper => helper.id === 'comment')?.command).toBe('br comments add borabr-m0z.4 --message "Comment"')
    expect(helpers.find(helper => helper.id === 'label')?.command).toBe('br update borabr-m0z.4 --add-label "label"')
    expect(helpers.find(helper => helper.id === 'review-start')?.command)
      .toContain('br update borabr-m0z.4 --status in_review')
    expect(helpers.find(helper => helper.id === 'review-fail')?.command)
      .toContain('--add-label "review:changes_requested"')
    expect(helpers.find(helper => helper.id === 'review-pass')?.command)
      .toContain('review:passed')
    expect(helpers.find(helper => helper.id === 'review-question')?.command)
      .toContain('--add-label "blocked:needs_answer"')
  })

  it('prefers deterministic workflow commands when a workflow label is present', () => {
    const helpers = buildTerminalHelperCommands({
      id: 'borabr-m0z.9',
      title: 'Align workflow contracts',
      labels: ['workflow:review'],
    })

    expect(helpers.map(helper => helper.id).slice(0, 5)).toEqual([
      'issue-id',
      'show',
      'workflow-check',
      'workflow-steps',
      'workflow-next',
    ])
    expect(helpers.find(helper => helper.id === 'workflow-check')?.command).toBe('br workflow check borabr-m0z.9')
    expect(helpers.find(helper => helper.id === 'workflow-steps')?.command).toBe('br workflow steps borabr-m0z.9 --apply')
    expect(helpers.find(helper => helper.id === 'workflow-next')?.command).toBe('br workflow next borabr-m0z.9')
    expect(helpers.find(helper => helper.id === 'review-start')?.label).toBe('Legacy Review')
  })
})
