export interface TerminalHelperIssue {
  id: string
  title?: string
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
    {
      id: 'start',
      label: 'Start',
      command: `br update ${issue.id} --status in_progress`,
      title: 'Mark selected issue in progress',
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
      command: `br update ${issue.id} --labels "label"`,
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
