<script setup lang="ts">
import type { Issue } from '~/types/issue'
import { getReviewWorkflowState } from '~/utils/review-workflow'

const props = defineProps<{
  issue: Issue
}>()

const reviewState = computed(() => getReviewWorkflowState(props.issue))

const stateClass = computed(() => {
  switch (reviewState.value.kind) {
    case 'queued':
      return 'border-slate-400/40 text-slate-300 bg-slate-500/10'
    case 'running':
      return 'border-emerald-400/40 text-emerald-300 bg-emerald-500/10'
    case 'stale':
      return 'border-amber-400/40 text-amber-300 bg-amber-500/10'
    case 'unknown':
      return 'border-zinc-400/40 text-zinc-300 bg-zinc-500/10'
    default:
      return 'border-border text-muted-foreground bg-muted/20'
  }
})
</script>

<template>
  <span
    class="inline-flex h-5 items-center rounded border px-1.5 text-[10px] font-medium uppercase leading-none"
    :class="stateClass"
    :title="reviewState.tool || reviewState.pid ? `${reviewState.label}${reviewState.tool ? ` via ${reviewState.tool}` : ''}${reviewState.pid ? ` pid ${reviewState.pid}` : ''}` : reviewState.label"
  >
    {{ reviewState.label }}
  </span>
</template>
