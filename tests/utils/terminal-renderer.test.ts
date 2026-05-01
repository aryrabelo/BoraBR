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

  it('falls back with an explicit reason when libghostty is not available', () => {
    const renderer = resolveTerminalRenderer({
      target: 'libghostty',
      capabilities: { libghostty: false },
    })

    expect(renderer.target).toBe('libghostty')
    expect(renderer.active).toBe('dom-scrollback')
    expect(renderer.fallbackReason).toContain('libghostty')
  })
})
