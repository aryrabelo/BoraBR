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

  it('sets cmux surface as assignee when starting an issue from helper commands', () => {
    const helpers = buildTerminalHelperCommands({
      id: 'borabr-m0z.4',
      title: 'Add Beads workflow helpers',
    })
    const startCommand = helpers.find(helper => helper.id === 'start')?.command

    expect(startCommand).toContain('SURFACE_ID="${CMUX_SURFACE_ID:-${CANIX_PANEL_ID:-${CMUX_PANEL_ID:-}}}"')
    expect(startCommand).toContain('ASSIGNEE="cmux:${SURFACE_ID}"')
    expect(startCommand).toContain('br update --actor "$ACTOR" \'borabr-m0z.4\' --status in_progress --assignee "$ASSIGNEE" --json')
    expect(startCommand).toContain('else br update --actor "$ACTOR" \'borabr-m0z.4\' --status in_progress --claim --json')
    expect(startCommand).not.toContain('then &&')
    expect(startCommand).not.toContain('else &&')
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

  it('shows workflow commands for project type-scoped bug and plan workflows', () => {
    const bugHelpers = buildTerminalHelperCommands({
      id: 'borabr-bug',
      title: 'Fix broken save',
      type: 'bug',
      labels: [],
    })
    const planHelpers = buildTerminalHelperCommands({
      id: 'borabr-plan',
      title: 'Plan workflow rollout',
      type: 'plan',
      labels: [],
    })

    expect(bugHelpers.find(helper => helper.id === 'workflow-check')?.command).toBe('br workflow check borabr-bug')
    expect(planHelpers.find(helper => helper.id === 'workflow-next')?.command).toBe('br workflow next borabr-plan')
  })
})
