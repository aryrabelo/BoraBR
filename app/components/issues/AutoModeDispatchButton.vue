<script setup lang="ts">
import { Play, X, Loader2 } from 'lucide-vue-next'
import { computed } from 'vue'

const props = defineProps<{
  issueId: string
  issueStatus: string
  dispatching?: boolean
  running?: boolean
}>()

defineEmits<{
  dispatch: [event: MouseEvent]
  cancel: [event: MouseEvent]
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
    case 'running': return 'Cancel auto-mode task'
    case 'ready': return 'Dispatch task to cmux'
    default: return 'Task not dispatchable'
  }
})
</script>

<template>
  <div v-if="state !== 'unavailable'" class="inline-flex gap-0.5">
    <button
      v-if="state === 'ready'"
      type="button"
      class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border transition-colors border-border text-muted-foreground hover:border-green-500/60 hover:text-green-600 dark:hover:text-green-400"
      :aria-label="ariaLabel"
      @click.stop="$emit('dispatch', $event)"
    >
      <Play class="h-3.5 w-3.5" />
    </button>
    <template v-else>
      <span
        v-if="state === 'dispatching'"
        class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-yellow-500/60 bg-yellow-500/10 text-yellow-600"
      >
        <Loader2 class="h-3.5 w-3.5 animate-spin" />
      </span>
      <span
        v-else
        class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-green-500/60 bg-green-500/10 text-green-600 dark:text-green-400"
      >
        <Loader2 class="h-3.5 w-3.5 animate-spin" />
      </span>
      <button
        type="button"
        class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border transition-colors border-red-500/60 bg-red-500/10 text-red-600 hover:bg-red-500/20 dark:text-red-400"
        aria-label="Cancel auto-mode task"
        @click.stop="$emit('cancel', $event)"
      >
        <X class="h-3.5 w-3.5" />
      </button>
    </template>
  </div>
</template>
