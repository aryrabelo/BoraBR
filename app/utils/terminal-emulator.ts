export interface TerminalEmulator {
  mount: (container: HTMLElement) => Promise<void>
  write: (data: string) => void
  resize: (cols: number, rows: number) => void
  clear: () => void
  focus: () => void
  getSelection: () => string
  dispose: () => void
}

export interface TerminalLike {
  loadAddon: (addon: unknown) => void
  open: (container: HTMLElement) => void
  write: (data: string) => void
  resize: (cols: number, rows: number) => void
  clear?: () => void
  focus?: () => void
  getSelection?: () => string
  dispose: () => void
}

export interface FitAddonLike {
  fit: () => void
}

export type TerminalConstructor = new (options: Record<string, unknown>) => TerminalLike
export type FitAddonConstructor = new () => FitAddonLike

export interface CreateTerminalEmulatorOptions {
  TerminalCtor?: TerminalConstructor
  FitAddonCtor?: FitAddonConstructor
  terminalOptions?: Record<string, unknown>
}

export function createTerminalEmulator(options: CreateTerminalEmulatorOptions = {}): TerminalEmulator {
  let terminal: TerminalLike | null = null
  let fitAddon: FitAddonLike | null = null

  async function resolveConstructors() {
    if (options.TerminalCtor && options.FitAddonCtor) {
      return {
        TerminalCtor: options.TerminalCtor,
        FitAddonCtor: options.FitAddonCtor,
      }
    }

    const [{ Terminal }, { FitAddon }] = await Promise.all([
      import('@xterm/xterm'),
      import('@xterm/addon-fit'),
    ])

    return {
      TerminalCtor: Terminal as unknown as TerminalConstructor,
      FitAddonCtor: FitAddon as unknown as FitAddonConstructor,
    }
  }

  return {
    async mount(container) {
      if (terminal) return

      const { TerminalCtor, FitAddonCtor } = await resolveConstructors()
      terminal = new TerminalCtor({
        allowProposedApi: false,
        convertEol: true,
        cursorBlink: true,
        fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", monospace',
        fontSize: 12,
        lineHeight: 1.35,
        scrollback: 5000,
        theme: {
          background: '#272b33',
          foreground: '#d7dce2',
          cursor: '#d7dce2',
          selectionBackground: '#4b5563',
          black: '#1f2329',
          red: '#df6f6c',
          green: '#a6be6a',
          yellow: '#d7ba6a',
          blue: '#78a9d9',
          magenta: '#b48ead',
          cyan: '#78b6ad',
          white: '#d7dce2',
          brightBlack: '#68717d',
          brightRed: '#e88380',
          brightGreen: '#b4ca7a',
          brightYellow: '#e0c77a',
          brightBlue: '#8bb8e8',
          brightMagenta: '#c09ad8',
          brightCyan: '#8bc7be',
          brightWhite: '#f0f3f6',
        },
        ...options.terminalOptions,
      })
      fitAddon = new FitAddonCtor()
      terminal.loadAddon(fitAddon)
      terminal.open(container)
      fitAddon.fit()
    },
    write(data) {
      terminal?.write(data)
    },
    resize(cols, rows) {
      terminal?.resize(cols, rows)
      fitAddon?.fit()
    },
    clear() {
      terminal?.clear?.()
    },
    focus() {
      terminal?.focus?.()
    },
    getSelection() {
      return terminal?.getSelection?.() ?? ''
    },
    dispose() {
      terminal?.dispose()
      terminal = null
      fitAddon = null
    },
  }
}
