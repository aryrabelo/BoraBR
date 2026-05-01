<script setup lang="ts">
import type { UnlistenFn } from '@tauri-apps/api/event'
import type { Issue } from '~/types/issue'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import {
  ClipboardPaste,
  Copy,
  GripHorizontal,
  PanelBottomClose,
  PanelBottomOpen,
  Plus,
  RotateCcw,
  SquareTerminal,
  Trash2,
  X,
} from 'lucide-vue-next'
import { Button } from '~/components/ui/button'
import {
  closeTerminal,
  createTerminalSession,
  isTerminalAvailable,
  onTerminalData,
  onTerminalError,
  onTerminalExit,
  resizeTerminal,
  restartTerminal,
  writeTerminal,
  type TerminalEventPayload,
} from '~/utils/terminal-api'
import { TERMINAL_PANEL_MAX_HEIGHT, TERMINAL_PANEL_MIN_HEIGHT, useTerminalPanel } from '~/composables/useTerminalPanel'
import { resolveTerminalRenderer, type TerminalRendererTarget } from '~/utils/terminal-renderer'
import { buildTerminalHelperCommands, type TerminalHelperCommand } from '~/utils/terminal-helpers'

const props = defineProps<{
  projectPath: string
  projectName?: string
  selectedIssue?: Issue | null
  mode?: 'dock' | 'inline'
  autoStart?: boolean
  rendererTarget?: TerminalRendererTarget
}>()

const emit = defineEmits<{
  closed: []
}>()

const panel = useTerminalPanel({ initialOpen: props.mode === 'inline' })
const outputRef = ref<HTMLElement | null>(null)
const commandInputRef = ref<HTMLInputElement | null>(null)
const commandInput = ref('')
const isCreatingSession = ref(false)
const isResizing = ref(false)
const resizeStartY = ref(0)
const resizeStartHeight = ref(0)
const uiMessage = ref('')
let uiMessageTimer: ReturnType<typeof setTimeout> | null = null
const unlisteners: UnlistenFn[] = []

const isInline = computed(() => props.mode === 'inline')
const currentProjectName = computed(() => props.projectName || projectNameFromPath(props.projectPath))
const activeCanWrite = computed(() => !!panel.activeSession.value?.backendSessionId && panel.activeSession.value.status === 'running')
const renderer = computed(() => resolveTerminalRenderer({ target: props.rendererTarget ?? 'libghostty' }))
const helperCommands = computed(() => buildTerminalHelperCommands(props.selectedIssue ? { id: props.selectedIssue.id, title: props.selectedIssue.title } : null))

function projectNameFromPath(path: string): string {
  const normalized = path.replace(/\\/g, '/').replace(/\/$/, '')
  return normalized.split('/').filter(Boolean).pop() || 'Project'
}

function setUiMessage(message: string) {
  uiMessage.value = message
  if (uiMessageTimer) clearTimeout(uiMessageTimer)
  uiMessageTimer = setTimeout(() => {
    uiMessage.value = ''
  }, 4000)
}

function sessionForBackend(backendSessionId: string) {
  return panel.sessions.value.find(session => session.backendSessionId === backendSessionId)
}

function terminalSize() {
  const width = outputRef.value?.clientWidth || 900
  const contentHeight = outputRef.value?.clientHeight || Math.max(panel.height.value - 128, 120)
  return {
    cols: Math.max(40, Math.floor(width / 8)),
    rows: Math.max(8, Math.floor(contentHeight / 18)),
  }
}

function scrollToBottom() {
  nextTick(() => {
    if (!outputRef.value) return
    outputRef.value.scrollTop = outputRef.value.scrollHeight
  })
}

async function resizeActiveBackend() {
  const session = panel.activeSession.value
  if (!session?.backendSessionId || !isTerminalAvailable()) return
  try {
    await resizeTerminal({ sessionId: session.backendSessionId, ...terminalSize() })
  } catch (error) {
    panel.markError(session.id, error instanceof Error ? error.message : String(error))
  }
}

