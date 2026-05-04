<script setup lang="ts">
import { Button } from '~/components/ui/button'
import { readAutoModeLog, clearAutoModeLog, type AutoModeLogRecord } from '~/utils/auto-mode-log'

const { beadsPath } = useBeadsPath()

const props = defineProps<{
  isOpen: boolean
}>()

const emit = defineEmits<{
  'update:isOpen': [value: boolean]
}>()

const entries = ref<AutoModeLogRecord[]>([])
const isAutoRefresh = ref(true)
let refreshInterval: ReturnType<typeof setInterval> | null = null

const logContainerRef = ref<HTMLDivElement | null>(null)
const isUserAtBottom = ref(true)
const SCROLL_THRESHOLD = 30

const onScroll = () => {
  if (!logContainerRef.value) return
  const el = logContainerRef.value
  isUserAtBottom.value = el.scrollHeight - el.scrollTop - el.clientHeight < SCROLL_THRESHOLD
}

const panelHeight = ref(220)
const isResizing = ref(false)
const startY = ref(0)
const startHeight = ref(0)
const minHeight = 120
const maxHeightPercent = 0.4

const startResize = (e: MouseEvent) => {
  e.preventDefault()
  isResizing.value = true
  startY.value = e.clientY
  startHeight.value = panelHeight.value
  document.addEventListener('mousemove', onResize)
  document.addEventListener('mouseup', stopResize)
  document.body.style.cursor = 'row-resize'
  document.body.style.userSelect = 'none'
}

const onResize = (e: MouseEvent) => {
  if (!isResizing.value) return
  const maxHeight = window.innerHeight * maxHeightPercent
  const diff = startY.value - e.clientY
  const newHeight = Math.min(Math.max(startHeight.value + diff, minHeight), maxHeight)
  panelHeight.value = newHeight
}

const stopResize = () => {
  isResizing.value = false
  document.removeEventListener('mousemove', onResize)
  document.removeEventListener('mouseup', stopResize)
  document.body.style.cursor = ''
  document.body.style.userSelect = ''
}

const scrollToBottom = () => {
  if (logContainerRef.value) {
    logContainerRef.value.scrollTop = logContainerRef.value.scrollHeight
    isUserAtBottom.value = true
  }
}

const eventTypeColors: Record<string, string> = {
  enabled: 'text-green-400',
  disabled: 'text-slate-400',
  dispatch_start: 'text-blue-400',
  dispatch_success: 'text-green-400',
  dispatch_failed: 'text-red-400',
  review_start: 'text-cyan-400',
  review_failed: 'text-red-400',
  merge_start: 'text-amber-400',
  merge_success: 'text-green-400',
  merge_failed: 'text-red-400',
  task_failed: 'text-red-500',
  watchdog: 'text-purple-400',
}

function formatTime(timestamp: string): string {
  try {
    const d = new Date(timestamp)
    return d.toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' })
  } catch {
    return timestamp
  }
}

function formatDate(timestamp: string): string {
  try {
    const d = new Date(timestamp)
    return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
  } catch {
    return ''
  }
}

const fetchEntries = async () => {
  if (!beadsPath.value) return
  const records = await readAutoModeLog(beadsPath.value, 200)
  entries.value = records
  if (isUserAtBottom.value) {
    nextTick(scrollToBottom)
  }
}

const handleClear = async () => {
  if (!beadsPath.value) return
  await clearAutoModeLog(beadsPath.value)
  entries.value = []
}

function startAutoRefresh() {
  if (refreshInterval) return
  refreshInterval = setInterval(fetchEntries, 5000)
}

function stopAutoRefresh() {
  if (refreshInterval) {
    clearInterval(refreshInterval)
    refreshInterval = null
  }
}

watch(() => props.isOpen, (open) => {
  if (open) {
    fetchEntries()
    if (isAutoRefresh.value) startAutoRefresh()
  } else {
    stopAutoRefresh()
  }
})

onMounted(() => {
  if (props.isOpen) {
    fetchEntries()
    if (isAutoRefresh.value) startAutoRefresh()
  }
})

onUnmounted(() => {
  stopAutoRefresh()
})
</script>

<template>
  <div v-if="isOpen" class="border-t border-border bg-card">
    <!-- Resize handle -->
    <div
      class="h-1 cursor-row-resize hover:bg-primary/20 transition-colors"
      @mousedown="startResize"
    />

    <!-- Header -->
    <div class="flex items-center justify-between px-3 py-1.5 border-b border-border">
      <div class="flex items-center gap-2">
        <span class="text-xs font-semibold text-foreground">Auto-Mode Log</span>
        <span class="text-[10px] text-muted-foreground">{{ entries.length }} entries</span>
      </div>
      <div class="flex items-center gap-1">
        <Button
          variant="ghost"
          size="sm"
          class="h-6 px-2 text-[10px]"
          @click="fetchEntries"
        >
          Refresh
        </Button>
        <Button
          variant="ghost"
          size="sm"
          class="h-6 px-2 text-[10px]"
          @click="handleClear"
        >
          Clear
        </Button>
        <Button
          variant="ghost"
          size="sm"
          class="h-6 w-6 p-0"
          @click="emit('update:isOpen', false)"
        >
          <svg xmlns="http://www.w3.org/2000/svg" class="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 6 6 18" /><path d="m6 6 12 12" />
          </svg>
        </Button>
      </div>
    </div>

    <!-- Log entries -->
    <div
      ref="logContainerRef"
      class="overflow-auto font-mono text-[11px] leading-relaxed px-3 py-1"
      :style="{ height: panelHeight + 'px' }"
      @scroll="onScroll"
    >
      <div v-if="entries.length === 0" class="text-muted-foreground text-center py-4">
        No auto-mode events yet
      </div>
      <div
        v-for="(entry, i) in entries"
        :key="i"
        class="flex gap-2 py-0.5 hover:bg-muted/30 rounded px-1"
      >
        <span class="text-muted-foreground whitespace-nowrap shrink-0">
          {{ formatDate(entry.timestamp) }} {{ formatTime(entry.timestamp) }}
        </span>
        <span
          class="whitespace-nowrap shrink-0 font-medium"
          :class="eventTypeColors[entry.eventType] ?? 'text-foreground'"
        >
          {{ entry.eventType }}
        </span>
        <span v-if="entry.issueId !== '-'" class="text-cyan-500 whitespace-nowrap shrink-0">
          {{ entry.issueId }}
        </span>
        <span class="text-foreground/80 truncate">{{ entry.detail }}</span>
        <span v-if="entry.error" class="text-red-400 truncate">{{ entry.error }}</span>
      </div>
    </div>
  </div>
</template>
