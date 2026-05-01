export type TerminalRendererTarget = 'libghostty' | 'xterm'
export type TerminalRendererActive = 'libghostty' | 'xterm'

export interface TerminalRendererCapabilities {
  libghostty: boolean
}

export interface ResolveTerminalRendererInput {
  target?: TerminalRendererTarget
  capabilities?: Partial<TerminalRendererCapabilities>
}

export interface TerminalRendererResolution {
  target: TerminalRendererTarget
  active: TerminalRendererActive
  fallbackReason: string | null
}

export function resolveTerminalRenderer(input: ResolveTerminalRendererInput = {}): TerminalRendererResolution {
  const target = input.target ?? 'libghostty'
  const libghosttyAvailable = input.capabilities?.libghostty ?? false

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

  return {
    target: 'libghostty',
    active: 'xterm',
    fallbackReason: 'libghostty native renderer bridge is not available in this build; using xterm terminal emulator',
  }
}