async function createSession() {
  if (!props.projectPath || isCreatingSession.value) return

  isCreatingSession.value = true
  const session = panel.createSession({
    projectPath: props.projectPath,
    projectName: currentProjectName.value,
    issue: props.selectedIssue ? { id: props.selectedIssue.id, title: props.selectedIssue.title } : null,
  })

  try {
    const info = await createTerminalSession({
      cwd: props.projectPath,
      issueId: props.selectedIssue?.id,
      ...terminalSize(),
    })
    panel.markRunning(session.id, info.sessionId)
    await resizeActiveBackend()
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    panel.markError(session.id, message)
    panel.appendOutput(session.id, `${message}\n`)
  } finally {
    isCreatingSession.value = false
    scrollToBottom()
  }
}

async function closeSession(id: string) {
  const session = panel.sessions.value.find(item => item.id === id)
  panel.closeSession(id)
  if (isInline.value && panel.sessions.value.length === 0) {
    emit('closed')
  }
  if (!session?.backendSessionId || !isTerminalAvailable()) return
  try {
    await closeTerminal(session.backendSessionId)
  } catch (error) {
    setUiMessage(error instanceof Error ? error.message : String(error))
  }
}

async function hidePanel() {
  if (isInline.value) {
    const hadSessions = panel.sessions.value.length > 0
    await closeAllSessions()
    if (!hadSessions) emit('closed')
    return
  }
  panel.closePanel()
}

function clearActiveSession() {
  const session = panel.activeSession.value
  if (!session) return
  panel.clearSession(session.id)
}

async function restartActiveSession() {
  const session = panel.activeSession.value
  if (!session?.backendSessionId) return

  panel.restartSession(session.id)
  try {
    const info = await restartTerminal(session.backendSessionId)
    panel.markRunning(session.id, info.sessionId)
    await resizeActiveBackend()
  } catch (error) {
    panel.markError(session.id, error instanceof Error ? error.message : String(error))
  } finally {
    scrollToBottom()
  }
}

async function submitCommand() {
  const session = panel.activeSession.value
  const command = commandInput.value
  if (!session?.backendSessionId || !command.trim()) return

  commandInput.value = ''
  try {
    await writeTerminal({ sessionId: session.backendSessionId, data: `${command}\r` })
  } catch (error) {
    panel.markError(session.id, error instanceof Error ? error.message : String(error))
  }
}

async function copyOutput() {
  const output = panel.activeSession.value?.buffer
  if (!output) return
  try {
    await navigator.clipboard.writeText(output)
    setUiMessage('Copied')
  } catch (error) {
    setUiMessage(error instanceof Error ? error.message : String(error))
  }
}

async function pasteIntoCommand() {
  try {
    commandInput.value += await navigator.clipboard.readText()
  } catch (error) {
    setUiMessage(error instanceof Error ? error.message : String(error))
  }
}

function stageHelperCommand(helper: TerminalHelperCommand) {
  if (helper.id === 'issue-id' && commandInput.value.trim()) {
    commandInput.value = `${commandInput.value.trimEnd()} ${helper.command}`
  } else {
    commandInput.value = helper.command
  }
  nextTick(() => commandInputRef.value?.focus())
}

function handleTerminalData(payload: TerminalEventPayload) {
  const session = sessionForBackend(payload.sessionId)
  if (!session || !payload.data) return
  panel.appendOutput(session.id, payload.data)
  scrollToBottom()
}

function handleTerminalExit(payload: TerminalEventPayload) {
  const session = sessionForBackend(payload.sessionId)
  if (!session) return
  panel.markExited(session.id)
}

function handleTerminalError(payload: TerminalEventPayload) {
  const session = sessionForBackend(payload.sessionId)
  if (!session) return
  panel.markError(session.id, payload.message || 'Terminal error')
}

function startResize(event: MouseEvent) {
  event.preventDefault()
  window.getSelection()?.removeAllRanges()
  isResizing.value = true
  resizeStartY.value = event.clientY
  resizeStartHeight.value = panel.height.value
  document.addEventListener('mousemove', onResize)
  document.addEventListener('mouseup', stopResize)
  document.body.style.cursor = 'row-resize'
  document.body.style.userSelect = 'none'
}

