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

const DISPATCH_COOLDOWN = 10_000

function isTauri(): boolean {
  return typeof window !== 'undefined' && (!!window.__TAURI__ || !!window.__TAURI_INTERNALS__)
}

export function useAutoMode(projectPath: Ref<string>, readyIssues: Ref<Issue[]>, inProgressIssues: Ref<Issue[]>) {
  const enabled = useProjectStorage('autoMode', false)
  const activeTaskMap = ref(new Map<string, AutoModeTask>())
  const lastDispatchAt = ref(0)
  const isDispatching = ref(false)

  const hasRunningTask = computed(() => {
    for (const task of activeTaskMap.value.values()) {
      if (task.status === 'dispatching' || task.status === 'running' || task.status === 'reviewing') {
        return true
      }
    }
    return false
  })

  const activeTaskList = computed(() => Array.from(activeTaskMap.value.values()))

  const canDispatch = computed(() => {
    if (!enabled.value) return false
    if (!projectPath.value) return false
    if (!isTauri()) return false
    if (isDispatching.value) return false
    if (hasRunningTask.value) return false
    if (inProgressIssues.value.length > 0) return false
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

    const topIssue = readyIssues.value[0]
    if (!topIssue) return

    const alreadyDispatched = activeTaskMap.value.has(topIssue.id)
    if (alreadyDispatched) return

    dispatchTask(topIssue)
  }

  watch([canDispatch, readyIssues], () => {
    tryDispatch()
  })

  watch(enabled, (on) => {
    if (on) {
      logFrontend('info', `[auto-mode] Enabled for ${projectPath.value}`)
      tryDispatch()
    } else {
      logFrontend('info', `[auto-mode] Disabled`)
    }
  })

  return {
    enabled,
    activeTaskList,
    hasRunningTask,
    isDispatching,
    canDispatch,
  }
}
