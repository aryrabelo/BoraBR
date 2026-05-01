export interface ResolveTaskTerminalToggleStateInput {
  issueId: string
  active?: boolean
  closeGuarded?: boolean
}

export interface TaskTerminalToggleState {
  ariaLabel: string
  disabled: boolean
  title: string
}

export function resolveTaskTerminalToggleState(input: ResolveTaskTerminalToggleStateInput): TaskTerminalToggleState {
  if (input.active && input.closeGuarded) {
    return {
      ariaLabel: `Task terminal for ${input.issueId} is running`,
      disabled: true,
      title: 'Task terminal is running; stop it from the terminal panel before closing',
    }
  }

  if (input.active) {
    return {
      ariaLabel: `Close task terminal for ${input.issueId}`,
      disabled: false,
      title: 'Close task terminal',
    }
  }

  return {
    ariaLabel: `Open task terminal for ${input.issueId}`,
    disabled: false,
    title: 'Open task terminal',
  }
}