function onResize(event: MouseEvent) {
  if (!isResizing.value) return
  const maxHeight = typeof window === 'undefined'
    ? TERMINAL_PANEL_MAX_HEIGHT
    : Math.min(TERMINAL_PANEL_MAX_HEIGHT, Math.floor(window.innerHeight * 0.55))
  const diff = resizeStartY.value - event.clientY
  panel.setHeight(resizeStartHeight.value + diff, maxHeight)
}

function stopResize() {
  isResizing.value = false
  document.removeEventListener('mousemove', onResize)
  document.removeEventListener('mouseup', stopResize)
  document.body.style.cursor = ''
  document.body.style.userSelect = ''
  resizeActiveBackend()
}

function statusClass(status: string) {
  if (status === 'running') return 'bg-emerald-500'
  if (status === 'starting') return 'bg-amber-500'
  if (status === 'error') return 'bg-destructive'
  return 'bg-muted-foreground'
}

async function closeAllSessions() {
  const sessions = [...panel.sessions.value]
  await Promise.all(sessions.map(session => closeSession(session.id)))
}

watch(() => props.projectPath, async (nextPath, previousPath) => {
  if (!previousPath || nextPath === previousPath) return
  await closeAllSessions()
})

watch(panel.activeSessionId, () => {
  resizeActiveBackend()
  scrollToBottom()
})

onMounted(async () => {
  if (isTerminalAvailable()) {
    unlisteners.push(await onTerminalData(handleTerminalData))
    unlisteners.push(await onTerminalExit(handleTerminalExit))
    unlisteners.push(await onTerminalError(handleTerminalError))
  }
  if (props.autoStart && panel.sessions.value.length === 0) {
    await createSession()
  }
})

onUnmounted(() => {
  if (uiMessageTimer) clearTimeout(uiMessageTimer)
  for (const unlisten of unlisteners) unlisten()
  for (const session of panel.sessions.value) {
    if (session.backendSessionId && isTerminalAvailable()) {
      closeTerminal(session.backendSessionId).catch(() => {})
    }
  }
})
</script>

