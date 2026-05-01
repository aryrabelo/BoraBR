export type TerminalRendererTarget = 'libghostty' | 'dom-scrollback'
export type TerminalRendererActive = 'libghostty' | 'dom-scrollback'

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

  if (target === 'dom-scrollback') {
    return {
      target,
      active: 'dom-scrollback',
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
    active: 'dom-scrollback',
    fallbackReason: 'libghostty native renderer bridge is not available in this build',
  }
}
