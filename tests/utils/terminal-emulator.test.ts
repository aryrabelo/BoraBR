import { describe, expect, it } from 'vitest'
import { Terminal as HeadlessTerminal } from '@xterm/headless'
import { createTerminalEmulator } from '~/utils/terminal-emulator'

class FakeFitAddon {
  fitCalls = 0

  fit() {
    this.fitCalls += 1
  }
}

class FakeTerminal {
  static instances: FakeTerminal[] = []

  writes: string[] = []
  loadAddonCalls: unknown[] = []
  openCalls: HTMLElement[] = []
  resizeCalls: Array<{ cols: number, rows: number }> = []
  focusCalls = 0
  disposed = false
  selection = 'selected output'

  constructor(public options: unknown) {
    FakeTerminal.instances.push(this)
  }

  loadAddon(addon: unknown) {
    this.loadAddonCalls.push(addon)
  }

  open(element: HTMLElement) {
    this.openCalls.push(element)
  }

  write(data: string) {
    this.writes.push(data)
  }

  resize(cols: number, rows: number) {
    this.resizeCalls.push({ cols, rows })
  }

  focus() {
    this.focusCalls += 1
  }

  getSelection() {
    return this.selection
  }

  dispose() {
    this.disposed = true
  }
}

describe('createTerminalEmulator', () => {
  it('passes raw PTY bytes to a terminal emulator instead of text-sanitizing them', async () => {
    FakeTerminal.instances = []
    const emulator = createTerminalEmulator({
      TerminalCtor: FakeTerminal,
      FitAddonCtor: FakeFitAddon,
    })
    const container = document.createElement('div')
    const rawPrompt = '\x1B[1;36m~/project\x1B[0m on \x1B[1;35mmain\x1B[0m \x1B[?2004h'

    await emulator.mount(container)
    emulator.write(rawPrompt)

    const terminal = FakeTerminal.instances[0]!
    const fitAddon = terminal.loadAddonCalls[0] as FakeFitAddon
    expect(terminal.openCalls).toEqual([container])
    expect(terminal.writes).toEqual([rawPrompt])
    expect(container.textContent).toBe('')
    expect(fitAddon.fitCalls).toBe(1)
  })

  it('resizes, focuses, copies selection, and disposes through the emulator', async () => {
    FakeTerminal.instances = []
    const emulator = createTerminalEmulator({
      TerminalCtor: FakeTerminal,
      FitAddonCtor: FakeFitAddon,
    })
    await emulator.mount(document.createElement('div'))

    emulator.resize(120, 32)
    emulator.focus()
    const selection = emulator.getSelection()
    emulator.dispose()

    const terminal = FakeTerminal.instances[0]!
    expect(terminal.resizeCalls).toEqual([{ cols: 120, rows: 32 }])
    expect(terminal.focusCalls).toBe(1)
    expect(selection).toBe('selected output')
    expect(terminal.disposed).toBe(true)
  })
})

describe('xterm parser regression coverage', () => {
  it('interprets the prompt ANSI, OSC, cursor, erase, and bracketed-paste sequences', async () => {
    const terminal = new HeadlessTerminal({ allowProposedApi: true, cols: 220, rows: 8 })
    const prompt = [
      '\x1B]7;file://M5-de-Ary-9.local/Users/aryrabelo/Sites/beads-task-issue-tracker\x07\x1B]0;master\x1B\\',
      '\x1B[0m\x1B[27m\x1B[24m\x1B[J',
      '\x1B[1;36m~/Sites/beads-task-issue-tracker\x1B[0m on  \x1B[1;35mmaster\x1B[0m  \x1B[1;33m[!3\u21e16]\x1B[0m  \x1B[1;32mv22.22.1\x1B[0m',
      '\x1B[1;32m\u276f\x1B[0m  \x1B[K\x1B[164C\x1B[1;37m15:12:29\x1B[0m\x1B[172D\x1B[?1h\x1B=\x1B[?2004h',
    ].join('\r\n')

    await writeHeadless(terminal, prompt)

    const rendered = Array.from({ length: terminal.buffer.active.length }, (_, index) =>
      terminal.buffer.active.getLine(index)?.translateToString(true) ?? '',
    ).join('\n')
    const styledCell = terminal.buffer.active.getLine(2)?.getCell(0)

    expect(rendered).toContain('~/Sites/beads-task-issue-tracker')
    expect(rendered).toContain('master')
    expect(rendered).toContain('[!3\u21e16]')
    expect(rendered).toContain('v22.22.1')
    expect(rendered).toContain('\u276f')
    expect(rendered).toContain('15:12:29')
    expect(rendered).not.toContain(']7;file://')
    expect(rendered).not.toContain(']0;')
    expect(rendered).not.toContain('[0m')
    expect(rendered).not.toContain('[27m')
    expect(rendered).not.toContain('[24m')
    expect(rendered).not.toContain('[J')
    expect(rendered).not.toContain('[K')
    expect(rendered).not.toContain('[164C')
    expect(rendered).not.toContain('[172D')
    expect(rendered).not.toContain('[?1h')
    expect(rendered).not.toContain('[?2004h')
    expect(styledCell?.isBold()).toBeGreaterThan(0)
  })
})

function writeHeadless(terminal: HeadlessTerminal, data: string) {
  return new Promise<void>(resolve => terminal.write(data, resolve))
}
