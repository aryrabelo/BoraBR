import { invoke } from '@tauri-apps/api/core'

export type AutoModeEventType =
  | 'enabled'
  | 'disabled'
  | 'dispatch_start'
  | 'dispatch_success'
  | 'dispatch_failed'
  | 'review_start'
  | 'review_failed'
  | 'merge_start'
  | 'merge_success'
  | 'merge_failed'
  | 'task_failed'
  | 'watchdog'

export interface AutoModeLogEntry {
  timestamp: string
  issueId: string
  eventType: AutoModeEventType
  detail: string
  surface?: string
  error?: string
}

export interface AutoModeLogRecord {
  timestamp: string
  issueId: string
  eventType: string
  detail: string
  surface?: string | null
  error?: string | null
}

function isTauri(): boolean {
  return typeof window !== 'undefined' && (!!window.__TAURI__ || !!window.__TAURI_INTERNALS__)
}

export async function appendAutoModeLog(
  projectPath: string,
  entry: Omit<AutoModeLogEntry, 'timestamp'>,
): Promise<void> {
  if (!isTauri()) return

  try {
    await invoke('auto_mode_log_append', {
      projectPath,
      entry: {
        timestamp: new Date().toISOString(),
        ...entry,
      },
    })
  } catch (e) {
    console.error('[auto-mode-log] Failed to append:', e)
  }
}

export async function readAutoModeLog(
  projectPath: string,
  limit?: number,
): Promise<AutoModeLogRecord[]> {
  if (!isTauri()) return []

  try {
    return await invoke<AutoModeLogRecord[]>('auto_mode_log_read', {
      projectPath,
      limit: limit ?? null,
    })
  } catch (e) {
    console.error('[auto-mode-log] Failed to read:', e)
    return []
  }
}

export async function clearAutoModeLog(projectPath: string): Promise<void> {
  if (!isTauri()) return

  try {
    await invoke('auto_mode_log_clear', { projectPath })
  } catch {
    // ignore
  }
}
