# Terminal Workflow Contracts

## Purpose

Task terminals should move toward `br`-enforced workflow contracts instead of
treating BoraBR's `in_review` state as the durable review policy.

## UI Path

- Detect `workflow:<id>` labels from the issue, inherited epic state, or project
  state when `br` exposes those sources.
- Show workflow state from `br workflow check <id>` output when available.
- Render `next_commands` from `br` as terminal-staged commands.
- Keep `in_review` as a backwards-compatible legacy display state.
- Do not create workflow steps locally from a label. If setup is missing, stage
  `br workflow check <id>` and let `br` return the next deterministic command.

## Current Bridge

The current installed `br` does not expose `br workflow` yet. BoraBR therefore
keeps the contract UI path forward-compatible: labels and future check output
are parsed, workflow helper commands are staged, and policy remains owned by
`br`.
