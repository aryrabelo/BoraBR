import type { Issue } from '~/types/issue'

export type WorkflowContractSource = 'issue' | 'inherited' | 'project'
export type WorkflowContractKind =
  | 'none'
  | 'uninitialized'
  | 'steps_missing'
  | 'step_ready'
  | 'step_running'
  | 'blocked'
  | 'passed'
  | 'legacy_in_review'

export interface WorkflowContract {
  id: string
  label: string
  source: WorkflowContractSource
}

export interface WorkflowCheckResult {
  kind: Exclude<WorkflowContractKind, 'none' | 'legacy_in_review'>
  workflowId?: string
  nextCommands: string[]
}

export interface WorkflowContractState {
  kind: WorkflowContractKind
  label: string
  workflowId?: string
  contracts: WorkflowContract[]
  nextCommands: string[]
  inferredPolicy: boolean
}

export interface ResolveWorkflowContractOptions {
  inheritedLabels?: string[]
  projectLabels?: string[]
  check?: WorkflowCheckResult | null
}

const WORKFLOW_LABEL_PREFIX = 'workflow:'
const VALID_CHECK_STATES = new Set<WorkflowCheckResult['kind']>([
  'uninitialized',
  'steps_missing',
  'step_ready',
  'step_running',
  'blocked',
  'passed',
])

const STATE_LABELS: Record<WorkflowContractKind, string> = {
  none: 'No workflow',
  uninitialized: 'Workflow setup',
  steps_missing: 'Steps missing',
  step_ready: 'Step ready',
  step_running: 'Step running',
  blocked: 'Workflow blocked',
  passed: 'Workflow passed',
  legacy_in_review: 'Legacy review',
}

export function detectWorkflowContractLabels(labels: string[], source: WorkflowContractSource = 'issue'): WorkflowContract[] {
  return labels
    .map(label => label.trim())
    .filter(label => label.toLowerCase().startsWith(WORKFLOW_LABEL_PREFIX) && label.length > WORKFLOW_LABEL_PREFIX.length)
    .map(label => ({
      id: label.slice(WORKFLOW_LABEL_PREFIX.length),
      label,
      source,
    }))
}

export function detectWorkflowContracts(issue: Issue, options: ResolveWorkflowContractOptions = {}): WorkflowContract[] {
  return [
    ...detectWorkflowContractLabels(issue.labels ?? [], 'issue'),
    ...detectWorkflowContractLabels(options.inheritedLabels ?? [], 'inherited'),
    ...detectWorkflowContractLabels(options.projectLabels ?? [], 'project'),
  ]
}

function parseJsonObject(input: string | Record<string, unknown>): Record<string, unknown> | null {
  if (typeof input !== 'string') return input && typeof input === 'object' ? input : null
  try {
    const parsed = JSON.parse(input)
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as Record<string, unknown> : null
  } catch {
    return null
  }
}

function stringValue(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value : undefined
}

function stringArrayValue(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string' && item.trim().length > 0) : []
}

export function parseWorkflowCheckOutput(input: string | Record<string, unknown>): WorkflowCheckResult | null {
  const parsed = parseJsonObject(input)
  if (!parsed) return null

  const rawState = stringValue(parsed.state) ?? stringValue(parsed.kind) ?? stringValue(parsed.status)
  if (!rawState || !VALID_CHECK_STATES.has(rawState as WorkflowCheckResult['kind'])) return null

  return {
    kind: rawState as WorkflowCheckResult['kind'],
    workflowId: stringValue(parsed.workflow_id) ?? stringValue(parsed.workflowId),
    nextCommands: stringArrayValue(parsed.next_commands).concat(stringArrayValue(parsed.nextCommands)),
  }
}

export function resolveWorkflowContractState(issue: Issue, options: ResolveWorkflowContractOptions = {}): WorkflowContractState {
  const contracts = detectWorkflowContracts(issue, options)

  if (options.check) {
    return {
      kind: options.check.kind,
      label: STATE_LABELS[options.check.kind],
      workflowId: options.check.workflowId ?? contracts[0]?.id,
      contracts,
      nextCommands: options.check.nextCommands,
      inferredPolicy: false,
    }
  }

  if (contracts.length > 0) {
    return {
      kind: 'uninitialized',
      label: STATE_LABELS.uninitialized,
      workflowId: contracts[0]?.id,
      contracts,
      nextCommands: [`br workflow check ${issue.id}`],
      inferredPolicy: false,
    }
  }

  if (issue.status === 'in_review') {
    return {
      kind: 'legacy_in_review',
      label: STATE_LABELS.legacy_in_review,
      contracts: [],
      nextCommands: [`br workflow check ${issue.id}`],
      inferredPolicy: false,
    }
  }

  return {
    kind: 'none',
    label: STATE_LABELS.none,
    contracts: [],
    nextCommands: [],
    inferredPolicy: false,
  }
}
