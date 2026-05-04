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

export interface AutoModeDispatchReviewRequest {
  projectPath: string
  issueId: string
  issueTitle: string
  taskBranch: string
  executorCommit: string
}

export interface AutoModeDispatchReviewResponse {
  surface: string
  workspaceName: string
}

export interface AutoModeMergeApprovedRequest {
  projectPath: string
  issueId: string
  taskBranch: string
}

export interface AutoModeMergeApprovedResponse {
  merged: boolean
  closed: boolean
  worktreeRemoved: boolean
}

export interface AutoModeCancelTaskRequest {
  projectPath: string
  issueId: string
  surface?: string
}

export interface AutoModeCancelTaskResponse {
  workspaceClosed: boolean
  issueReset: boolean
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

  async function dispatchReview(issueId: string, executorCommit: string) {
    const task = activeTaskMap.value.get(issueId)
    if (!task || task.status !== 'running') return
    if (!executorCommit) return

    task.status = 'reviewing'
    activeTaskMap.value = new Map(activeTaskMap.value.set(issueId, { ...task }))

    try {
      logFrontend('info', `[auto-mode] Dispatching reviewer for ${issueId} (commit ${executorCommit})`)

      const result = await invoke<AutoModeDispatchReviewResponse>('auto_mode_dispatch_review', {
        request: {
          projectPath: projectPath.value,
          issueId,
          issueTitle: task.title,
          taskBranch: task.worktreeBranch ?? `task-${issueId}`,
          executorCommit,
        } satisfies AutoModeDispatchReviewRequest,
      })

      logFrontend('info', `[auto-mode] Reviewer running on ${result.surface} for ${issueId}`)
    } catch (e) {
      task.status = 'failed'
      task.error = `Review dispatch failed: ${e}`
      activeTaskMap.value = new Map(activeTaskMap.value.set(issueId, { ...task }))
      logFrontend('error', `[auto-mode] Failed to dispatch reviewer for ${issueId}: ${e}`)
    }
  }

  async function mergeApproved(issueId: string) {
    const task = activeTaskMap.value.get(issueId)
    if (!task || task.status !== 'reviewing') return

    task.status = 'merging'
    activeTaskMap.value = new Map(activeTaskMap.value.set(issueId, { ...task }))

    try {
      logFrontend('info', `[auto-mode] Merging approved task ${issueId}`)

      const result = await invoke<AutoModeMergeApprovedResponse>('auto_mode_merge_approved', {
        request: {
          projectPath: projectPath.value,
          issueId,
          taskBranch: task.worktreeBranch ?? `task-${issueId}`,
        } satisfies AutoModeMergeApprovedRequest,
      })

      task.status = 'done'
      activeTaskMap.value = new Map(activeTaskMap.value.set(issueId, { ...task }))
      logFrontend('info', `[auto-mode] Task ${issueId} merged=${result.merged} closed=${result.closed} worktree_removed=${result.worktreeRemoved}`)
    } catch (e) {
      task.status = 'failed'
      task.error = `Merge failed: ${e}`
      activeTaskMap.value = new Map(activeTaskMap.value.set(issueId, { ...task }))
      logFrontend('error', `[auto-mode] Merge failed for ${issueId}: ${e}`)
    }
  }

  async function cancelTask(issueId: string) {
    const task = activeTaskMap.value.get(issueId)
    if (!task) return

    logFrontend('info', `[auto-mode] Cancelling task ${issueId}`)

    try {
      await invoke<AutoModeCancelTaskResponse>('auto_mode_cancel_task', {
        request: {
          projectPath: projectPath.value,
          issueId,
          surface: task.surface ?? undefined,
        } satisfies AutoModeCancelTaskRequest,
      })

      const updated = new Map(activeTaskMap.value)
      updated.delete(issueId)
      activeTaskMap.value = updated

      logFrontend('info', `[auto-mode] Task ${issueId} cancelled`)
    } catch (e) {
      logFrontend('error', `[auto-mode] Cancel failed for ${issueId}: ${e}`)
      task.status = 'failed'
      task.error = `Cancel failed: ${e}`
      activeTaskMap.value = new Map(activeTaskMap.value.set(issueId, { ...task }))
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
    cancelTask,
    dispatchReview,
    mergeApproved,
  }
}
