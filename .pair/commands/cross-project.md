# Cross-Project Sync

Check associated projects and establish cross-project awareness for the current session. This is the entry point for the cross-project communication protocol described in `.pair/AGENTS.md`.

## Input

$ARGUMENTS - Optional: duration for journal lookback (e.g., `4h`, `1d`, `2d`). Default: `4h`.

## Tasks

1. **Check associations**
   ```bash
   pair associations
   ```
   If no associations exist, inform the user and stop — there is nothing to sync.

2. **Read each associated project's journal**
   For each associated project prefix returned in step 1:
   ```bash
   pair journal --from <prefix> --since <duration>
   ```
   Use the duration from `$ARGUMENTS` (default `4h`).

3. **Analyze and classify entries**
   For each journal entry found, classify it:
   - **Breaking change** — a shared interface (API, schema, config, data format) was modified
   - **Related work** — work in progress that touches a shared boundary
   - **Completion signal** — the other project finished something this project depends on
   - **Informational** — context that's good to know but requires no action
   - **No relevant activity** — nothing in the journal affects this project

4. **Report to the user**
   Present a concise summary, grouped by associated project:

   ```
   ## Cross-project sync (<duration>)

   ### <Project B prefix>
   - [breaking] Changed /items response to paginated format (#42)
     → Our client code at src/api/items.ts needs updating
   - [completion] Export endpoint is live (#38)
     → We can start integration work

   ### <Project C prefix>
   - No relevant activity
   ```

   **Rules for the report:**
   - Only flag entries that cross the project boundary (affect shared interfaces, contracts, or dependencies)
   - Skip routine internal entries from the other project (refactoring, UI tweaks, internal bugs)
   - For breaking changes, suggest which files or areas in the current project may be impacted
   - If there are `reply-to` entries that reference this project, highlight them — it means the other agent responded to something we wrote

5. **Set session context**
   If breaking changes or completion signals were found, remind yourself of the implications before starting work. For example:
   - "Before modifying the API client, check if the /items response format still matches our types"
   - "The export feature is now available — integration tasks are unblocked"

   If no relevant activity was found, say so briefly and move on.

## Output Format

```
## Cross-project sync (4h)

### project-b
- [breaking] Description of change (#id)
  → Impact on current project
- [completion] Description (#id)
  → What this unblocks

### project-c
- No relevant activity

---
Ready to work. Key consideration: [one-line summary of the most important cross-project impact, or "no cross-project impact detected"].
```

## Example Usage

```
# Default lookback (4h)
/cross-project

# Check last 24 hours
/cross-project 1d

# Check last 2 days (e.g., after a weekend)
/cross-project 2d
```
