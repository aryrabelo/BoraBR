import { computed, ref } from 'vue'

export function useTaskTerminalSlots() {
  const openIssueIdSet = ref(new Set<string>())

  const openIssueIds = computed(() => Array.from(openIssueIdSet.value))

  const isIssueTerminalOpen = (issueId: string) => openIssueIdSet.value.has(issueId)

  const openIssueTerminal = (issueId: string) => {
    openIssueIdSet.value = new Set([...openIssueIdSet.value, issueId])
  }

  const closeIssueTerminal = (issueId: string) => {
    const next = new Set(openIssueIdSet.value)
    next.delete(issueId)
    openIssueIdSet.value = next
  }

  const toggleIssueTerminal = (issueId: string) => {
    if (isIssueTerminalOpen(issueId)) {
      closeIssueTerminal(issueId)
    } else {
      openIssueTerminal(issueId)
    }
  }

  return {
    openIssueIds,
    isIssueTerminalOpen,
    openIssueTerminal,
    closeIssueTerminal,
    toggleIssueTerminal,
  }
}
