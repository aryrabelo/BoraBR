import { detectWorkflowContractLabels } from '~/utils/workflow-contracts'

export interface TerminalHelperIssue {
  id: string
  title?: string
  status?: string
  labels?: string[]
}

export interface TerminalHelperCommand {
  id: string
  label: string
  command: string
  title: string
}

export function shellQuote(value: string): string {
  return `'${value.replace(/'/g, `'\"'\"'`)}'`
}

export function buildTerminalHelperCommands(issue?: TerminalHelperIssue | null): TerminalHelperCommand[] {
  const projectCommands: TerminalHelperCommand[] = [
    {
      id: 'create',
      label: 'Create',
      command: 'br create "New issue title" --type task --priority p2',
      title: 'Stage a new issue command',
    },
    {
      id: 'ready',
      label: 'Ready',
      command: 'br ready',
      title: 'List ready Beads work',
    },
    {
      id: 'sync',
      label: 'Sync',
      command: 'br sync',
      title: 'Sync Beads JSONL state',
    },
  ]

  if (!issue?.id) return projectCommands

  const hasWorkflowContract = detectWorkflowContractLabels(issue.labels ?? []).length > 0

  const workflowCommands: TerminalHelperCommand[] = hasWorkflowContract
    ? [
        {
          id: 'workflow-check',
          label: 'Workflow',
          command: `br workflow check ${issue.id}`,
          title: 'Check workflow contract state for selected issue',
        },
        {
          id: 'workflow-steps',
          label: 'Steps',
          command: `br workflow steps ${issue.id} --apply`,
          title: 'Apply deterministic workflow steps returned by br',
        },
        {
          id: 'workflow-next',
          label: 'Next',
          command: `br workflow next ${issue.id}`,
          title: 'Stage next deterministic workflow command',
        },
      ]
    : []

  const issueCommands: TerminalHelperCommand[] = [
    {
      id: 'issue-id',
      label: 'ID',
      command: issue.id,
      title: issue.title ? `Insert ${issue.id}: ${issue.title}` : `Insert ${issue.id}`,
    },
    {
      id: 'show',
      label: 'Show',
      command: `br show ${issue.id}`,
      title: 'Show selected issue',
    },
    ...workflowCommands,
    {
      id: 'start',
      label: 'Start',
      command: `br update ${issue.id} --status in_progress`,
      title: 'Mark selected issue in progress',
    },
    {
      id: 'review-start',
      label: hasWorkflowContract ? 'Legacy Review' : 'Review',
      command: `br update ${issue.id} --status in_review && br comments add ${issue.id} --message "review:started {\\"tool\\":\\"codex\\",\\"started_at\\":\\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\\"}"`,
      title: hasWorkflowContract ? 'Legacy in_review path; prefer br workflow commands' : 'Mark selected issue in review and record review start',
    },
    {
      id: 'review-fail',
      label: 'Changes',
      command: `br update ${issue.id} --status open --assignee "" --add-label "review:changes_requested" && br comments add ${issue.id} --message "review:failed {\\"reason\\":\\"Changes requested\\",\\"at\\":\\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\\"}"`,
      title: 'Reopen selected issue with review changes requested',
    },
    {
      id: 'review-pass',
      label: 'Pass',
      command: `br comments add ${issue.id} --message "review:passed {\\"summary\\":\\"No actionable findings\\",\\"at\\":\\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\\"}"`,
      title: 'Record a passing review comment',
    },
    {
      id: 'review-question',
      label: 'Question',
      command: `br update ${issue.id} --status blocked --add-label "blocked:needs_answer" && br comments add ${issue.id} --message "review:needs_answer {\\"question\\":\\"Question for human\\",\\"at\\":\\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\\"}"`,
      title: 'Block selected issue with a human review question',
    },
    {
      id: 'close',
      label: 'Close',
      command: `br close ${issue.id}`,
      title: 'Close selected issue',
    },
    {
      id: 'comment',
      label: 'Comment',
      command: `br comments add ${issue.id} --message "Comment"`,
      title: 'Add a comment to selected issue',
    },
    {
      id: 'label',
      label: 'Label',
      command: `br update ${issue.id} --add-label "label"`,
      title: 'Set labels on selected issue',
    },
    {
      id: 'blocker',
      label: 'Blocker',
      command: `br dep add ${issue.id} <blocked-by-id>`,
      title: 'Add a blocking dependency',
    },
  ]

  return [...issueCommands, ...projectCommands]
}
