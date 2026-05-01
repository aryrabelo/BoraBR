import { computed, ref } from 'vue'

export interface CloseIssueTerminalOptions {
  force?: boolean
}

export function useTaskTerminalSlots() {
  const openIssueIdSet = ref(new Set<string>())
  const activeAgentIssueIdSet = ref(new Set<string>())

  const openIssueIds = computed(() => Array.from(openIssueIdSet.value))

  const isIssueTerminalOpen = (issueId: string) => openIssueIdSet.value.has(issueId)
  const isIssueTerminalCloseGuarded = (issueId: string) => activeAgentIssueIdSet.value.has(issueId)

  const openIssueTerminal = (issueId: string) => {
    openIssueIdSet.value = new Set([...openIssueIdSet.value, issueId])
  }

  const closeIssueTerminal = (issueId: string, options: CloseIssueTerminalOptions = {}) => {
    if (isIssueTerminalCloseGuarded(issueId) && !options.force) {
      return false
    }

    const next = new Set(openIssueIdSet.value)
    next.delete(issueId)
    openIssueIdSet.value = next

    const nextActive = new Set(activeAgentIssueIdSet.value)
    nextActive.delete(issueId)
    activeAgentIssueIdSet.value = nextActive

    return true
  }

  const toggleIssueTerminal = (issueId: string) => {
    if (isIssueTerminalOpen(issueId)) {
      return closeIssueTerminal(issueId)
    }

    openIssueTerminal(issueId)
    return true
  }

  const setIssueTerminalAgentActive = (issueId: string, active: boolean) => {
    const next = new Set(activeAgentIssueIdSet.value)
    if (active) {
      next.add(issueId)
    } else {
      next.delete(issueId)
    }
    activeAgentIssueIdSet.value = next
  }

  return {
    openIssueIds,
    isIssueTerminalOpen,
    isIssueTerminalCloseGuarded,
    openIssueTerminal,
    closeIssueTerminal,
    setIssueTerminalAgentActive,
    toggleIssueTerminal,
  }
}
