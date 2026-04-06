# Built-in Issue Tracker Engine — Full Specification

## 1. Context & Motivation

### The Problem

The app currently depends on external CLI tools (`bd` or `br`) for all issue management. This creates several pain points:

- **Folder conflict**: Both `bd` (Go) and `br` (Rust) use the same `.beads/` directory. Running both on the same project corrupts data. The `br` maintainer refused to make the folder name configurable, citing too many implications.
- **Symlink workaround rejected**: Symlinks were suggested but are not portable (Windows compatibility) and fragile.
- **CLI overhead**: Every CRUD operation spawns a child process (`bd list`, `bd show`, etc.), adding ~50-100ms latency per call. The per-project mutex serializes all calls.
- **Version fragmentation**: bd 0.49.x (SQLite+CGO), bd 0.50-0.56+ (Dolt server mode), br (Rust/SQLite) — three incompatible variants requiring version-gated code paths.
- **Blocked evolution**: Can't add features (task orchestration, workflow engines) without being constrained by upstream decisions.

### The Solution

Replace the CLI dependency with a **built-in SQLite-native engine** embedded directly in the Tauri/Rust backend. The frontend (Vue 3) remains **100% unchanged** — only the backend switches from spawning CLI processes to direct `rusqlite` calls.

### What We Keep

- The **data model** (issues, comments, dependencies, labels) — proven and sufficient
- The **JSONL export format** — for git-based sync and interoperability
- The **attachment system** — already filesystem-only, no dependency on bd/br
- The **frontend API surface** — all Tauri command signatures stay identical

### What We Drop (when using built-in backend)

- CLI process spawning (`Command::new("bd")`)
- Version detection and version-gated behavior
- Daemon management (`--no-daemon`, `daemon.lock`, `daemon.pid`)
- Dolt backend support (`.dolt/` directories)
- `bd sync` as a black box — replaced by transparent git operations

### Backend Selector — User's Choice

The app offers **3 backends** selectable in Settings:

| Backend | Folder | Engine | Status |
|---------|--------|--------|--------|
| **Built-in** (default for new projects) | `.tracker/` | `rusqlite` direct | New |
| **bd 0.49.x** | `.beads/` | CLI spawn → SQLite/Dolt | Legacy, maintained |
| **br** | `.beads/` | CLI spawn → SQLite | Legacy, maintained |

The backend choice is per-project. Existing bd/br projects keep working. The user migrates when ready.

---

## 2. Architecture Overview

### Dual Access: Tauri App + CLI `tracker`

**Decision (confirmed):** The built-in engine is accessible via two entry points:
1. **Tauri app** — direct `rusqlite` calls (fast, no process spawn)
2. **CLI `tracker`** — standalone binary for AI agents (Claude Code, worktree agents, orchestrators)

Both share the **same Rust code** and the **same SQLite database**. The CLI is compiled alongside the app in the same Cargo workspace and shipped inside the app bundle.

```
┌─────────────────────────────────────────────────────────────┐
│  Frontend (Vue 3) — NO CHANGES                             │
│  useIssues, useDashboard, useFilters, etc.                 │
│  bd-api.ts → invoke('bd_*', {...})                         │
└────────────┬────────────────────────────────────────────────┘
             │ Tauri invoke() — same signatures
             ↓
┌─────────────────────────────────────────────────────────────┐
│  Rust Backend                                               │
│                                                             │
│  ┌──────────────────┐    ┌──────────────────┐               │
│  │  Tauri App       │    │  CLI `tracker`   │               │
│  │  (lib.rs)        │    │  (cli/main.rs)   │               │
│  │  GUI entry point │    │  AI entry point  │               │
│  └────────┬─────────┘    └────────┬─────────┘               │
│           │                       │                         │
│           └───────────┬───────────┘                         │
│                       ↓                                     │
│           ┌───────────────────────┐                         │
│           │  tracker::Engine      │  ← shared crate         │
│           │  (tracker/mod.rs)     │                         │
│           └───────────┬───────────┘                         │
│                       │ rusqlite (bundled SQLite)            │
│                       ↓                                     │
│           ┌───────────────────────┐                         │
│           │  .tracker/tracker.db  │                         │
│           └───────────────────────┘                         │
└─────────────────────────────────────────────────────────────┘
```

### CLI `tracker` — AI Interface

```bash
# AI agents use the CLI exactly like they use bd/br today:
tracker create "Fix login bug" --type bug --priority p1
tracker list --open --json
tracker update tracker-a4f2 --status in_progress
tracker close tracker-a4f2
tracker show tracker-a4f2 --json
```

