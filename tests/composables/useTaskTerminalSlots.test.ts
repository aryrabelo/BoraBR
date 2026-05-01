import { describe, expect, it } from 'vitest'
import { useTaskTerminalSlots } from '~/composables/useTaskTerminalSlots'

describe('useTaskTerminalSlots', () => {
  it('opens and closes a terminal slot for a task issue', () => {
    const slots = useTaskTerminalSlots()

    slots.openIssueTerminal('borabr-m0z.6')
    expect(slots.isIssueTerminalOpen('borabr-m0z.6')).toBe(true)
    expect(slots.openIssueIds.value).toEqual(['borabr-m0z.6'])

    slots.closeIssueTerminal('borabr-m0z.6')
    expect(slots.isIssueTerminalOpen('borabr-m0z.6')).toBe(false)
    expect(slots.openIssueIds.value).toEqual([])
  })

  it('keeps multiple task terminals open without losing state', () => {
    const slots = useTaskTerminalSlots()

    slots.openIssueTerminal('borabr-m0z.6')
    slots.openIssueTerminal('borabr-m0z.7')
    slots.closeIssueTerminal('borabr-m0z.6')

    expect(slots.isIssueTerminalOpen('borabr-m0z.6')).toBe(false)
    expect(slots.isIssueTerminalOpen('borabr-m0z.7')).toBe(true)
    expect(slots.openIssueIds.value).toEqual(['borabr-m0z.7'])
  })

  it('toggles the same task terminal slot', () => {
    const slots = useTaskTerminalSlots()

    slots.toggleIssueTerminal('borabr-m0z.6')
    expect(slots.isIssueTerminalOpen('borabr-m0z.6')).toBe(true)

    slots.toggleIssueTerminal('borabr-m0z.6')
    expect(slots.isIssueTerminalOpen('borabr-m0z.6')).toBe(false)
  })

  it('guards an active agent terminal from one-click close until it exits', () => {
    const slots = useTaskTerminalSlots()

    slots.openIssueTerminal('borabr-m0z.12')
    slots.setIssueTerminalAgentActive('borabr-m0z.12', true)

    expect(slots.isIssueTerminalCloseGuarded('borabr-m0z.12')).toBe(true)
    expect(slots.closeIssueTerminal('borabr-m0z.12')).toBe(false)
    expect(slots.isIssueTerminalOpen('borabr-m0z.12')).toBe(true)

    slots.setIssueTerminalAgentActive('borabr-m0z.12', false)

    expect(slots.isIssueTerminalCloseGuarded('borabr-m0z.12')).toBe(false)
    expect(slots.closeIssueTerminal('borabr-m0z.12')).toBe(true)
    expect(slots.isIssueTerminalOpen('borabr-m0z.12')).toBe(false)
  })

  it('allows explicit forced close for an active terminal stop path', () => {
    const slots = useTaskTerminalSlots()

    slots.openIssueTerminal('borabr-m0z.12')
    slots.setIssueTerminalAgentActive('borabr-m0z.12', true)

    expect(slots.closeIssueTerminal('borabr-m0z.12', { force: true })).toBe(true)
    expect(slots.isIssueTerminalOpen('borabr-m0z.12')).toBe(false)
    expect(slots.isIssueTerminalCloseGuarded('borabr-m0z.12')).toBe(false)
  })
})
