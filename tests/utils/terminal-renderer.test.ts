import { describe, expect, it } from 'vitest'
import { resolveTerminalRenderer } from '~/utils/terminal-renderer'

describe('resolveTerminalRenderer', () => {
  it('uses libghostty when the native renderer bridge is available', () => {
    const renderer = resolveTerminalRenderer({
      target: 'libghostty',
      capabilities: { libghostty: true },
    })

    expect(renderer.target).toBe('libghostty')
    expect(renderer.active).toBe('libghostty')
    expect(renderer.fallbackReason).toBeNull()
  })

  it('uses a Ghostty-compatible external bridge when the embedded bridge is unavailable', () => {
    const renderer = resolveTerminalRenderer({
      target: 'libghostty',
      capabilities: {
        libghostty: false,
        ghosttyExternal: {
          available: true,
          command: 'open',
        },
      },
    })

    expect(renderer.target).toBe('libghostty')
    expect(renderer.active).toBe('ghostty-external')
    expect(renderer.fallbackReason).toBeNull()
    expect(renderer.bridgeCommand).toBe('open')
  })

  it('falls back to a terminal emulator with an explicit reason when libghostty is not available', () => {
    const renderer = resolveTerminalRenderer({
      target: 'libghostty',
      capabilities: { libghostty: false },
    })

    expect(renderer.target).toBe('libghostty')
    expect(renderer.active).toBe('xterm')
    expect(renderer.fallbackReason).toContain('libghostty')
  })
})
