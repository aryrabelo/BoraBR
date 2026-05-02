<script setup lang="ts">
import { Checkbox } from '~/components/ui/checkbox'
import { Label } from '~/components/ui/label'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '~/components/ui/tooltip'

const enabled = useProjectStorage('autoMode', false)

function toggle() {
  enabled.value = !enabled.value
}
</script>

<template>
  <div class="flex items-center gap-1.5">
    <Tooltip>
      <TooltipTrigger as-child>
        <button
          class="flex items-center gap-1.5 px-1.5 py-0.5 rounded text-xs transition-colors"
          :class="enabled ? 'bg-green-500/15 text-green-600 dark:text-green-400' : 'text-muted-foreground hover:text-foreground'"
          @click="toggle"
        >
          <span class="relative flex h-2 w-2">
            <span
              v-if="enabled"
              class="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75"
            />
            <span
              class="relative inline-flex rounded-full h-2 w-2"
              :class="enabled ? 'bg-green-500' : 'bg-muted-foreground/40'"
            />
          </span>
          <span class="font-medium">Auto</span>
        </button>
      </TooltipTrigger>
      <TooltipContent side="bottom">
        <p>{{ enabled ? 'Auto-mode ON: picks tasks and dispatches to cmux' : 'Enable auto-mode to auto-dispatch tasks via cmux' }}</p>
      </TooltipContent>
    </Tooltip>
  </div>
</template>