The CLI binary is bundled inside the app:
```
# macOS
/Applications/Beads Task-Issue Tracker.app/Contents/MacOS/tracker

# Linux (.deb)
/usr/bin/tracker

# Windows
C:\Program Files\Beads Task-Issue Tracker\tracker.exe
```

### Cargo Workspace Setup

```toml
# src-tauri/Cargo.toml
[[bin]]
name = "beads-task-issue-tracker"
path = "src/main.rs"

[[bin]]
name = "tracker"
path = "src/cli/main.rs"

[dependencies]
rusqlite = { version = "0.31", features = ["bundled"] }
```

Both binaries link the same `tracker::Engine` crate. Same compilation, same CI, same target platform — no portability issue.

### SQLite: Not a Server

SQLite is a **library** compiled into the binary, not a server process. There is nothing running in the background. Each access (Tauri app or CLI) opens the file, reads/writes, and closes. WAL mode handles concurrent access safely.

The database file (`.tracker/tracker.db`) is **local-only** — it is never committed to git. The JSONL export is the source of truth for git sync. If the DB is deleted, it is rebuilt from JSONL on next open.

### Concurrency Model

| Scenario | Risk | Handling |
|----------|------|----------|
| Multiple projects in parallel | None | Separate DB files per project |
| Multiple agents + worktrees | None | Each worktree has its own DB, merge via JSONL in git |
| Multiple agents + same folder | Quasi-nil | WAL mode + `busy_timeout(5s)`, writes take ~1ms |
| NFS/network filesystem | Same as bd/br | SQLite locking unreliable on NFS — local-only use |

### Project Directory Layout

```
.tracker/
├── tracker.db         (SQLite — .gitignored, local cache)
├── tracker.db-wal     (WAL — .gitignored)
├── issues.jsonl       (git-tracked, source of truth for sync)
├── config.yaml        (git-tracked, project settings)
├── attachments/{id}/  (git-tracked, files)
└── .gitignore         (excludes DB files)
```

### Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Folder name | `.tracker/` (configurable) | No conflict with `.beads/`, clear purpose |
| DB engine | `rusqlite` + `bundled` | Real SQLite C (25 years mature), not fsqlite (too young) |
| AI access | CLI `tracker` binary | Same UX as bd/br for agents, no app dependency |
| CLI distribution | Bundled in app | Same build, same CI, no separate portability problem |
| Backend selector | Settings → built-in / bd / br | User chooses per project, migration when ready |
| ID format | `{prefix}-{base36}` | Compatible with existing IDs, prefix from config |
| Sync format | JSONL | Git-friendly, one line per issue, proven by bd/br |
| API surface | Same Tauri commands | Frontend sees zero difference |

---

## 3. Database Schema

### Table: `issues`

```sql
CREATE TABLE issues (
    id              TEXT PRIMARY KEY,
    title           TEXT NOT NULL,
    description     TEXT DEFAULT '',
    status          TEXT NOT NULL DEFAULT 'open'
                    CHECK(status IN ('open','in_progress','blocked','closed','deferred')),
    priority        INTEGER NOT NULL DEFAULT 2
                    CHECK(priority BETWEEN 0 AND 4),
    issue_type      TEXT NOT NULL DEFAULT 'task'
                    CHECK(issue_type IN ('bug','task','feature','epic','chore')),
    assignee        TEXT,
    external_ref    TEXT UNIQUE,
    estimate        INTEGER,              -- minutes
    design          TEXT,
    acceptance_criteria TEXT,
    notes           TEXT,
    parent_id       TEXT REFERENCES issues(id) ON DELETE SET NULL,
    metadata        TEXT,                  -- JSON blob
    spec_id         TEXT,
    created_by      TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    closed_at       TEXT
);

CREATE INDEX idx_issues_status ON issues(status);
CREATE INDEX idx_issues_parent ON issues(parent_id);
CREATE INDEX idx_issues_type ON issues(issue_type);
```

### Table: `comments`

```sql
CREATE TABLE comments (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    issue_id    TEXT NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    author      TEXT NOT NULL,
    content     TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_comments_issue ON comments(issue_id);
```

### Table: `labels`

```sql
CREATE TABLE labels (
    issue_id    TEXT NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    label       TEXT NOT NULL,
    PRIMARY KEY (issue_id, label)
);

CREATE INDEX idx_labels_label ON labels(label);
```

### Table: `dependencies`

