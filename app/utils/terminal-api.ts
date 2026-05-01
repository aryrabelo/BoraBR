import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

declare global {
  interface Window {
    __TAURI__?: unknown
    __TAURI_INTERNALS__?: unknown
  }
}

export interface TerminalCreateRequest {
  cwd: string
  issueId?: string
  shell?: string
  cols?: number
  rows?: number
}

export interface TerminalSessionInfo {
  sessionId: string
  cwd: string
  issueId?: string | null
  cols: number
  rows: number
}

export interface TerminalEventPayload {
  sessionId: string
  kind: 'data' | 'exit' | 'error'
  data?: string | null
  message?: string | null
  code?: number | null
}

export interface TerminalWriteRequest {
  sessionId: string
  data: string
}

export interface TerminalResizeRequest {
  sessionId: string
  cols: number
  rows: number
}

export interface TerminalNativeGhosttyExternalBridge {
  available: boolean
  command?: string | null
  reason?: string | null
}

export interface TerminalNativeRendererCapabilities {
  libghostty: boolean
  ghosttyExternal: TerminalNativeGhosttyExternalBridge
}

export interface OpenNativeTerminalRendererRequest {
  cwd: string
  issueId?: string
  shell?: string
}

export interface OpenNativeTerminalRendererResponse {
  renderer: 'ghostty-external'
  sessionId: string
  command: string
  pid?: number | null
}

export function isTerminalAvailable(): boolean {
  return typeof window !== 'undefined' && (!!window.__TAURI__ || !!window.__TAURI_INTERNALS__)
}

function requireTerminal(): void {
  if (!isTerminalAvailable()) {
    throw new Error('Terminal sessions are available in the desktop app')
  }
}

export async function createTerminalSession(request: TerminalCreateRequest): Promise<TerminalSessionInfo> {
  requireTerminal()
  return invoke<TerminalSessionInfo>('terminal_create', { request })
}

export async function writeTerminal(request: TerminalWriteRequest): Promise<void> {
  requireTerminal()
  await invoke('terminal_write', { request })
}

export async function resizeTerminal(request: TerminalResizeRequest): Promise<TerminalSessionInfo> {
  requireTerminal()
  return invoke<TerminalSessionInfo>('terminal_resize', { request })
}

export async function restartTerminal(sessionId: string): Promise<TerminalSessionInfo> {
  requireTerminal()
  return invoke<TerminalSessionInfo>('terminal_restart', { request: { sessionId } })
}

export async function closeTerminal(sessionId: string): Promise<void> {
  requireTerminal()
  await invoke('terminal_close', { request: { sessionId } })
}

export async function listTerminalSessions(): Promise<TerminalSessionInfo[]> {
  requireTerminal()
  return invoke<TerminalSessionInfo[]>('terminal_list')
}

export async function getTerminalNativeRendererCapabilities(): Promise<TerminalNativeRendererCapabilities> {
  requireTerminal()
  return invoke<TerminalNativeRendererCapabilities>('terminal_native_renderer_capabilities')
}

export async function openNativeTerminalRenderer(request: OpenNativeTerminalRendererRequest): Promise<OpenNativeTerminalRendererResponse> {
  requireTerminal()
  return invoke<OpenNativeTerminalRendererResponse>('terminal_open_native_renderer', { request })
}

export async function onTerminalData(handler: (payload: TerminalEventPayload) => void): Promise<UnlistenFn> {
  return listen<TerminalEventPayload>('terminal:data', event => handler(event.payload))
}

export async function onTerminalExit(handler: (payload: TerminalEventPayload) => void): Promise<UnlistenFn> {
  return listen<TerminalEventPayload>('terminal:exit', event => handler(event.payload))
}

export async function onTerminalError(handler: (payload: TerminalEventPayload) => void): Promise<UnlistenFn> {
  return listen<TerminalEventPayload>('terminal:error', event => handler(event.payload))
}