<template>
  <section
    v-if="panel.isOpen.value"
    class="relative shrink-0 border-t border-border bg-card text-card-foreground flex flex-col overflow-hidden"
    :data-renderer-target="renderer.target"
    :data-renderer-active="renderer.active"
    :style="{ height: `${panel.height.value}px`, minHeight: `${TERMINAL_PANEL_MIN_HEIGHT}px` }"
  >
    <button
      type="button"
      class="absolute left-0 right-0 top-0 z-10 flex h-2 cursor-row-resize items-center justify-center text-muted-foreground hover:bg-primary/20"
      title="Resize terminal"
      @mousedown="startResize"
    >
      <GripHorizontal class="h-3 w-3" />
    </button>

    <div class="flex h-11 shrink-0 items-center gap-2 border-b border-border bg-muted/30 px-3 pt-1">
      <div class="flex min-w-0 flex-1 items-center gap-2">
        <SquareTerminal class="h-4 w-4 shrink-0 text-muted-foreground" />

        <div class="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
          <div
            v-for="session in panel.sessions.value"
            :key="session.id"
            role="button"
            tabindex="0"
            class="group flex h-8 max-w-56 shrink-0 items-center gap-2 rounded-md border px-2 text-left text-xs transition-colors"
            :class="session.id === panel.activeSessionId.value ? 'border-primary bg-background text-foreground' : 'border-transparent text-muted-foreground hover:bg-background/70 hover:text-foreground'"
            :title="session.issueTitle || session.label"
            @click="panel.setActiveSession(session.id)"
            @keydown.enter.prevent="panel.setActiveSession(session.id)"
            @keydown.space.prevent="panel.setActiveSession(session.id)"
          >
            <span class="h-2 w-2 rounded-full" :class="statusClass(session.status)" />
            <span class="truncate">{{ session.label }}</span>
            <button
              type="button"
              class="rounded p-0.5 opacity-60 hover:bg-muted hover:opacity-100"
              title="Close session"
              @click.stop="closeSession(session.id)"
            >
              <X class="h-3 w-3" />
            </button>
          </div>
        </div>
      </div>

      <div class="flex shrink-0 items-center gap-1">
        <Button variant="ghost" size="icon-sm" title="New session" :disabled="isCreatingSession || !props.projectPath" @click="createSession">
          <Plus class="h-4 w-4" />
        </Button>
        <Button variant="ghost" size="icon-sm" title="Restart session" :disabled="!panel.activeSession.value?.backendSessionId" @click="restartActiveSession">
          <RotateCcw class="h-4 w-4" />
        </Button>
        <Button variant="ghost" size="icon-sm" title="Clear output" :disabled="!panel.activeSession.value" @click="clearActiveSession">
          <Trash2 class="h-4 w-4" />
        </Button>
        <Button variant="ghost" size="icon-sm" title="Copy output" :disabled="!panel.activeSession.value?.buffer" @click="copyOutput">
          <Copy class="h-4 w-4" />
        </Button>
        <Button variant="ghost" size="icon-sm" title="Paste" :disabled="!panel.activeSession.value" @click="pasteIntoCommand">
          <ClipboardPaste class="h-4 w-4" />
        </Button>
        <Button variant="ghost" size="icon-sm" title="Hide terminal" @click="hidePanel">
          <PanelBottomClose class="h-4 w-4" />
        </Button>
      </div>
    </div>

    <div v-if="panel.activeSession.value" class="flex min-h-0 flex-1 flex-col bg-zinc-950 text-zinc-100">
      <div class="flex h-7 shrink-0 items-center gap-3 border-b border-white/10 px-3 text-[11px] text-zinc-400">
        <span class="truncate">{{ panel.activeSession.value.projectPath }}</span>
        <span v-if="panel.activeSession.value.issueId" class="shrink-0 text-zinc-500">{{ panel.activeSession.value.issueId }}</span>
        <span class="ml-auto shrink-0 capitalize">{{ panel.activeSession.value.status }}</span>
      </div>

      <pre
        ref="outputRef"
        class="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words px-3 py-2 font-mono text-xs leading-5"
      >{{ panel.activeSession.value.buffer }}</pre>

      <div class="flex h-9 shrink-0 items-center gap-1 overflow-x-auto border-t border-white/10 px-3">
        <button
          v-for="helper in helperCommands"
          :key="helper.id"
          type="button"
          class="shrink-0 rounded border border-white/10 px-2 py-1 text-[11px] text-zinc-300 transition-colors hover:border-emerald-400/40 hover:text-emerald-300"
          :title="helper.title"
          @click="stageHelperCommand(helper)"
        >
          {{ helper.label }}
        </button>
      </div>

      <form class="flex h-10 shrink-0 items-center gap-2 border-t border-white/10 px-3" @submit.prevent="submitCommand">
        <span class="font-mono text-xs text-emerald-400">$</span>
        <input
          ref="commandInputRef"
          v-model="commandInput"
          class="min-w-0 flex-1 bg-transparent font-mono text-xs text-zinc-100 outline-none placeholder:text-zinc-600"
          :disabled="!activeCanWrite"
          placeholder="Type command"
        >
        <span v-if="uiMessage" class="shrink-0 text-[11px] text-zinc-400">{{ uiMessage }}</span>
      </form>
    </div>

    <div v-else class="flex min-h-0 flex-1 items-center justify-center bg-zinc-950 px-4 text-sm text-zinc-400">
      <Button variant="secondary" size="sm" :disabled="isCreatingSession || !props.projectPath" @click="createSession">
        <Plus class="h-4 w-4" />
        New session
      </Button>
    </div>
  </section>

  <div
    v-else-if="!isInline"
    class="flex h-11 shrink-0 items-center justify-between gap-3 border-t border-border bg-card px-3 text-sm text-muted-foreground"
  >
    <button type="button" class="flex min-w-0 items-center gap-2 hover:text-foreground" @click="panel.openPanel">
      <PanelBottomOpen class="h-4 w-4 shrink-0" />
      <span class="font-medium text-foreground">Terminal</span>
      <span class="truncate text-xs">{{ currentProjectName }}</span>
      <span v-if="panel.sessions.value.length" class="text-xs">{{ panel.sessions.value.length }}</span>
    </button>
    <Button variant="ghost" size="icon-sm" title="New session" :disabled="isCreatingSession || !props.projectPath" @click="createSession">
      <Plus class="h-4 w-4" />
    </Button>
  </div>
</template>
