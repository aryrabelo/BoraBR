<script setup lang="ts">
import { SquareTerminal } from 'lucide-vue-next'

defineProps<{
  issueId: string
  active?: boolean
}>()

defineEmits<{
  toggle: [event: MouseEvent]
}>()
</script>

<template>
  <button
    type="button"
    class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border transition-colors"
    :class="active
      ? 'border-primary bg-primary/10 text-primary'
      : 'border-border text-muted-foreground hover:border-primary/60 hover:text-primary'"
    :aria-label="`${active ? 'Close' : 'Open'} task terminal for ${issueId}`"
    :title="active ? 'Close task terminal' : 'Open task terminal'"
    @click.stop="$emit('toggle', $event)"
  >
    <SquareTerminal class="h-3.5 w-3.5" />
  </button>
</template>
