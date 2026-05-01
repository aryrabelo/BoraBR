export type TerminalRendererTarget = 'libghostty' | 'xterm'
export type TerminalRendererActive = 'libghostty' | 'ghostty-external' | 'xterm'

export interface TerminalGhosttyExternalBridge {
  available: boolean
  command?: string | null
  reason?: string | null
}

export interface TerminalRendererCapabilities {
  libghostty: boolean
  ghosttyExternal: TerminalGhosttyExternalBridge
}

export interface ResolveTerminalRendererInput {
  target?: TerminalRendererTarget
  capabilities?: Partial<TerminalRendererCapabilities>
}

export interface TerminalRendererResolution {
  target: TerminalRendererTarget
  active: TerminalRendererActive
  fallbackReason: string | null
  bridgeCommand?: string | null
}

export function resolveTerminalRenderer(input: ResolveTerminalRendererInput = {}): TerminalRendererResolution {
  const target = input.target ?? 'libghostty'
  const libghosttyAvailable = input.capabilities?.libghostty ?? false
  const ghosttyExternal = input.capabilities?.ghosttyExternal

  if (target === 'xterm') {
    return {
      target,
      active: 'xterm',
      fallbackReason: null,
    }
  }

  if (libghosttyAvailable) {
    return {
      target: 'libghostty',
      active: 'libghostty',
      fallbackReason: null,
    }
  }

  if (ghosttyExternal?.available) {
    return {
      target: 'libghostty',
      active: 'ghostty-external',
      fallbackReason: null,
      bridgeCommand: ghosttyExternal.command ?? null,
    }
  }

  return {
    target: 'libghostty',
    active: 'xterm',
    fallbackReason: ghosttyExternal?.reason
      ? `libghostty native renderer bridge is not available in this build and Ghostty external bridge is unavailable: ${ghosttyExternal.reason}; using xterm terminal emulator`
      : 'libghostty native renderer bridge is not available in this build; using xterm terminal emulator',
    bridgeCommand: null,
  }
}
