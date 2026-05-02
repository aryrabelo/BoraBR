import { ref, watch, onUnmounted, computed } from 'vue'
import { useProjectStorage } from './useProjectStorage'
import { invoke } from '@tauri-apps/api/core'
import { logFrontend } from '~/utils/bd-api'
import type { Issue } from '~/types/issue'

export interface AutoModeTask {
  issueId: string
  title: string
  surface?: string
  worktreeBranch?: string
  status: 'dispatching' | 'running' | 'reviewing' | 'merging' | 'done' | 'failed'
  startedAt: number
  error?: string
}

export interface AutoModeDispatchRequest {
  projectPath: string
  issueId: string
  issueTitle: string
}

export interface AutoModeDispatchResponse {
  surface: string
  worktreePath: string
  branch: string
}

export interface UseAutoModeOptions {
  refreshReady?: () => Promise<Issue[] | null | void>
  readyPollIntervalMs?: number
}

const DISPATCH_COOLDOWN = 10_000
const AUTO_MODE_READY_POLL_INTERVAL = 5_000
const AUTO_MODE_PRIORITY_ORDER: Record<string, number> = {
  p0: 0,
  p1: 1,
  p2: 2,
  p3: 3,
  p4: 4,
}

function isTauri(): boolean {
  return typeof window !== 'undefined' && (!!window.__TAURI__ || !!window.__TAURI_INTERNALS__)
}

function getIssuePriorityRank(issue: Issue): number {
  return AUTO_MODE_PRIORITY_ORDER[issue.priority] ?? Number.MAX_SAFE_INTEGER
}

function compareAutoModeIssues(a: Issue, b: Issue): number {
  const priorityDelta = getIssuePriorityRank(a) - getIssuePriorityRank(b)
  if (priorityDelta !== 0) return priorityDelta

  const createdDelta = Date.parse(a.createdAt) - Date.parse(b.createdAt)
  if (Number.isFinite(createdDelta) && createdDelta !== 0) return createdDelta

  return a.id.localeCompare(b.id, undefined, { numeric: true })
}

export function hasAutoModeInProgressTask(issues: Issue[]): boolean {
  return issues.some(issue => issue.status === 'in_progress' && issue.type !== 'epic')
}

export function pickAutoModeIssue(
  readyIssues: Issue[],
  activeTasks: ReadonlyMap<string, AutoModeTask> = new Map(),
): Issue | null {
  return [...readyIssues]
    .filter(issue => issue.status === 'open')
    .filter(issue => issue.type !== 'epic')
    .filter(issue => !activeTasks.has(issue.id))
    .sort(compareAutoModeIssues)[0] ?? null
}

export function useAutoMode(
  projectPath: Ref<string>,
  readyIssues: Ref<Issue[]>,
  inProgressIssues: Ref<Issue[]>,
  options: UseAutoModeOptions = {},
) {
  const enabled = useProjectStorage('autoMode', false)
  const activeTaskMap = ref(new Map<string, AutoModeTask>())
  const lastDispatchAt = ref(0)
  const isDispatching = ref(false)
  let readyPollTimer: ReturnType<typeof setInterval> | null = null
  let isRefreshingReady = false

  const hasRunningTask = computed(() => {
    for (const task of activeTaskMap.value.values()) {
      if (task.status === 'dispatching' || task.status === 'running' || task.status === 'reviewing') {
        return true
      }
    }
    return false
  })

  const activeTaskList = computed(() => Array.from(activeTaskMap.value.values()))

  async function refreshReadyForAutoMode() {
    if (!options.refreshReady) return
    if (!enabled.value) return
    if (!projectPath.value) return
    if (!isTauri()) return
    if (isRefreshingReady) return

    isRefreshingReady = true
    try {
      await options.refreshReady()
    } catch (e) {
      logFrontend('warn', `[auto-mode] Ready refresh failed: ${e}`)
    } finally {
      isRefreshingReady = false
    }
  }

  function stopReadyPolling() {
    if (readyPollTimer) {
      clearInterval(readyPollTimer)
      readyPollTimer = null
    }
  }

  function startReadyPolling() {
    if (!options.refreshReady || readyPollTimer) return

    refreshReadyForAutoMode()
    readyPollTimer = setInterval(
      refreshReadyForAutoMode,
      options.readyPollIntervalMs ?? AUTO_MODE_READY_POLL_INTERVAL,
    )
  }

  const canDispatch = computed(() => {
    if (!enabled.value) return false
    if (!projectPath.value) return false
    if (!isTauri()) return false
    if (isDispatching.value) return false
    if (hasRunningTask.value) return false
    if (hasAutoModeInProgressTask(inProgressIssues.value)) return false
    if (readyIssues.value.length === 0) return false
    if (Date.now() - lastDispatchAt.value < DISPATCH_COOLDOWN) return false
    return true
  })

  async function dispatchTask(issue: Issue) {
    if (isDispatching.value) return

    const task: AutoModeTask = {
      issueId: issue.id,
      title: issue.title,
      status: 'dispatching',
      startedAt: Date.now(),
    }
    activeTaskMap.value = new Map(activeTaskMap.value.set(issue.id, task))
    isDispatching.value = true

    try {
      logFrontend('info', `[auto-mode] Dispatching ${issue.id}: ${issue.title}`)

      const result = await invoke<AutoModeDispatchResponse>('auto_mode_dispatch', {
        request: {
          projectPath: projectPath.value,
          issueId: issue.id,
          issueTitle: issue.title,
        } satisfies AutoModeDispatchRequest,
      })

      task.surface = result.surface
      task.worktreeBranch = result.branch
      task.status = 'running'
      activeTaskMap.value = new Map(activeTaskMap.value.set(issue.id, { ...task }))

      logFrontend('info', `[auto-mode] Task ${issue.id} running on surface ${result.surface}`)
      lastDispatchAt.value = Date.now()
    } catch (e) {
      task.status = 'failed'
      task.error = String(e)
      activeTaskMap.value = new Map(activeTaskMap.value.set(issue.id, { ...task }))
      logFrontend('error', `[auto-mode] Failed to dispatch ${issue.id}: ${e}`)
    } finally {
      isDispatching.value = false
    }
  }

  function tryDispatch() {
    if (!canDispatch.value) return

    const topIssue = pickAutoModeIssue(readyIssues.value, activeTaskMap.value)
    if (!topIssue) return

    dispatchTask(topIssue)
  }

  watch([canDispatch, readyIssues], () => {
    tryDispatch()
  })

  watch(enabled, (on) => {
    if (on) {
      logFrontend('info', `[auto-mode] Enabled for ${projectPath.value}`)
      startReadyPolling()
      tryDispatch()
    } else {
      logFrontend('info', `[auto-mode] Disabled`)
      stopReadyPolling()
    }
  })

  watch(projectPath, () => {
    if (!enabled.value) return
    stopReadyPolling()
    startReadyPolling()
  })

  if (enabled.value) {
    startReadyPolling()
  }

  onUnmounted(() => {
    stopReadyPolling()
  })

  return {
    enabled,
    activeTaskList,
    hasRunningTask,
    isDispatching,
    canDispatch,
  }
}
