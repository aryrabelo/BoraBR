import { describe, expect, it } from 'vitest'
import { resolveTaskTerminalToggleState } from '~/utils/task-terminal-lifecycle'

describe('resolveTaskTerminalToggleState', () => {
  it('uses normal open and close labels when the task terminal is not guarded', () => {
    expect(resolveTaskTerminalToggleState({ issueId: 'borabr-m0z.12', active: false, closeGuarded: false })).toMatchObject({
      ariaLabel: 'Open task terminal for borabr-m0z.12',
      disabled: false,
      title: 'Open task terminal',
    })

    expect(resolveTaskTerminalToggleState({ issueId: 'borabr-m0z.12', active: true, closeGuarded: false })).toMatchObject({
      ariaLabel: 'Close task terminal for borabr-m0z.12',
      disabled: false,
      title: 'Close task terminal',
    })
  })

  it('disables one-click row close while the task terminal is guarded', () => {
    expect(resolveTaskTerminalToggleState({ issueId: 'borabr-m0z.12', active: true, closeGuarded: true })).toMatchObject({
      ariaLabel: 'Task terminal for borabr-m0z.12 is running',
      disabled: true,
      title: 'Task terminal is running; stop it from the terminal panel before closing',
    })
  })
})
