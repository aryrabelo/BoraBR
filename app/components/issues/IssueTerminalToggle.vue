<script setup lang="ts">
import { SquareTerminal } from 'lucide-vue-next'
import { computed } from 'vue'
import { resolveTaskTerminalToggleState } from '~/utils/task-terminal-lifecycle'

const props = defineProps<{
  issueId: string
  active?: boolean
  closeGuarded?: boolean
}>()

defineEmits<{
  toggle: [event: MouseEvent]
}>()

const toggleState = computed(() => resolveTaskTerminalToggleState({
  issueId: props.issueId,
  active: props.active,
  closeGuarded: props.closeGuarded,
}))
</script>

<template>
  <button
    type="button"
    class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border transition-colors disabled:cursor-not-allowed disabled:opacity-60"
    :class="active
      ? 'border-primary bg-primary/10 text-primary'
      : 'border-border text-muted-foreground hover:border-primary/60 hover:text-primary'"
    :aria-label="toggleState.ariaLabel"
    :disabled="toggleState.disabled"
    :title="toggleState.title"
    @click.stop="$emit('toggle', $event)"
  >
    <SquareTerminal class="h-3.5 w-3.5" />
  </button>
</template>
