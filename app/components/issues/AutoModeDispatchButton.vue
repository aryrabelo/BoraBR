<script setup lang="ts">
import { Play, Pause, Loader2 } from 'lucide-vue-next'
import { computed } from 'vue'

const props = defineProps<{
  issueId: string
  issueStatus: string
  dispatching?: boolean
  running?: boolean
}>()

defineEmits<{
  dispatch: [event: MouseEvent]
  pause: [event: MouseEvent]
}>()

const state = computed(() => {
  if (props.dispatching) return 'dispatching'
  if (props.running) return 'running'
  if (props.issueStatus === 'open') return 'ready'
  return 'unavailable'
})

const ariaLabel = computed(() => {
  switch (state.value) {
    case 'dispatching': return 'Dispatching task...'
    case 'running': return 'Pause auto-mode task'
    case 'ready': return 'Dispatch task to cmux'
    default: return 'Task not dispatchable'
  }
})
</script>

<template>
  <button
    v-if="state !== 'unavailable'"
    type="button"
    class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border transition-colors disabled:cursor-not-allowed disabled:opacity-60"
    :class="{
      'border-green-500/60 bg-green-500/10 text-green-600 dark:text-green-400': state === 'running',
      'border-yellow-500/60 bg-yellow-500/10 text-yellow-600': state === 'dispatching',
      'border-border text-muted-foreground hover:border-green-500/60 hover:text-green-600 dark:hover:text-green-400': state === 'ready',
    }"
    :aria-label="ariaLabel"
    :disabled="state === 'dispatching'"
    @click.stop="state === 'running' ? $emit('pause', $event) : $emit('dispatch', $event)"
  >
    <Loader2 v-if="state === 'dispatching'" class="h-3.5 w-3.5 animate-spin" />
    <Pause v-else-if="state === 'running'" class="h-3.5 w-3.5" />
    <Play v-else class="h-3.5 w-3.5" />
  </button>
</template>
