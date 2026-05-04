import { ref, watch, onUnmounted, computed } from 'vue'
import { useProjectStorage } from './useProjectStorage'
import { invoke } from '@tauri-apps/api/core'
import { logFrontend } from '~/utils/bd-api'
import { appendAutoModeLog, type AutoModeEventType } from '~/utils/auto-mode-log'
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

export type AutoModeLifecycleAction =
  | { type: 'dispatch_review', commit: string }
  | { type: 'merge_approved' }
  | { type: 'review_failed', error: string }
  | null

export interface UseAutoModeOptions {
  refreshReady?: () => Promise<Issue[] | null | void>
  readyPollIntervalMs?: number
  allIssues?: Ref<Issue[]>
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

function latestMatchingComment(issue: Issue, predicate: (content: string) => boolean): string | null {
  for (const comment of [...(issue.comments ?? [])].reverse()) {
    if (predicate(comment.content)) return comment.content
  }
  return null
}

function extractExecutorCommit(content: string): string | null {
  const match = content.match(/^commit:\s*([0-9a-f]{6,40})\s*$/im)
  return match?.[1] ?? null
}

export function getAutoModeLifecycleAction(task: AutoModeTask, issue: Issue): AutoModeLifecycleAction {
  if (task.status === 'running') {
    const executorComplete = latestMatchingComment(issue, content => content.includes('EXECUTOR_COMPLETE'))
    const commit = executorComplete ? extractExecutorCommit(executorComplete) : null
    return commit ? { type: 'dispatch_review', commit } : null
  }

  if (task.status === 'reviewing') {
    const verdict = latestMatchingComment(issue, content => content.includes('REVIEW_VERDICT:'))
    if (!verdict) return null
    if (/REVIEW_VERDICT:\s*APPROVED/i.test(verdict)) return { type: 'merge_approved' }
    if (/REVIEW_VERDICT:\s*CHANGES_REQUESTED/i.test(verdict)) {
      return { type: 'review_failed', error: verdict }
    }
  }

  return null
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

  function logEvent(eventType: AutoModeEventType, issueId: string, detail: string, extra?: { surface?: string, error?: string }) {
    if (!projectPath.value) return
    appendAutoModeLog(projectPath.value, { issueId, eventType, detail, ...extra })
  }

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

  async function caffeinateStart() {
    if (!isTauri()) return
    try {
      const started = await invoke<boolean>('caffeinate_start')
      if (started) logFrontend('info', '[auto-mode] caffeinate started — Mac sleep prevented')
    } catch (e) {
      logFrontend('warn', `[auto-mode] caffeinate_start failed: ${e}`)
    }
  }

  async function caffeinateStop() {
    if (!isTauri()) return
    try {
      const stopped = await invoke<boolean>('caffeinate_stop')
      if (stopped) logFrontend('info', '[auto-mode] caffeinate stopped — Mac sleep allowed')
    } catch (e) {
      logFrontend('warn', `[auto-mode] caffeinate_stop failed: ${e}`)
    }
  }

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
      logEvent('dispatch_start', issue.id, `Dispatching: ${issue.title}`)
      await caffeinateStart()

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
      logEvent('dispatch_success', issue.id, `Running on ${result.surface}, branch ${result.branch}`, { surface: result.surface })
      lastDispatchAt.value = Date.now()
    } catch (e) {
      task.status = 'failed'
      task.error = String(e)
      activeTaskMap.value = new Map(activeTaskMap.value.set(issue.id, { ...task }))
      logFrontend('error', `[auto-mode] Failed to dispatch ${issue.id}: ${e}`)
      logEvent('dispatch_failed', issue.id, `Dispatch failed`, { error: String(e) })
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
      logEvent('review_start', issueId, `Review dispatched for commit ${executorCommit}`)

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
      logEvent('review_start', issueId, `Reviewer running on ${result.surface}`, { surface: result.surface })
    } catch (e) {
      task.status = 'failed'
      task.error = `Review dispatch failed: ${e}`
      activeTaskMap.value = new Map(activeTaskMap.value.set(issueId, { ...task }))
      logFrontend('error', `[auto-mode] Failed to dispatch reviewer for ${issueId}: ${e}`)
      logEvent('review_failed', issueId, `Review dispatch failed`, { error: String(e) })
    }
  }

  async function mergeApproved(issueId: string) {
    const task = activeTaskMap.value.get(issueId)
    if (!task || task.status !== 'reviewing') return

    task.status = 'merging'
    activeTaskMap.value = new Map(activeTaskMap.value.set(issueId, { ...task }))

    try {
      logFrontend('info', `[auto-mode] Merging approved task ${issueId}`)
      logEvent('merge_start', issueId, `Merge started`)

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
      logEvent('merge_success', issueId, `Merged=${result.merged} closed=${result.closed} worktree_removed=${result.worktreeRemoved}`)
    } catch (e) {
      task.status = 'failed'
      task.error = `Merge failed: ${e}`
      activeTaskMap.value = new Map(activeTaskMap.value.set(issueId, { ...task }))
      logFrontend('error', `[auto-mode] Merge failed for ${issueId}: ${e}`)
      logEvent('merge_failed', issueId, `Merge failed`, { error: String(e) })
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

  function reconcileLifecycle(issues: Issue[]) {
    for (const task of activeTaskMap.value.values()) {
      const issue = issues.find(candidate => candidate.id === task.issueId)
      if (!issue) continue

      const action = getAutoModeLifecycleAction(task, issue)
      if (!action) continue

      if (action.type === 'dispatch_review') {
        dispatchReview(task.issueId, action.commit)
      } else if (action.type === 'merge_approved') {
        mergeApproved(task.issueId)
      } else if (action.type === 'review_failed') {
        task.status = 'failed'
        task.error = action.error
        activeTaskMap.value = new Map(activeTaskMap.value.set(task.issueId, { ...task }))
        logEvent('review_failed', task.issueId, 'Review requested changes', { error: action.error })
      }
    }
  }

  const lifecycleIssues = computed(() => options.allIssues?.value ?? [...readyIssues.value, ...inProgressIssues.value])

  watch([canDispatch, readyIssues, lifecycleIssues], () => {
    tryDispatch()
    reconcileLifecycle(lifecycleIssues.value)
  })

  watch(enabled, (on) => {
    if (on) {
      logFrontend('info', `[auto-mode] Enabled for ${projectPath.value}`)
      logEvent('enabled', '-', `Auto-mode enabled for ${projectPath.value}`)
      startReadyPolling()
      tryDispatch()
    } else {
      logFrontend('info', `[auto-mode] Disabled`)
      logEvent('disabled', '-', `Auto-mode disabled`)
      stopReadyPolling()
      caffeinateStop()
    }
  })

  watch(hasRunningTask, (running) => {
    if (!running && enabled.value) {
      caffeinateStop()
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
    caffeinateStop()
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