```sql
CREATE TABLE dependencies (
    from_id     TEXT NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    to_id       TEXT NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    rel_type    TEXT NOT NULL DEFAULT 'blocks'
                CHECK(rel_type IN ('blocks','relates-to','duplicates','similar','references')),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (from_id, to_id, rel_type)
);
```

### Table: `schema_version`

```sql
CREATE TABLE schema_version (
    version     INTEGER PRIMARY KEY,
    applied_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
```

---

## 4. Rust Engine API

### Core Module Structure

```
src-tauri/src/
├── lib.rs              (existing — Tauri commands, gradually migrated)
├── tracker/
│   ├── mod.rs          (public API: Engine struct)
│   ├── db.rs           (SQLite connection, migrations, WAL setup)
│   ├── issues.rs       (CRUD operations)
│   ├── comments.rs     (comment operations)
│   ├── deps.rs         (dependency/relation operations)
│   ├── labels.rs       (label operations)
│   ├── search.rs       (full-text search via FTS5)
│   ├── export.rs       (JSONL export/import)
│   ├── ids.rs          (ID generation: prefix-base36)
│   └── config.rs       (project config: prefix, author, etc.)
```

### Engine API (Rust)

```rust
pub struct Engine {
    conn: Connection,       // rusqlite
    config: ProjectConfig,  // from .tracker/config.yaml
}

impl Engine {
    // Lifecycle
    pub fn open(project_path: &Path) -> Result<Self>;
    pub fn init(project_path: &Path, config: ProjectConfig) -> Result<Self>;

    // Issues
    pub fn list(&self, include_closed: bool) -> Result<Vec<Issue>>;
    pub fn show(&self, id: &str) -> Result<Option<Issue>>;
    pub fn create(&self, input: CreateIssue) -> Result<Issue>;
    pub fn update(&self, id: &str, input: UpdateIssue) -> Result<Issue>;
    pub fn close(&self, id: &str) -> Result<Issue>;
    pub fn delete(&self, id: &str, hard: bool) -> Result<()>;
    pub fn search(&self, query: &str) -> Result<Vec<Issue>>;
    pub fn ready(&self) -> Result<Vec<Issue>>;  // open + no unresolved blockers
    pub fn count(&self) -> Result<IssueCounts>;

    // Comments
    pub fn add_comment(&self, issue_id: &str, author: &str, content: &str) -> Result<Comment>;

    // Dependencies
    pub fn add_dep(&self, from: &str, to: &str, rel_type: &str) -> Result<()>;
    pub fn remove_dep(&self, from: &str, to: &str) -> Result<()>;

    // Labels
    pub fn add_label(&self, issue_id: &str, label: &str) -> Result<()>;
    pub fn remove_label(&self, issue_id: &str, label: &str) -> Result<()>;

    // Sync
    pub fn export_jsonl(&self, path: &Path) -> Result<()>;
    pub fn import_jsonl(&self, path: &Path) -> Result<ImportResult>;

    // Status
    pub fn status(&self) -> Result<ProjectStatus>;
}
```

---

## 5. ID Generation

### Format: `{prefix}-{base36}`

- **prefix**: Configurable per project (default: from `config.yaml`, e.g., `"tracker"`)
- **base36**: 4-character random string from `[0-9a-z]` (1.6M combinations)
- **Collision check**: Generate, check existence, retry if taken
- **Examples**: `tracker-a4f2`, `myapp-9kx1`

### Compatibility

The ID format is compatible with existing bd/br IDs (`beads-task-issue-tracker-2ju7`). Migration can preserve existing IDs.

---

## 6. JSONL Sync Format

### Export (DB → JSONL)

```jsonl
{"id":"tracker-a4f2","title":"Fix login bug","status":"open","priority":2,"issue_type":"bug","labels":["auth"],"comments":[{"id":1,"author":"user","content":"...","created_at":"..."}],"dependencies":[{"to_id":"tracker-b3e1","rel_type":"blocks"}],"created_at":"...","updated_at":"..."}
```

Each line is a **complete, self-contained** JSON object with all issue data (comments, labels, dependencies inlined). This makes git diffs readable and merges tractable.

### Import (JSONL → DB)

Strategy: **last-write-wins by `updated_at`**

1. Parse each JSONL line
2. If issue exists in DB and JSONL `updated_at` > DB `updated_at` → update
3. If issue doesn't exist → insert
4. If issue exists in DB but not in JSONL → keep (don't delete, could be local-only)
5. Comments: append-only merge by `(issue_id, author, created_at)` composite key

### Git Integration

```
.tracker/
├── .gitignore          # Contains: tracker.db, tracker.db-wal, tracker.db-shm, daemon.*
├── issues.jsonl        # ← This is committed to git
├── config.yaml         # ← This is committed to git
└── attachments/        # ← This is committed to git
```

Only `issues.jsonl`, `config.yaml`, and `attachments/` are tracked by git. The SQLite database is local-only and rebuilt from JSONL on first open.

---

## 7. Migration Strategy (bd/br → built-in)

### Phase 1: Side-by-side

- Built-in engine uses `.tracker/` folder
- bd/br continues using `.beads/`
- User can choose which backend to use in Settings
- Import tool: read `.beads/issues.jsonl` or `beads.db` → write to `.tracker/`

### Phase 2: Default switch

- New projects default to built-in engine
- Existing projects offer one-click migration
- bd/br support remains for backward compatibility

### Phase 3: Cleanup (optional)

- Remove bd/br CLI code paths
- Simplify version detection, mutex, daemon management
- Drop ~60% of complexity from `lib.rs`

---

## 8. Performance Comparison

| Operation | bd/br CLI (current) | Built-in SQLite (target) |
|-----------|-------------------|------------------------|
| List 100 issues | ~80ms (spawn + parse) | ~2ms (direct query) |
| Show 1 issue | ~50ms (spawn + parse) | ~0.5ms (indexed lookup) |
| Create issue | ~60ms (spawn + write) | ~1ms (INSERT + WAL) |
| Full-text search | ~100ms (spawn + scan) | ~5ms (FTS5 index) |
| Change detection | ~2ms (stat) | ~0.5ms (stat, same) |
| Sync | ~500ms (bd sync) | ~50ms (export JSONL + git) |

**Expected speedup: 10-50x** for most operations.

---

## 9. Implementation Phases

### Phase 1 — MVP: Local Engine (target: 2-3 sessions)

- [ ] `tracker/db.rs` — SQLite connection, schema creation, WAL mode
- [ ] `tracker/ids.rs` — ID generation
- [ ] `tracker/issues.rs` — Full CRUD (list, show, create, update, close, delete)
- [ ] `tracker/comments.rs` — Add/list comments
- [ ] `tracker/deps.rs` — Add/remove dependencies and relations
- [ ] `tracker/labels.rs` — Add/remove labels
- [ ] `tracker/search.rs` — FTS5 full-text search
- [ ] `tracker/export.rs` — JSONL export
- [ ] `tracker/config.rs` — Project config from YAML
- [ ] Wire Tauri commands to Engine (behind feature flag or setting)
- [ ] Settings: backend selector (bd / br / built-in)
- [ ] Claude Code skill for `tracker` CLI — teaches AI agents the available commands, flags, and workflows (equivalent to existing bd/br skills like `run-issue`, `close-issue`, `create-issue`)

### Phase 2 — Sync & Migration (target: 3-5 sessions)

- [ ] `tracker/export.rs` — JSONL import with merge logic
- [ ] Git sync: export → git add/commit/push, git pull → import
- [ ] Migration tool: `.beads/` → `.tracker/` one-click conversion
- [ ] Conflict resolution UI (if needed)

### Phase 3 — Orchestration (future)

- [ ] Task state machine (status transitions with rules)
- [ ] Dependency-aware scheduling (what's ready to work on)
- [ ] Agent distribution via worktrees (link to epic beads-l1m9)
- [ ] Multi-project support

---

## 10. Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| Data loss during migration | Keep `.beads/` intact, migration is copy-only |
| Sync conflicts | Start with local-only (Phase 1), add sync carefully |
| Schema evolution | `schema_version` table + migration scripts |
| FTS5 not available | Bundled SQLite via `rusqlite` always includes FTS5 |
| Concurrent access | WAL mode handles concurrent reads; single-writer is fine for desktop |

---

## 11. Dependencies (Rust crates)

| Crate | Purpose | Status in project |
|-------|---------|-------------------|
| `rusqlite` (+ `bundled` feature) | SQLite access | New dependency |
| `serde` / `serde_json` | JSON serialization | Already used |
| `chrono` | Timestamp handling | Already used |
| `rand` | ID generation | Already used |
| `serde_yaml` | Config parsing | May already exist |

---

## 12. Success Criteria

- [ ] All 53 existing Tauri commands work identically with built-in engine
- [ ] Frontend requires zero changes
- [ ] Performance: list 100 issues < 5ms
- [ ] JSONL export is compatible with existing bd/br format
- [ ] Migration from `.beads/` preserves all data (issues, comments, labels, deps, attachments)
- [ ] Folder name (`.tracker/`) is configurable
