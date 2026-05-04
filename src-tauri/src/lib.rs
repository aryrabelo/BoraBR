use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

mod terminal;

// Global flags for logging
static LOGGING_ENABLED: AtomicBool = AtomicBool::new(false);
static VERBOSE_LOGGING: AtomicBool = AtomicBool::new(false);

// Sync cooldown: skip redundant syncs within 10 seconds
static LAST_SYNC_TIME: Mutex<Option<Instant>> = Mutex::new(None);
const SYNC_COOLDOWN_SECS: u64 = 10;

// Filesystem mtime tracking for change detection (per-project)
static LAST_KNOWN_MTIME: LazyLock<Mutex<HashMap<String, std::time::SystemTime>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// Configurable CLI binary name (default: "bd")
static CLI_BINARY: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new("bd".to_string()));

// Global child process handle for beads-probe
static PROBE_CHILD: LazyLock<Mutex<Option<std::process::Child>>> =
    LazyLock::new(|| Mutex::new(None));

// caffeinate child process — prevents Mac sleep during auto-mode
static CAFFEINATE_CHILD: LazyLock<Mutex<Option<std::process::Child>>> =
    LazyLock::new(|| Mutex::new(None));

// Per-project mutex to prevent concurrent bd/Dolt access.
// bd 0.55 uses embedded Dolt which crashes (SIGSEGV) when two bd processes
// access the same database simultaneously. This serializes all bd calls per project.
static BD_PROJECT_LOCKS: LazyLock<Mutex<HashMap<String, std::sync::Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// Cached CLI client info — detected once on first use
// Stores: (client_type, major, minor, patch)
#[derive(Debug, Clone, Copy, PartialEq)]
enum CliClient {
    Bd,  // Original Go-based beads CLI
    Br,  // beads_rust — frozen at classic SQLite+JSONL architecture, no daemon
    Unknown,
}

static CLI_CLIENT_INFO: LazyLock<Mutex<Option<(CliClient, u32, u32, u32)>>> =
    LazyLock::new(|| Mutex::new(None));
static GITHUB_PR_SIGNAL_CACHE: LazyLock<Mutex<HashMap<String, GitHubPullRequestCacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static GITHUB_ACTION_CENTER_PR_CACHE: LazyLock<Mutex<HashMap<String, GitHubActionCenterPullRequestCacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static LINEAR_ACTION_CENTER_CACHE: LazyLock<Mutex<Option<LinearActionCenterCacheEntry>>> =
    LazyLock::new(|| Mutex::new(None));

// Conditional logging macros
macro_rules! log_info {
    ($($arg:tt)*) => {
        if LOGGING_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            log::info!($($arg)*);
        }
    };
}

macro_rules! log_warn {
    ($($arg:tt)*) => {
        if LOGGING_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            log::warn!($($arg)*);
        }
    };
}

macro_rules! log_error {
    ($($arg:tt)*) => {
        if LOGGING_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            log::error!($($arg)*);
        }
    };
}

macro_rules! log_debug {
    ($($arg:tt)*) => {
        if LOGGING_ENABLED.load(std::sync::atomic::Ordering::Relaxed) && VERBOSE_LOGGING.load(std::sync::atomic::Ordering::Relaxed) {
            log::debug!($($arg)*);
        }
    };
}

// ============================================================================
// Update Checker Types
// ============================================================================

#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    #[serde(rename = "currentVersion")]
    pub current_version: String,
    #[serde(rename = "latestVersion")]
    pub latest_version: String,
    #[serde(rename = "hasUpdate")]
    pub has_update: bool,
    #[serde(rename = "releaseUrl")]
    pub release_url: String,
    #[serde(rename = "downloadUrl")]
    pub download_url: Option<String>,
    pub platform: String,
    #[serde(rename = "releaseNotes")]
    pub release_notes: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
    #[serde(default)]
    body: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProjectWorktreePullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    #[serde(rename = "mergedAt")]
    pub merged_at: Option<String>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActionCenterGitHubPullRequest {
    #[serde(rename = "repoFullName")]
    pub repo_full_name: String,
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub branch: String,
    pub author: String,
    #[serde(rename = "isDraft")]
    pub is_draft: bool,
    #[serde(rename = "reviewState")]
    pub review_state: String,
    pub comments: u64,
    #[serde(rename = "reviewComments")]
    pub review_comments: u64,
    #[serde(rename = "requestedReviewers")]
    pub requested_reviewers: u64,
    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<String>,
    #[serde(rename = "actionTimestamp")]
    pub action_timestamp: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActionCenterGitHubPullRequestResponse {
    #[serde(rename = "projectPath")]
    pub project_path: String,
    #[serde(rename = "repoFullName")]
    pub repo_full_name: Option<String>,
    pub error: Option<String>,
    #[serde(rename = "pullRequests")]
    pub pull_requests: Vec<ActionCenterGitHubPullRequest>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActionCenterLinearIssue {
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub status: String,
    #[serde(rename = "stateType")]
    pub state_type: String,
    #[serde(rename = "isUat")]
    pub is_uat: bool,
    pub assignee: Option<String>,
    pub labels: Vec<String>,
    #[serde(rename = "pullRequestUrls")]
    pub pull_request_urls: Vec<String>,
    #[serde(rename = "unackedComments")]
    pub unacked_comments: usize,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<String>,
    #[serde(rename = "actionTimestamp")]
    pub action_timestamp: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActionCenterLinearIssueResponse {
    #[serde(rename = "teamKey")]
    pub team_key: String,
    pub assignee: Option<String>,
    pub error: Option<String>,
    pub issues: Vec<ActionCenterLinearIssue>,
}

#[derive(Debug, Deserialize)]
struct GitHubPullRequest {
    number: u64,
    title: String,
    #[serde(rename = "html_url")]
    url: String,
    state: String,
    merged_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubRepoPullRequestUser {
    login: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubAuthenticatedUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRepoPullRequestHead {
    #[serde(rename = "ref")]
    ref_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubRepoPullRequest {
    number: u64,
    title: String,
    #[serde(rename = "html_url")]
    url: String,
    state: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    user: Option<GitHubRepoPullRequestUser>,
    #[serde(default)]
    head: Option<GitHubRepoPullRequestHead>,
    #[serde(default)]
    comments: Option<u64>,
    #[serde(default)]
    review_comments: Option<u64>,
    #[serde(default)]
    requested_reviewers: Vec<GitHubRepoPullRequestUser>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubPullRequestReview {
    state: String,
    #[serde(default)]
    user: Option<GitHubRepoPullRequestUser>,
}

#[derive(Debug, Clone)]
struct GitHubPullRequestCacheEntry {
    fetched_at: Instant,
    pull_request: Option<ProjectWorktreePullRequest>,
}

#[derive(Debug, Clone)]
struct GitHubActionCenterPullRequestCacheEntry {
    fetched_at: Instant,
    repo_full_name: String,
    pull_requests: Vec<ActionCenterGitHubPullRequest>,
}

#[derive(Debug, Clone)]
struct LinearActionCenterCacheEntry {
    fetched_at: Instant,
    response: ActionCenterLinearIssueResponse,
}

#[derive(Debug, Clone, Serialize)]
pub struct BdCliUpdateInfo {
    #[serde(rename = "currentVersion")]
    pub current_version: String,
    #[serde(rename = "latestVersion")]
    pub latest_version: String,
    #[serde(rename = "hasUpdate")]
    pub has_update: bool,
    #[serde(rename = "releaseUrl")]
    pub release_url: String,
}

// ============================================================================
// File Watcher (debounced native fs watcher via notify crate)
// ============================================================================

struct WatcherState {
    debouncer: Option<notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>>,
    watched_path: Option<String>,
}

impl Default for WatcherState {
    fn default() -> Self {
        Self {
            debouncer: None,
            watched_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct BeadsChangedPayload {
    path: String,
}

// ============================================================================
// Types
// ============================================================================

/// Dependency relationship as returned by bd CLI
/// Format: {"issue_id": "...", "depends_on_id": "...", "type": "blocks", "created_at": "...", "created_by": "..."}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BdRawDependency {
    pub id: Option<String>,
    pub issue_id: Option<String>,
    pub depends_on_id: Option<String>,
    #[serde(rename = "type", alias = "dependency_type")]
    pub dependency_type: Option<String>,
    pub created_at: Option<String>,
    pub created_by: Option<String>,
}

/// Dependent info (for parent-child relationships with full issue info)
/// Some bd versions may return this format instead
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BdRawDependent {
    pub id: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub priority: Option<i32>,
    pub issue_type: Option<String>,
    pub dependency_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BdRawIssue {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub priority: i32,
    pub issue_type: String,
    pub owner: Option<String>,
    pub assignee: Option<String>,
    pub labels: Option<Vec<String>>,
    pub created_at: String,
    pub created_by: Option<String>,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub close_reason: Option<String>,
    pub blocked_by: Option<Vec<String>>,
    pub blocks: Option<Vec<String>>,
    pub comments: Option<Vec<BdRawComment>>,
    pub external_ref: Option<String>,
    pub estimate: Option<i32>,
    pub design: Option<String>,
    pub acceptance_criteria: Option<String>,
    pub notes: Option<String>,
    pub parent: Option<String>,
    pub dependents: Option<Vec<BdRawDependent>>,
    pub dependencies: Option<Vec<BdRawDependency>>,
    pub dependency_count: Option<i32>,
    pub dependent_count: Option<i32>,
    pub metadata: Option<String>,
    pub spec_id: Option<String>,
    pub comment_count: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BdRawComment {
    pub id: serde_json::Value,
    pub issue_id: Option<String>,
    pub author: String,
    pub text: Option<String>,
    pub content: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(rename = "type")]
    pub issue_type: String,
    pub status: String,
    pub priority: String,
    pub assignee: Option<String>,
    pub labels: Vec<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    #[serde(rename = "closedAt")]
    pub closed_at: Option<String>,
    pub comments: Vec<Comment>,
    #[serde(rename = "blockedBy")]
    pub blocked_by: Option<Vec<String>>,
    pub blocks: Option<Vec<String>>,
    #[serde(rename = "externalRef")]
    pub external_ref: Option<String>,
    #[serde(rename = "estimateMinutes")]
    pub estimate_minutes: Option<i32>,
    #[serde(rename = "designNotes")]
    pub design_notes: Option<String>,
    #[serde(rename = "acceptanceCriteria")]
    pub acceptance_criteria: Option<String>,
    #[serde(rename = "workingNotes")]
    pub working_notes: Option<String>,
    pub parent: Option<ParentIssue>,
    pub children: Option<Vec<ChildIssue>>,
    pub relations: Option<Vec<Relation>>,
    pub metadata: Option<String>,
    #[serde(rename = "specId")]
    pub spec_id: Option<String>,
    #[serde(rename = "commentCount")]
    pub comment_count: Option<i32>,
    #[serde(rename = "dependencyCount")]
    pub dependency_count: Option<i32>,
    #[serde(rename = "dependentCount")]
    pub dependent_count: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub content: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChildIssue {
    pub id: String,
    pub title: String,
    pub status: String,
    pub priority: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ParentIssue {
    pub id: String,
    pub title: String,
    pub status: String,
    pub priority: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Relation {
    pub id: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    #[serde(rename = "relationType")]
    pub relation_type: String,
    pub direction: String, // "dependency" or "dependent"
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CountResult {
    pub count: usize,
    #[serde(rename = "byType")]
    pub by_type: HashMap<String, usize>,
    #[serde(rename = "byPriority")]
    pub by_priority: HashMap<String, usize>,
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "isDirectory")]
    pub is_directory: bool,
    #[serde(rename = "hasBeads")]
    pub has_beads: bool,
    #[serde(rename = "usesDolt")]
    pub uses_dolt: bool,
}

#[derive(Debug, Serialize)]
pub struct PurgeResult {
    #[serde(rename = "deletedCount")]
    pub deleted_count: usize,
    #[serde(rename = "deletedFolders")]
    pub deleted_folders: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FsListResult {
    #[serde(rename = "currentPath")]
    pub current_path: String,
    #[serde(rename = "hasBeads")]
    pub has_beads: bool,
    #[serde(rename = "usesDolt")]
    pub uses_dolt: bool,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProjectWorktree {
    #[serde(rename = "rootPath")]
    pub root_path: String,
    #[serde(rename = "worktreePath")]
    pub worktree_path: String,
    #[serde(rename = "canonicalPath")]
    pub canonical_path: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    #[serde(rename = "repoRemote")]
    pub repo_remote: Option<String>,
    #[serde(rename = "isRoot")]
    pub is_root: bool,
    #[serde(rename = "inclusionReason")]
    pub inclusion_reason: String,
    #[serde(rename = "lastActivityAt")]
    pub last_activity_at: Option<u64>,
    #[serde(rename = "lastActivitySource")]
    pub last_activity_source: Option<String>,
    #[serde(rename = "activityScanLimited")]
    pub activity_scan_limited: bool,
    #[serde(rename = "recentActivityRank")]
    pub recent_activity_rank: Option<usize>,
    #[serde(rename = "pullRequest")]
    pub pull_request: Option<ProjectWorktreePullRequest>,
    #[serde(rename = "prPromoted")]
    pub pr_promoted: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct ParsedGitWorktree {
    path: String,
    branch: Option<String>,
    head: Option<String>,
    prunable: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct ProjectWorktreeCandidate {
    root_path: String,
    worktree_path: String,
    canonical_path: String,
    branch: Option<String>,
    head: Option<String>,
    repo_remote: Option<String>,
    is_root: bool,
    inclusion_reason: String,
    last_activity_at: Option<u64>,
    last_activity_source: Option<String>,
    activity_scan_limited: bool,
    recent_activity_rank: Option<usize>,
    pull_request: Option<ProjectWorktreePullRequest>,
    pr_promoted: bool,
}

// ============================================================================
// Options structs for commands
// ============================================================================

#[derive(Debug, Deserialize, Default)]
pub struct ListOptions {
    pub status: Option<Vec<String>>,
    #[serde(rename = "type")]
    pub issue_type: Option<Vec<String>>,
    pub priority: Option<Vec<String>>,
    pub assignee: Option<String>,
    #[serde(rename = "includeAll")]
    pub include_all: Option<bool>,
    pub cwd: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct CwdOptions {
    pub cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePayload {
    pub title: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub issue_type: Option<String>,
    pub priority: Option<String>,
    pub assignee: Option<String>,
    pub labels: Option<Vec<String>>,
    #[serde(rename = "externalRef")]
    pub external_ref: Option<String>,
    #[serde(rename = "estimateMinutes")]
    pub estimate_minutes: Option<i32>,
    #[serde(rename = "designNotes")]
    pub design_notes: Option<String>,
    #[serde(rename = "acceptanceCriteria")]
    pub acceptance_criteria: Option<String>,
    #[serde(rename = "workingNotes")]
    pub working_notes: Option<String>,
    pub parent: Option<String>, // Parent epic ID for hierarchical child
    #[serde(rename = "specId")]
    pub spec_id: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePayload {
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub issue_type: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    pub assignee: Option<String>,
    pub labels: Option<Vec<String>>,
    #[serde(rename = "externalRef")]
    pub external_ref: Option<String>,
    #[serde(rename = "estimateMinutes")]
    pub estimate_minutes: Option<i32>,
    #[serde(rename = "designNotes")]
    pub design_notes: Option<String>,
    #[serde(rename = "acceptanceCriteria")]
    pub acceptance_criteria: Option<String>,
    #[serde(rename = "workingNotes")]
    pub working_notes: Option<String>,
    pub parent: Option<String>, // Some("") to detach, Some("id") to attach
    pub metadata: Option<String>,
    #[serde(rename = "specId")]
    pub spec_id: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentProcessStatusRequest {
    pub tool: Option<String>,
    pub pid: Option<u32>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentProcessStatusResponse {
    pub tool: Option<String>,
    pub pid: Option<u32>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CmuxFocusSurfaceRequest {
    pub surface: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CmuxFocusSurfaceResponse {
    pub surface: String,
    pub command: String,
    pub stdout: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CmuxSendPromptRequest {
    pub surface: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CmuxSendPromptResponse {
    pub surface: String,
    pub command: String,
    pub stdout: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TerminalNativeRendererCapabilitiesResponse {
    pub libghostty: bool,
    #[serde(rename = "ghosttyExternal")]
    pub ghostty_external: GhosttyExternalBridgeCapability,
}

#[derive(Debug, Clone, Serialize)]
pub struct GhosttyExternalBridgeCapability {
    pub available: bool,
    pub command: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenNativeTerminalRendererRequest {
    pub cwd: String,
    #[serde(rename = "issueId")]
    pub issue_id: Option<String>,
    pub shell: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenNativeTerminalRendererResponse {
    pub renderer: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub command: String,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeTerminalLaunchPlan {
    program: String,
    args: Vec<String>,
}

// ============================================================================
// Helpers
// ============================================================================

fn priority_to_string(priority: i32) -> String {
    let p = if (0..=4).contains(&priority) { priority } else { 3 };
    format!("p{}", p)
}

fn priority_to_number(priority: &str) -> String {
    if let Some(caps) = priority.strip_prefix('p') {
        if caps.len() == 1 && caps.chars().next().unwrap_or('x').is_ascii_digit() {
            return caps.to_string();
        }
    }
    "3".to_string()
}

fn normalize_issue_type(issue_type: &str) -> String {
    let valid_types = ["bug", "plan", "task", "feature", "epic", "chore"];
    if valid_types.contains(&issue_type) {
        issue_type.to_string()
    } else {
        "task".to_string()
    }
}

fn normalize_issue_status(status: &str) -> String {
    let valid_statuses = ["open", "in_progress", "in_review", "blocked", "closed", "deferred", "tombstone", "pinned", "hooked"];
    if valid_statuses.contains(&status) {
        status.to_string()
    } else {
        "open".to_string()
    }
}

fn classify_agent_process_status<F>(
    request: AgentProcessStatusRequest,
    probe_process: F,
) -> AgentProcessStatusResponse
where
    F: FnOnce(u32) -> Option<bool>,
{
    let status = match request.pid {
        Some(pid) => match probe_process(pid) {
            Some(true) => "running",
            Some(false) => "not_running",
            None => "unknown",
        },
        None => "unknown",
    };

    AgentProcessStatusResponse {
        tool: request.tool,
        pid: request.pid,
        session_id: request.session_id,
        status: status.to_string(),
    }
}

fn validate_cmux_surface_id(surface: &str) -> Result<(), String> {
    let value = surface.trim();
    if value.is_empty() {
        return Err("cmux surface id is required".to_string());
    }
    if value.len() > 128 {
        return Err("cmux surface id is too long".to_string());
    }
    let valid = value.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | ':' | '_'));
    if !valid {
        return Err("cmux surface id contains invalid characters".to_string());
    }
    Ok(())
}

fn validate_cmux_prompt(prompt: &str) -> Result<(), String> {
    let value = prompt.trim();
    if value.is_empty() {
        return Err("cmux prompt is required".to_string());
    }
    if value.len() > 4096 {
        return Err("cmux prompt is too long".to_string());
    }
    if value.chars().any(|ch| ch == '\0') {
        return Err("cmux prompt contains invalid characters".to_string());
    }
    Ok(())
}

#[cfg(test)]
fn should_fallback_cmux_focus(stderr: &str) -> bool {
    stderr.contains("Unknown command") || stderr.contains("focus-surface")
}

fn cmux_focus_surface_command(surface: &str) -> Vec<String> {
    vec!["focus-surface".to_string(), "--surface".to_string(), surface.to_string()]
}

fn cmux_focus_surface_rpc_command(surface: &str) -> Vec<String> {
    vec![
        "rpc".to_string(),
        "surface.focus".to_string(),
        format!("{{\"surface_id\":\"{}\"}}", surface),
    ]
}

fn cmux_identify_surface_command(surface: &str) -> Vec<String> {
    vec![
        "identify".to_string(),
        "--surface".to_string(),
        surface.to_string(),
    ]
}

fn cmux_select_workspace_command(workspace: &str) -> Vec<String> {
    vec![
        "select-workspace".to_string(),
        "--workspace".to_string(),
        workspace.to_string(),
    ]
}

fn cmux_focus_surface_fallback_command(surface: &str) -> Vec<String> {
    vec![
        "move-surface".to_string(),
        "--surface".to_string(),
        surface.to_string(),
        "--focus".to_string(),
        "true".to_string(),
    ]
}

fn cmux_send_prompt_command(surface: &str, prompt: &str) -> Vec<String> {
    if surface.starts_with("workspace:") {
        vec![
            "send".to_string(),
            "--workspace".to_string(),
            surface.to_string(),
            format!("{}\\n", prompt.trim()),
        ]
    } else {
        vec![
            "send".to_string(),
            "--surface".to_string(),
            surface.to_string(),
            format!("{}\\n", prompt.trim()),
        ]
    }
}

fn parse_workspace_ref_from_cmux_identify_output(stdout: &str) -> Option<String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(workspace_ref) = value.get("workspace_ref").and_then(|v| v.as_str()) {
            if !workspace_ref.trim().is_empty() {
                return Some(workspace_ref.to_string());
            }
        }
    }

    for line in trimmed.lines() {
        if !line.contains("workspace_ref") {
            continue;
        }

        let key_pos = match line.find("workspace_ref") {
            Some(pos) => pos,
            None => continue,
        };
        let mut value = line[key_pos + "workspace_ref".len()..].trim();
        value = value.trim_start_matches(|c: char| c == ':' || c == '=' || c.is_whitespace());
        if value.is_empty() {
            continue;
        }

        if let Some(first) = value.chars().next() {
            if first == '"' || first == '\'' {
                let value = &value[1..];
                let end = value.find(first).unwrap_or(value.len());
                let parsed = value[..end].trim();
                if !parsed.is_empty() {
                    return Some(parsed.to_string());
                }
                continue;
            }
        }

        let end = value
            .find(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .unwrap_or(value.len());
        let parsed = value[..end].trim();
        if !parsed.is_empty() {
            return Some(parsed.to_string());
        }
    }

    None
}

fn run_cmux(args: &[String]) -> Result<std::process::Output, String> {
    new_command("cmux")
        .args(args)
        .env("PATH", get_extended_path())
        .output()
        .map_err(|e| format!("Failed to run cmux: {}", e))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn native_terminal_shell_script(
    cwd: &str,
    issue_id: Option<&str>,
    shell: &str,
    session_id: &str,
) -> String {
    let mut lines = vec![
        format!("cd {} || exit", shell_single_quote(cwd)),
        format!("export BEADS_PATH={}", shell_single_quote(cwd)),
        format!(
            "export BORABR_TERMINAL_SESSION_ID={}",
            shell_single_quote(session_id)
        ),
    ];
    if let Some(issue_id) = issue_id {
        lines.push(format!(
            "export BORABR_ISSUE_ID={}",
            shell_single_quote(issue_id)
        ));
    }
    lines.push(format!("exec {} -l", shell_single_quote(shell)));
    lines.join("\n")
}

fn build_native_terminal_launch_plan(
    platform: &str,
    macos_app_path: Option<String>,
    cwd: &str,
    issue_id: Option<&str>,
    shell: &str,
    session_id: &str,
) -> Result<NativeTerminalLaunchPlan, String> {
    let script = native_terminal_shell_script(cwd, issue_id, shell, session_id);

    if platform == "macos" {
        let app_path = macos_app_path
            .ok_or_else(|| "Ghostty.app is required to launch a native renderer on macOS".to_string())?;
        return Ok(NativeTerminalLaunchPlan {
            program: "open".to_string(),
            args: vec![
                "-n".to_string(),
                app_path,
                "--args".to_string(),
                "-e".to_string(),
                shell.to_string(),
                "-lc".to_string(),
                script,
            ],
        });
    }

    Ok(NativeTerminalLaunchPlan {
        program: "ghostty".to_string(),
        args: vec![
            "-e".to_string(),
            shell.to_string(),
            "-lc".to_string(),
            script,
        ],
    })
}

fn default_native_shell() -> String {
    if cfg!(target_os = "windows") {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
    }
}

fn native_terminal_session_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("native-term-{}", millis)
}

fn find_ghostty_app() -> Option<String> {
    if let Ok(path) = env::var("BORABR_GHOSTTY_APP_PATH") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }

    let mut candidates = vec![PathBuf::from("/Applications/Ghostty.app")];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("Applications/Ghostty.app"));
    }

    candidates
        .into_iter()
        .find(|path| path.exists())
        .map(|path| path.to_string_lossy().to_string())
}

fn ghostty_cli_available() -> Result<String, String> {
    let output = new_command("ghostty")
        .arg("+version")
        .env("PATH", get_extended_path())
        .output()
        .map_err(|e| format!("Ghostty CLI not found: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn detect_native_terminal_renderer_capabilities() -> TerminalNativeRendererCapabilitiesResponse {
    let ghostty_cli = ghostty_cli_available();
    let ghostty_external = if cfg!(target_os = "macos") {
        match (find_ghostty_app(), ghostty_cli) {
            (Some(_), _) => GhosttyExternalBridgeCapability {
                available: true,
                command: Some("open".to_string()),
                reason: None,
            },
            (None, Ok(version)) => GhosttyExternalBridgeCapability {
                available: false,
                command: Some("ghostty".to_string()),
                reason: Some(format!(
                    "{} is on PATH, but macOS Ghostty renderer launch requires Ghostty.app",
                    version.lines().next().unwrap_or("Ghostty")
                )),
            },
            (None, Err(error)) => GhosttyExternalBridgeCapability {
                available: false,
                command: None,
                reason: Some(error),
            },
        }
    } else {
        match ghostty_cli {
            Ok(_) => GhosttyExternalBridgeCapability {
                available: true,
                command: Some("ghostty".to_string()),
                reason: None,
            },
            Err(error) => GhosttyExternalBridgeCapability {
                available: false,
                command: None,
                reason: Some(error),
            },
        }
    };

    TerminalNativeRendererCapabilitiesResponse {
        libghostty: false,
        ghostty_external,
    }
}

fn validate_native_terminal_cwd(cwd: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(cwd);
    if !path.exists() {
        return Err(format!("Native terminal cwd does not exist: {}", cwd));
    }
    if !path.is_dir() {
        return Err(format!("Native terminal cwd is not a directory: {}", cwd));
    }
    path.canonicalize()
        .map_err(|e| format!("Failed to resolve native terminal cwd {}: {}", cwd, e))
}

fn probe_process_running(pid: u32) -> Option<bool> {
    if pid == 0 {
        return None;
    }

    #[cfg(unix)]
    {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .ok()
            .map(|status| status.success())
    }

    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

fn transform_issue(raw: BdRawIssue) -> Issue {
    // Parent info - dependencies array now contains relationship info, not full issue details
    // For now, we just use the parent ID if available
    let parent = raw.parent.as_ref().map(|parent_id| {
        ParentIssue {
            id: parent_id.clone(),
            title: String::new(), // Not available in dependency format
            status: "open".to_string(),
            priority: "p3".to_string(),
        }
    });

    // Extract children from dependents array (with dependency_type: "parent-child")
    let children: Option<Vec<ChildIssue>> = raw.dependents.as_ref().map(|deps| {
        deps.iter()
            .filter(|d| d.dependency_type.as_deref() == Some("parent-child") && d.id.is_some())
            .map(|c| ChildIssue {
                id: c.id.clone().unwrap_or_default(),
                title: c.title.clone().unwrap_or_default(),
                status: normalize_issue_status(&c.status.clone().unwrap_or_else(|| "open".to_string())),
                priority: priority_to_string(c.priority.unwrap_or(3)),
            })
            .collect()
    }).filter(|v: &Vec<ChildIssue>| !v.is_empty());

    // Extract non-blocking relations (everything except "blocks" and "parent-child")
    let structural_types = ["blocks", "parent-child"];
    let mut relations: Vec<Relation> = Vec::new();
    let mut seen_relations: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

    // From dependencies array (these are issues the current issue depends on)
    if let Some(ref deps) = raw.dependencies {
        for dep in deps {
            if let Some(ref dep_type) = dep.dependency_type {
                if structural_types.contains(&dep_type.as_str()) {
                    continue;
                }
                let id = dep.id.clone().or_else(|| dep.depends_on_id.clone()).unwrap_or_default();
                if id.is_empty() {
                    continue;
                }
                let key = (id.clone(), dep_type.clone());
                if !seen_relations.contains(&key) {
                    seen_relations.insert(key);
                    relations.push(Relation {
                        id,
                        title: String::new(),
                        status: String::new(),
                        priority: String::new(),
                        relation_type: dep_type.clone(),
                        direction: "dependency".to_string(),
                    });
                }
            }
        }
    }

    // From dependents array (these are issues that depend on the current issue — has full metadata)
    if let Some(ref dependents) = raw.dependents {
        for dep in dependents {
            if let Some(ref dep_type) = dep.dependency_type {
                if structural_types.contains(&dep_type.as_str()) {
                    continue;
                }
                let id = dep.id.clone().unwrap_or_default();
                if id.is_empty() {
                    continue;
                }
                let key = (id.clone(), dep_type.clone());
                if seen_relations.contains(&key) {
                    // Replace existing entry from dependencies if this one has more metadata
                    if dep.title.is_some() {
                        if let Some(existing) = relations.iter_mut().find(|r| r.id == id && r.relation_type == *dep_type) {
                            existing.title = dep.title.clone().unwrap_or_default();
                            existing.status = normalize_issue_status(&dep.status.clone().unwrap_or_else(|| "open".to_string()));
                            existing.priority = priority_to_string(dep.priority.unwrap_or(3));
                            existing.direction = "dependent".to_string();
                        }
                    }
                } else {
                    seen_relations.insert(key);
                    relations.push(Relation {
                        id,
                        title: dep.title.clone().unwrap_or_default(),
                        status: normalize_issue_status(&dep.status.clone().unwrap_or_else(|| "open".to_string())),
                        priority: priority_to_string(dep.priority.unwrap_or(3)),
                        relation_type: dep_type.clone(),
                        direction: "dependent".to_string(),
                    });
                }
            }
        }
    }

    // Compute comment_count before consuming raw.comments
    let comment_count = raw.comment_count.or_else(|| {
        raw.comments.as_ref().map(|c| c.len() as i32)
    });

    Issue {
        id: raw.id,
        title: raw.title,
        description: raw.description.unwrap_or_default(),
        issue_type: normalize_issue_type(&raw.issue_type),
        status: normalize_issue_status(&raw.status),
        priority: priority_to_string(raw.priority),
        assignee: raw.assignee,
        labels: raw.labels.unwrap_or_default(),
        created_at: raw.created_at,
        updated_at: raw.updated_at,
        closed_at: raw.closed_at,
        comments: raw.comments.unwrap_or_default().into_iter().map(|c| {
            Comment {
                id: match c.id {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s,
                    _ => "0".to_string(),
                },
                author: c.author,
                content: c.text.or(c.content).unwrap_or_default(),
                created_at: c.created_at,
            }
        }).collect(),
        blocked_by: {
            // Try raw.blocked_by first (if bd ever populates it directly)
            let mut bb = raw.blocked_by.unwrap_or_default();
            // Extract from dependencies array (bd show: objects with dependency_type "blocks" = blockers)
            if let Some(ref deps) = raw.dependencies {
                // bd show format: [{id, dependency_type: "blocks"}] — these block the current issue
                for dep in deps {
                    if let (Some(ref dep_type), Some(ref id)) = (&dep.dependency_type, &dep.id) {
                        if dep_type == "blocks" && !bb.contains(id) {
                            bb.push(id.clone());
                        }
                    }
                    // bd list format: [{issue_id, depends_on_id, type: "blocks"}]
                    if let (Some(ref dep_type), Some(ref depends_on_id), Some(ref _issue_id)) = (&dep.dependency_type, &dep.depends_on_id, &dep.issue_id) {
                        if dep_type == "blocks" && !bb.contains(depends_on_id) {
                            bb.push(depends_on_id.clone());
                        }
                    }
                }
            }
            if bb.is_empty() { None } else { Some(bb) }
        },
        blocks: {
            let mut bl = raw.blocks.unwrap_or_default();
            // Extract from dependents array (bd show: objects with dependency_type "blocks" = issues blocked by current)
            // Filter to only "blocks" type — exclude "parent-child" which are children, not dependencies
            if let Some(ref dependents) = raw.dependents {
                for dep in dependents {
                    if let (Some(ref dep_type), Some(ref id)) = (&dep.dependency_type, &dep.id) {
                        if dep_type == "blocks" && !bl.contains(id) {
                            bl.push(id.clone());
                        }
                    }
                }
            }
            if bl.is_empty() { None } else { Some(bl) }
        },
        external_ref: raw.external_ref,
        estimate_minutes: raw.estimate,
        design_notes: raw.design,
        acceptance_criteria: raw.acceptance_criteria,
        working_notes: raw.notes,
        parent,
        children,
        relations: if relations.is_empty() { None } else { Some(relations) },
        metadata: raw.metadata,
        spec_id: raw.spec_id,
        comment_count,
        dependency_count: raw.dependency_count.or_else(|| {
            raw.dependencies.as_ref().map(|d| d.len() as i32)
        }),
        dependent_count: raw.dependent_count.or_else(|| {
            raw.dependents.as_ref().map(|d| d.len() as i32)
        }),
    }
}

/// Parse issues with tolerance for malformed entries
/// Returns all successfully parsed issues and logs failures
fn parse_issues_tolerant(output: &str, context: &str) -> Result<Vec<BdRawIssue>, String> {
    // First try strict parsing
    if let Ok(issues) = serde_json::from_str::<Vec<BdRawIssue>>(output) {
        return Ok(issues);
    }

    // If strict parsing fails, try tolerant parsing
    log_warn!("[{}] Strict parsing failed, attempting tolerant parsing", context);

    let value: serde_json::Value = serde_json::from_str(output)
        .map_err(|e| {
            log_error!("[{}] JSON is completely invalid: {}", context, e);
            format!("Invalid JSON: {}", e)
        })?;

    // br >= 0.1.30 wraps `list` output in a paginated envelope:
    // {"issues": [...], "total": N, "offset": N, "limit": N, "has_more": bool}
    // Unwrap the envelope if present, otherwise expect a flat array.
    let arr_value;
    let arr = if let Some(obj) = value.as_object() {
        if let Some(issues) = obj.get("issues").and_then(|v| v.as_array()) {
            log_info!("[{}] Unwrapped paginated envelope ({} issues)", context, issues.len());
            arr_value = issues.clone();
            &arr_value
        } else {
            log_error!("[{}] Expected array or envelope with 'issues' key, got object: {:?}", context, obj.keys().collect::<Vec<_>>());
            return Err("Expected JSON array or paginated envelope".to_string());
        }
    } else {
        value.as_array().ok_or_else(|| {
            log_error!("[{}] Expected array, got: {:?}", context, value);
            "Expected JSON array".to_string()
        })?
    };

    let mut issues = Vec::new();
    let mut failed_count = 0;

    for (i, obj) in arr.iter().enumerate() {
        let obj_str = serde_json::to_string(obj).unwrap_or_default();
        match serde_json::from_str::<BdRawIssue>(&obj_str) {
            Ok(issue) => issues.push(issue),
            Err(e) => {
                failed_count += 1;
                let id = obj.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                log_error!("[{}] Skipping issue {} (id={}): {}", context, i, id, e);

                // Log which fields are present/missing
                if let Some(obj_map) = obj.as_object() {
                    let keys: Vec<&str> = obj_map.keys().map(|s| s.as_str()).collect();
                    log_error!("[{}] Issue {} has keys: {:?}", context, i, keys);

                    // Check for common missing required fields
                    let required = ["id", "title", "status", "priority", "issue_type", "created_at", "updated_at"];
                    let missing: Vec<&&str> = required.iter().filter(|k| !keys.contains(*k)).collect();
                    if !missing.is_empty() {
                        log_error!("[{}] Issue {} missing required fields: {:?}", context, i, missing);
                    }
                }
            }
        }
    }

    if failed_count > 0 {
        log_warn!("[{}] Parsed {} issues, skipped {} malformed entries", context, issues.len(), failed_count);
    }

    Ok(issues)
}

pub(crate) fn get_extended_path() -> String {
    let current_path = env::var("PATH").unwrap_or_default();

    #[cfg(target_os = "windows")]
    {
        let userprofile = env::var("USERPROFILE").unwrap_or_default();
        let localappdata = env::var("LOCALAPPDATA").unwrap_or_default();
        let mut extra_paths = vec![
            format!(r"{}\AppData\Local\bin", userprofile),
            format!(r"{}\.local\bin", userprofile),
            format!(r"{}\Programs", localappdata),
        ];
        extra_paths.extend(current_path.split(';').map(String::from));
        extra_paths.join(";")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = env::var("HOME").unwrap_or_default();
        let mut extra_paths = vec![
            "/opt/homebrew/bin".to_string(),
            "/usr/local/bin".to_string(),
            "/usr/bin".to_string(),
            "/bin".to_string(),
            format!("{}/.local/bin", home),
            format!("{}/bin", home),
        ];
        extra_paths.extend(current_path.split(':').map(String::from));
        extra_paths.join(":")
    }
}

/// Creates a Command with platform-specific flags.
/// On Windows, sets CREATE_NO_WINDOW to prevent console popups.
fn new_command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd
}

// ============================================================================
// CLI Binary Configuration
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct AppConfig {
    #[serde(default = "default_cli_binary")]
    cli_binary: String,
}

fn default_cli_binary() -> String {
    // Auto-detect: prefer br (Rust), fallback to bd (Go)
    for bin in &["br", "bd"] {
        if let Ok(output) = std::process::Command::new(bin)
            .arg("--version")
            .current_dir(std::env::temp_dir())
            .output()
        {
            if output.status.success() {
                return bin.to_string();
            }
        }
    }
    // Neither found — default to br (will fail later with clear error)
    "br".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            cli_binary: default_cli_binary(),
        }
    }
}

fn get_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.aryrabelo.borabr")
        .join("settings.json")
}

fn load_config() -> AppConfig {
    let path = get_config_path();
    if path.exists() {
        match fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(config) => return config,
                Err(e) => log::warn!("[config] Failed to parse settings.json: {}", e),
            },
            Err(e) => log::warn!("[config] Failed to read settings.json: {}", e),
        }
    }
    AppConfig::default()
}

fn save_config(config: &AppConfig) -> Result<(), String> {
    let path = get_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {}", e))?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    fs::write(&path, json)
        .map_err(|e| format!("Failed to write config: {}", e))?;
    Ok(())
}

fn get_cli_binary() -> String {
    CLI_BINARY.lock().unwrap().clone()
}

// ============================================================================
// CLI Client Detection (bd vs br)
// ============================================================================

/// Detect the client type from the version string.
/// - "bd version 0.49.6 (Homebrew)" → Bd
/// - "br 0.1.13 (rustc 1.85.0-nightly)" → Br
fn detect_cli_client(version_str: &str) -> CliClient {
    let lower = version_str.to_lowercase();
    if lower.starts_with("br ") || lower.contains("beads_rust") || lower.contains("beads-rust") {
        CliClient::Br
    } else if lower.starts_with("bd ") || lower.contains("bd version") {
        CliClient::Bd
    } else {
        CliClient::Unknown
    }
}

/// Parse a version string into (major, minor, patch).
/// Works for both "bd version 0.49.6 (Homebrew)" and "br 0.1.13 (rustc ...)".
fn parse_bd_version(version_str: &str) -> Option<(u32, u32, u32)> {
    // Look for a semver-like pattern: digits.digits.digits
    let re_like = version_str
        .split_whitespace()
        .find(|word| word.contains('.') && word.chars().next().map_or(false, |c| c.is_ascii_digit()));

    let version_part = re_like?;
    let parts: Vec<&str> = version_part.split('.').collect();
    if parts.len() >= 3 {
        let major = parts[0].parse::<u32>().ok()?;
        let minor = parts[1].parse::<u32>().ok()?;
        // Patch may have trailing non-numeric chars (e.g. "6-beta")
        let patch_str: String = parts[2].chars().take_while(|c| c.is_ascii_digit()).collect();
        let patch = patch_str.parse::<u32>().ok()?;
        Some((major, minor, patch))
    } else {
        None
    }
}

/// Detect and cache the CLI client type and version. Runs `binary --version` once.
fn get_cli_client_info() -> Option<(CliClient, u32, u32, u32)> {
    let mut cached = CLI_CLIENT_INFO.lock().unwrap();
    if let Some(info) = *cached {
        return Some(info);
    }

    let binary = get_cli_binary();
    // Run from temp dir to avoid bd auto-migrating projects in cwd
    let output = new_command(&binary)
        .arg("--version")
        .current_dir(std::env::temp_dir())
        .env("PATH", get_extended_path())
        .output()
        .ok()?;

    if !output.status.success() {
        log_warn!("[cli_detect] Failed to get version from {}", binary);
        return None;
    }

    let version_str = String::from_utf8_lossy(&output.stdout);
    let trimmed = version_str.trim();
    let client = detect_cli_client(trimmed);
    let tuple = parse_bd_version(trimmed);

    if let Some((major, minor, patch)) = tuple {
        let info = (client, major, minor, patch);
        let client_name = match client {
            CliClient::Bd => "bd",
            CliClient::Br => "br",
            CliClient::Unknown => "unknown",
        };
        log_info!("[cli_detect] Detected {} client v{}.{}.{}", client_name, major, minor, patch);
        *cached = Some(info);
        Some(info)
    } else {
        log_warn!("[cli_detect] Could not parse version from: {}", trimmed);
        None
    }
}

/// Returns true if the CLI supports the --no-daemon flag.
/// - br: NEVER (no daemon concept)
/// - bd < 0.50.0: YES
/// - bd >= 0.50.0: NO (daemon removed)
/// - unknown: NO (safe default)
fn supports_daemon_flag() -> bool {
    match get_cli_client_info() {
        Some((CliClient::Br, _, _, _)) => false, // br has no daemon
        Some((CliClient::Bd, major, minor, _)) => major == 0 && minor < 50,
        Some((CliClient::Unknown, _, _, _)) => false,
        None => false,
    }
}

/// Returns true if the CLI uses issues.jsonl files.
/// - br: ALWAYS (frozen on SQLite+JSONL architecture)
/// - bd < 0.50.0: YES
/// - bd >= 0.50.0: NO (Dolt only)
/// - unknown: NO (safe default)
fn uses_jsonl_files() -> bool {
    match get_cli_client_info() {
        Some((CliClient::Br, _, _, _)) => true, // br always uses JSONL
        Some((CliClient::Bd, major, minor, _)) => major == 0 && minor < 50,
        Some((CliClient::Unknown, _, _, _)) => false,
        None => false,
    }
}

/// Returns true if `bd list --all` works correctly.
/// The --all flag was buggy before bd 0.55.0 (returned incorrect results).
/// - br: NO
/// - bd >= 0.55.0: YES
/// - bd < 0.55.0: NO (use 2 separate calls instead)
/// - unknown: NO (safe default)
fn supports_list_all_flag() -> bool {
    match get_cli_client_info() {
        Some((CliClient::Bd, major, minor, _)) => major > 0 || minor >= 55,
        Some((CliClient::Br, _, _, _)) => true, // br always supports --all
        _ => false,
    }
}

/// Returns true if `bd delete --hard` is supported.
/// The --hard flag was removed in bd 0.50.0.
/// - br: NO
/// - bd < 0.50.0: YES
/// - bd >= 0.50.0: NO (only --force needed)
/// - unknown: NO (safe default)
fn supports_delete_hard_flag() -> bool {
    match get_cli_client_info() {
        Some((CliClient::Bd, major, minor, _)) => major == 0 && minor < 50,
        _ => false,
    }
}

/// Returns true if the CLI uses the Dolt backend (inverse of uses_jsonl_files).
/// - br: NEVER (frozen on SQLite+JSONL architecture)
/// - bd >= 0.50.0: YES (Dolt only)
/// - bd < 0.50.0: NO (SQLite+JSONL)
/// - unknown: NO (safe default)
fn uses_dolt_backend() -> bool {
    match get_cli_client_info() {
        Some((CliClient::Br, _, _, _)) => false, // br never uses Dolt
        Some((CliClient::Bd, major, minor, _)) => major > 0 || minor >= 50,
        Some((CliClient::Unknown, _, _, _)) => false,
        None => false,
    }
}

/// Returns true if a specific project uses the Dolt backend.
/// Checks for the presence of `.beads/.dolt/` directory in the project.
/// - br: NEVER (frozen on SQLite+JSONL architecture)
/// - bd < 0.50.0: NEVER (CLI doesn't support Dolt)
/// - bd >= 0.50.0: checks if `.dolt/` directory exists inside the beads dir
fn project_uses_dolt(beads_dir: &std::path::Path) -> bool {
    match get_cli_client_info() {
        Some((CliClient::Br, _, _, _)) => false,
        Some((CliClient::Bd, major, minor, _)) if major == 0 && minor < 50 => false,
        _ => {
            // Check .beads/.dolt (legacy) or .beads/dolt/<name>/.dolt (bd 0.52+)
            if beads_dir.join(".dolt").is_dir() {
                return true;
            }
            // Check metadata.json for backend: "dolt"
            let metadata_path = beads_dir.join("metadata.json");
            if let Ok(content) = std::fs::read_to_string(&metadata_path) {
                if content.contains("\"backend\":\"dolt\"") || content.contains("\"backend\": \"dolt\"") {
                    // Verify dolt database actually exists
                    let dolt_dir = beads_dir.join("dolt");
                    if dolt_dir.is_dir() {
                        // Check if any subdirectory has .dolt
                        if let Ok(entries) = std::fs::read_dir(&dolt_dir) {
                            for entry in entries.flatten() {
                                if entry.path().join(".dolt").is_dir() {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
            false
        }
    }
}

/// Reset the cached client info (called when CLI binary path changes).
fn reset_bd_version_cache() {
    let mut cached = CLI_CLIENT_INFO.lock().unwrap();
    *cached = None;
}

fn execute_bd(command: &str, args: &[String], cwd: Option<&str>) -> Result<String, String> {
    let working_dir = cwd
        .map(String::from)
        .or_else(|| env::var("BEADS_PATH").ok())
        .unwrap_or_else(|| {
            env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });

    // Split command by spaces to handle subcommands like "comments add"
    let mut full_args: Vec<&str> = command.split_whitespace().collect();
    for arg in args {
        full_args.push(arg);
    }
    if supports_daemon_flag() {
        full_args.push("--no-daemon");
    }
    full_args.push("--json");

    let binary = get_cli_binary();
    log_info!("[bd] {} {} | cwd: {}", binary, full_args.join(" "), working_dir);

    // Acquire per-project lock to prevent concurrent Dolt access (causes SIGSEGV).
    let project_lock = {
        let mut locks = BD_PROJECT_LOCKS.lock().unwrap();
        locks.entry(working_dir.clone())
            .or_insert_with(|| std::sync::Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = project_lock.lock().unwrap();

    let output = new_command(&binary)
        .args(&full_args)
        .current_dir(&working_dir)
        .env("PATH", get_extended_path())
        .env("BEADS_PATH", &working_dir)
        .output()
        .map_err(|e| {
            log_error!("[bd] Failed to execute {}: {}", binary, e);
            format!("Failed to execute {}: {}", binary, e)
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log_error!("[bd] Command failed | status: {} | stderr: {}", output.status, stderr);

        // Detect schema migration failure (bd 0.49.4 migration bug)
        if stderr.contains("no such column: spec_id") {
            log_error!("[bd] Schema migration failure detected - database needs repair");
            return Err("SCHEMA_MIGRATION_ERROR: Database schema is incompatible. Please use the repair function to fix this issue.".to_string());
        }

        if !stderr.is_empty() {
            return Err(stderr.to_string());
        }
        return Err(format!("bd command failed with status: {}", output.status));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    log_info!("[bd] OK | {} bytes", stdout.len());

    // Log output preview only if verbose mode is enabled
    if VERBOSE_LOGGING.load(Ordering::Relaxed) {
        let preview: String = stdout.chars().take(500).collect();
        log_debug!("[bd] Output: {}", preview);
    }

    Ok(stdout)
}

/// Auto-run refs migration v3 (filesystem-only attachments) if needed.
/// Called synchronously before br sync to prevent UNIQUE constraint errors.
fn ensure_refs_migrated_v3(beads_dir: &std::path::Path, working_dir: &str) {
    if beads_dir.join(".migrated-attachments").exists() {
        return;
    }
    let jsonl_path = beads_dir.join("issues.jsonl");
    if !jsonl_path.exists() {
        let _ = std::fs::write(beads_dir.join(".migrated-attachments"), "");
        return;
    }

    let content = match std::fs::read_to_string(&jsonl_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    // Quick scan: does any line have non-real external refs?
    let mut needs_migration = false;
    for line in content.lines() {
        if line.trim().is_empty() { continue; }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ext_ref = v.get("external_ref").and_then(|r| r.as_str()).unwrap_or("");
        // Non-real ref (att:, paths, cleared: sentinels, etc.)
        if ext_ref.is_empty() { continue; }
        for r in ext_ref.split(|c: char| c == '\n' || c == '|') {
            let trimmed = r.trim();
            if !trimmed.is_empty() && !is_real_external_ref(trimmed) {
                needs_migration = true;
                break;
            }
        }
        if needs_migration { break; }
    }

    // Also check if attachment folders need renaming
    let attachments_dir_check = beads_dir.join("attachments");
    let mut needs_folder_work = false;
    if attachments_dir_check.exists() {
        if let Ok(entries) = std::fs::read_dir(&attachments_dir_check) {
            for entry in entries.flatten() {
                if !entry.path().is_dir() { continue; }
                let name = entry.file_name().to_string_lossy().to_string();
                if issue_short_id(&name) != name {
                    needs_folder_work = true;
                    break;
                }
            }
        }
    }

    if !needs_migration && !needs_folder_work {
        let _ = std::fs::write(beads_dir.join(".migrated-attachments"), "");
        return;
    }

    log_info!("[sync] Auto-migrating v3 (refs={}, folders={}) for: {}", needs_migration, needs_folder_work, working_dir);

    // Backup
    let backup_path = beads_dir.join("issues.jsonl.bak-refs-v3-migration");
    if std::fs::copy(&jsonl_path, &backup_path).is_err() {
        log_error!("[sync] Failed to backup JSONL for v3 migration, skipping");
        return;
    }

    // Migrate: strip non-real refs, deduplicate
    let mut refs_updated: u32 = 0;
    let mut output_lines: Vec<String> = Vec::new();
    let mut seen_refs: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in content.lines() {
        if line.trim().is_empty() {
            output_lines.push(line.to_string());
            continue;
        }
        let mut v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => { output_lines.push(line.to_string()); continue; }
        };

        let issue_id = v.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();

        let ext_ref = v.get("external_ref").and_then(|r| r.as_str()).unwrap_or("").to_string();

        // Parse existing refs, keep only real external ones
        let real_refs: Vec<String> = if ext_ref.is_empty() {
            vec![]
        } else {
            ext_ref.split(|c: char| c == '\n' || c == '|')
                .map(|r| r.trim())
                .filter(|r| is_real_external_ref(r))
                .map(String::from)
                .collect()
        };

        let mut new_ref = if real_refs.is_empty() {
            String::new()
        } else {
            real_refs.join("|")
        };

        // Deduplicate: if another issue already has this exact ref, clear it
        if !new_ref.is_empty() && seen_refs.contains(&new_ref) {
            log_info!("[sync] Duplicate external_ref '{}' for issue {}, clearing", new_ref, issue_id);
            new_ref = String::new();
        }
        if !new_ref.is_empty() {
            seen_refs.insert(new_ref.clone());
        }

        if new_ref != ext_ref {
            v["external_ref"] = serde_json::Value::String(new_ref);
            refs_updated += 1;
            output_lines.push(serde_json::to_string(&v).unwrap_or_else(|_| line.to_string()));
            continue;
        }

        // Track existing refs that weren't modified too
        if let Some(ext_ref) = v.get("external_ref").and_then(|r| r.as_str()) {
            seen_refs.insert(ext_ref.to_string());
        }

        output_lines.push(line.to_string());
    }

    if refs_updated > 0 {
        let new_content = output_lines.join("\n");
        if std::fs::write(&jsonl_path, &new_content).is_err() {
            log_error!("[sync] Failed to write migrated JSONL");
            return;
        }
        log_info!("[sync] Refs v3 migration: {} ref(s) cleaned", refs_updated);
    }

    // Rename attachment folders: {full-id}/ → {short-id}/
    let attachments_dir = beads_dir.join("attachments");
    if attachments_dir.exists() {
        let mut renamed = 0u32;
        if let Ok(entries) = std::fs::read_dir(&attachments_dir) {
            let dirs: Vec<_> = entries.flatten()
                .filter(|e| e.path().is_dir())
                .collect();
            for entry in dirs {
                let folder_name = entry.file_name().to_string_lossy().to_string();
                let short = issue_short_id(&folder_name);
                if short != folder_name {
                    let target = attachments_dir.join(short);
                    if target.exists() {
                        log_warn!("[sync] Cannot rename '{}' → '{}': target already exists", folder_name, short);
                        continue;
                    }
                    if std::fs::rename(entry.path(), &target).is_ok() {
                        renamed += 1;
                    } else {
                        log_warn!("[sync] Failed to rename '{}' → '{}'", folder_name, short);
                    }
                }
            }
        }
        if renamed > 0 {
            log_info!("[sync] Renamed {} attachment folder(s) to short IDs", renamed);
        }
    }

    let _ = std::fs::write(beads_dir.join(".migrated-attachments"), "");
    // Signal for the frontend to show a notification
    let _ = std::fs::write(beads_dir.join(".migrated-attachments-notify"), "");
    log_info!("[sync] Migration v3 complete (refs cleaned + folders renamed)");
}

/// Sync the beads database before read operations to ensure data is up-to-date
/// Uses bidirectional sync to preserve local changes while getting remote updates
/// Has a cooldown to avoid redundant syncs within the same poll cycle
fn sync_bd_database(cwd: Option<&str>) {
    let working_dir = cwd
        .map(String::from)
        .or_else(|| env::var("BEADS_PATH").ok())
        .unwrap_or_else(|| {
            env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });

    // Dolt backend handles its own sync via git — skip bd sync
    let beads_dir = std::path::Path::new(&working_dir).join(".beads");
    if project_uses_dolt(&beads_dir) {
        log_info!("[sync] Skipping — Dolt backend handles sync via git");
        return;
    }

    // Check cooldown — skip if synced recently
    {
        let last = LAST_SYNC_TIME.lock().unwrap();
        if let Some(t) = *last {
            if t.elapsed().as_secs() < SYNC_COOLDOWN_SECS {
                log_info!("[sync] Skipping — cooldown active ({:.1}s ago)", t.elapsed().as_secs_f32());
                return;
            }
        }
    }

    log_info!("[sync] Starting bidirectional sync for: {}", working_dir);

    // Auto-migrate refs v3 before sync if needed (prevents UNIQUE constraint errors)
    ensure_refs_migrated_v3(&beads_dir, &working_dir);

    // Run bd sync (bidirectional - exports local changes AND imports remote changes)
    let binary = get_cli_binary();
    let mut sync_args = vec!["sync"];
    if supports_daemon_flag() {
        sync_args.push("--no-daemon");
    }
    match new_command(&binary)
        .args(&sync_args)
        .current_dir(&working_dir)
        .env("PATH", get_extended_path())
        .env("BEADS_PATH", &working_dir)
        .output()
    {
        Ok(output) if output.status.success() => {
            log_info!("[sync] Sync completed successfully");
            // Update cooldown timestamp
            let mut last = LAST_SYNC_TIME.lock().unwrap();
            *last = Some(Instant::now());
        }
        Ok(output) => {
            log_warn!(
                "[sync] {} sync failed: {}",
                binary,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Err(e) => {
            log_error!("[sync] Failed to run {} sync: {}", binary, e);
        }
    }
}

// ============================================================================
// Tauri Commands
// ============================================================================

#[tauri::command]
async fn bd_sync(cwd: Option<String>) -> Result<(), String> {
    let working_dir = cwd
        .or_else(|| env::var("BEADS_PATH").ok())
        .unwrap_or_else(|| {
            env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });

    // Dolt backend handles its own sync via git — skip bd sync
    let beads_dir = std::path::Path::new(&working_dir).join(".beads");
    if project_uses_dolt(&beads_dir) {
        log_info!("[bd_sync] Skipping — Dolt backend handles sync via git");
        return Ok(());
    }

    let binary = get_cli_binary();
    log_info!("[bd_sync] Manual sync requested for: {}", working_dir);

    let mut sync_args = vec!["sync"];
    if supports_daemon_flag() {
        sync_args.push("--no-daemon");
    }
    let output = new_command(&binary)
        .args(&sync_args)
        .current_dir(&working_dir)
        .env("PATH", get_extended_path())
        .env("BEADS_PATH", &working_dir)
        .output()
        .map_err(|e| format!("Failed to run {} sync: {}", binary, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log_error!("[bd_sync] Sync failed: {}", stderr.trim());
        return Err(format!("Sync failed: {}", stderr.trim()));
    }

    log_info!("[bd_sync] Sync completed successfully");
    // Reset cooldown so subsequent reads pick up the fresh sync
    let mut last = LAST_SYNC_TIME.lock().unwrap();
    *last = Some(Instant::now());
    Ok(())
}

#[derive(Debug, serde::Serialize)]
struct RepairResult {
    success: bool,
    message: String,
    backup_path: Option<String>,
}

#[tauri::command]
async fn bd_repair_database(cwd: Option<String>) -> Result<RepairResult, String> {
    let working_dir = cwd
        .or_else(|| env::var("BEADS_PATH").ok())
        .unwrap_or_else(|| {
            env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });

    log_info!("[bd_repair] Starting database repair for: {}", working_dir);

    let beads_dir = std::path::Path::new(&working_dir).join(".beads");

    // Check if .beads directory exists
    if !beads_dir.exists() {
        return Err("No .beads directory found in this project".to_string());
    }

    // Dolt backend: use `bd doctor --fix --yes`
    if project_uses_dolt(&beads_dir) {
        log_info!("[bd_repair] Using Dolt-based repair strategy (bd >= 0.50.0): bd doctor --fix --yes");
        let binary = get_cli_binary();
        let output = new_command(&binary)
            .args(&["doctor", "--fix", "--yes"])
            .current_dir(&working_dir)
            .env("PATH", get_extended_path())
            .env("BEADS_PATH", &working_dir)
            .output()
            .map_err(|e| format!("Failed to run bd doctor: {}", e))?;

        return if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            log_info!("[bd_repair] Dolt repair successful: {}", stdout.trim());
            Ok(RepairResult {
                success: true,
                message: format!("Database repaired via bd doctor. {}", stdout.trim()),
                backup_path: None,
            })
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log_error!("[bd_repair] Dolt repair failed: {}", stderr.trim());
            Err(format!("Repair failed: {}", stderr.trim()))
        };
    }

    // SQLite backend: original repair logic
    let db_path = beads_dir.join("beads.db");
    let jsonl_path = beads_dir.join("issues.jsonl");
    let backup_path = beads_dir.join("beads.db.backup");

    // Check if database exists
    if !db_path.exists() {
        return Ok(RepairResult {
            success: true,
            message: "No database to repair - it will be created on next operation".to_string(),
            backup_path: None,
        });
    }

    // For bd < 0.50.0: require issues.jsonl for repair (db is rebuilt from JSONL)
    if uses_jsonl_files() {
        let jsonl_size = std::fs::metadata(&jsonl_path)
            .map(|m| m.len())
            .unwrap_or(0);

        if !jsonl_path.exists() || jsonl_size == 0 {
            return Err("Cannot repair: issues.jsonl is missing or empty. Your data would be lost.".to_string());
        }
        log_info!("[bd_repair] Using JSONL-based repair strategy (bd < 0.50.0)");
    } else {
        log_info!("[bd_repair] Using repair strategy for unknown version");
    }

    // Create backup of current database
    if let Err(e) = std::fs::copy(&db_path, &backup_path) {
        log_error!("[bd_repair] Failed to create backup: {}", e);
        return Err(format!("Failed to create backup: {}", e));
    }
    log_info!("[bd_repair] Backup created at: {:?}", backup_path);

    // Remove database files
    std::fs::remove_file(&db_path).ok();
    std::fs::remove_file(beads_dir.join("beads.db-shm")).ok();
    std::fs::remove_file(beads_dir.join("beads.db-wal")).ok();
    log_info!("[bd_repair] Removed old database files");

    // Test that bd can now work (it will recreate the database)
    let mut test_args = vec!["list", "--limit=1"];
    if supports_daemon_flag() {
        test_args.push("--no-daemon");
    }
    test_args.push("--json");
    let test_output = new_command(&get_cli_binary())
        .args(&test_args)
        .current_dir(&working_dir)
        .env("PATH", get_extended_path())
        .env("BEADS_PATH", &working_dir)
        .output();

    match test_output {
        Ok(output) if output.status.success() => {
            log_info!("[bd_repair] Repair successful - database recreated");
            Ok(RepairResult {
                success: true,
                message: "Database repaired successfully. Your issues have been restored from the backup file.".to_string(),
                backup_path: Some(backup_path.to_string_lossy().to_string()),
            })
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log_error!("[bd_repair] Repair verification failed: {}", stderr);
            Err(format!("Repair failed during verification: {}", stderr))
        }
        Err(e) => {
            log_error!("[bd_repair] Failed to verify repair: {}", e);
            Err(format!("Failed to verify repair: {}", e))
        }
    }
}

// ============================================================================
// Dolt Migration
// ============================================================================

#[derive(Debug, serde::Serialize)]
struct MigrateResult {
    success: bool,
    message: String,
}

/// Remove orphaned Dolt lock files that block database access.
///
/// Uses `lsof` to check if any process actually holds the lock file open.
/// - If no process has it open → orphaned lock from a crashed/finished bd → safe to remove.
/// - If a process has it open → active agent (Claude Code, Gastown, etc.) → leave it alone.
///
/// This is the only reliable way to distinguish a stale lock from an active one,
/// regardless of timing. bd 0.55+ in embedded Dolt mode leaves noms/LOCK behind
/// after every command, so these accumulate and block subsequent operations.

#[derive(Debug, serde::Serialize)]
struct CleanupResult {
    removed: Vec<String>,
}

/// Stale lock cleanup — currently a no-op.
///
/// bd 0.55 in embedded Dolt mode leaves lock files (dolt-access.lock, noms/LOCK)
/// after every command. These locks are NOT safe to remove externally:
/// - Removing noms/LOCK causes Dolt SIGSEGV (nil pointer dereference) on next bd call
/// - Removing dolt-access.lock also triggers the same Dolt crash
///
/// This is a bd/Dolt bug that needs to be fixed upstream. The command is kept as a
/// no-op so the frontend call doesn't need to change when a fix becomes available.
#[tauri::command]
async fn bd_cleanup_stale_locks(cwd: Option<String>) -> Result<CleanupResult, String> {
    let _ = cwd; // suppress unused warning
    Ok(CleanupResult { removed: vec![] })
}

/// Check if a project needs Dolt migration.
/// Returns true when bd >= 0.50, project has .beads/, but is not fully migrated to Dolt.
/// Detects both "never migrated" and "partially migrated" (dolt/ dir exists but .dolt marker missing).
#[derive(Debug, serde::Serialize)]
struct MigrationStatus {
    needs_migration: bool,
    reason: String,
}

#[tauri::command]
async fn bd_check_needs_migration(cwd: Option<String>) -> Result<MigrationStatus, String> {
    let working_dir = cwd
        .or_else(|| env::var("BEADS_PATH").ok())
        .unwrap_or_else(|| {
            env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });

    let beads_dir = std::path::Path::new(&working_dir).join(".beads");

    if !beads_dir.exists() {
        return Ok(MigrationStatus {
            needs_migration: false,
            reason: "No .beads directory".to_string(),
        });
    }

    // Check bd version — only bd >= 0.50 requires Dolt
    match get_cli_client_info() {
        Some((CliClient::Bd, major, minor, _)) if major > 0 || minor >= 50 => {
            // bd >= 0.50: check if project is fully migrated
        }
        _ => {
            return Ok(MigrationStatus {
                needs_migration: false,
                reason: "bd version does not require Dolt".to_string(),
            });
        }
    }

    // Already fully using Dolt? (.beads/.dolt exists)
    if project_uses_dolt(&beads_dir) {
        return Ok(MigrationStatus {
            needs_migration: false,
            reason: "Already using Dolt backend".to_string(),
        });
    }

    // Check for partial migration (dolt/ dir exists but not complete)
    let dolt_dir = beads_dir.join("dolt");
    if dolt_dir.exists() {
        return Ok(MigrationStatus {
            needs_migration: true,
            reason: "Partial migration detected (dolt/ exists but migration incomplete)".to_string(),
        });
    }

    // Has JSONL data but no Dolt — needs migration
    let jsonl_path = beads_dir.join("issues.jsonl");
    if jsonl_path.exists() {
        let jsonl_size = std::fs::metadata(&jsonl_path).map(|m| m.len()).unwrap_or(0);
        if jsonl_size > 0 {
            return Ok(MigrationStatus {
                needs_migration: true,
                reason: "SQLite/JSONL project needs Dolt migration".to_string(),
            });
        }
    }

    // Has SQLite db but no Dolt
    let db_path = beads_dir.join("beads.db");
    if db_path.exists() {
        return Ok(MigrationStatus {
            needs_migration: true,
            reason: "SQLite project needs Dolt migration".to_string(),
        });
    }

    // Empty project — no migration needed (bd init will create Dolt directly)
    Ok(MigrationStatus {
        needs_migration: false,
        reason: "Empty project".to_string(),
    })
}

/// Re-prefix an issue ID if it uses a non-target prefix
fn reprefix_id(id: &str, target_prefix: &str, prefix_counts: &std::collections::HashMap<String, usize>) -> String {
    if let Some(last_dash) = id.rfind('-') {
        let current_prefix = &id[..last_dash];
        if current_prefix != target_prefix && prefix_counts.contains_key(current_prefix) {
            return format!("{}{}", target_prefix, &id[last_dash..]);
        }
    }
    id.to_string()
}

#[tauri::command]
async fn bd_migrate_to_dolt(cwd: Option<String>) -> Result<MigrateResult, String> {
    let working_dir = cwd
        .or_else(|| env::var("BEADS_PATH").ok())
        .unwrap_or_else(|| {
            env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });

    log_info!("[bd_migrate] Starting Dolt migration for: {}", working_dir);

    let beads_dir = std::path::Path::new(&working_dir).join(".beads");

    // Check if .beads directory exists
    if !beads_dir.exists() {
        return Err("No .beads directory found in this project".to_string());
    }

    // Already using Dolt?
    if project_uses_dolt(&beads_dir) {
        return Ok(MigrateResult {
            success: true,
            message: "Project already uses the Dolt backend.".to_string(),
        });
    }

    // Verify bd >= 0.50
    if let Some((_, major, minor, _)) = get_cli_client_info() {
        if major == 0 && minor < 50 {
            return Err(format!(
                "bd version 0.50+ is required for Dolt migration (current: {}.{})",
                major, minor
            ));
        }
    } else {
        return Err("Could not determine bd version".to_string());
    }

    // Clean up partial migration if dolt/ directory exists
    let dolt_dir = beads_dir.join("dolt");
    if dolt_dir.exists() {
        log_info!("[bd_migrate] Removing partial dolt/ directory for re-migration");
        std::fs::remove_dir_all(&dolt_dir)
            .map_err(|e| format!("Failed to remove partial dolt/ directory: {}", e))?;
    }

    // Remove dolt-access.lock if present
    let dolt_lock = beads_dir.join("dolt-access.lock");
    if dolt_lock.exists() {
        std::fs::remove_file(&dolt_lock).ok();
    }

    // Try `bd migrate --to-dolt --yes` first
    let binary = get_cli_binary();
    let output = new_command(&binary)
        .args(&["migrate", "--to-dolt", "--yes"])
        .current_dir(&working_dir)
        .env("PATH", get_extended_path())
        .env("BEADS_PATH", &working_dir)
        .output()
        .map_err(|e| format!("Failed to run bd migrate: {}", e))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        log_info!("[bd_migrate] Migration via bd migrate successful: {}", stdout.trim());
        return Ok(MigrateResult {
            success: true,
            message: format!("Migration to Dolt completed successfully. {}", stdout.trim()),
        });
    }

    // bd migrate failed (typically: corrupt SQLite, missing table, etc.)
    // Fallback: bd init + bd import from JSONL
    let stderr_migrate = String::from_utf8_lossy(&output.stderr);
    log_info!("[bd_migrate] bd migrate failed ({}), trying init+import fallback", stderr_migrate.trim());

    let jsonl_path = beads_dir.join("issues.jsonl");
    if !jsonl_path.exists() || std::fs::metadata(&jsonl_path).map(|m| m.len()).unwrap_or(0) == 0 {
        // Empty project — no JSONL data to import, just run bd init
        log_info!("[bd_migrate] No issues.jsonl data — empty project, attempting init-only migration");
        // Rename existing .db files to .db.backup so bd init doesn't refuse
        if let Ok(entries) = std::fs::read_dir(&beads_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".db") && !name.ends_with(".db.backup") {
                    let src = entry.path();
                    let dst = beads_dir.join(format!("{}.backup", name));
                    log_info!("[bd_migrate] Renaming {} -> {}", src.display(), dst.display());
                    std::fs::rename(&src, &dst).ok();
                }
                // Also remove .db-shm and .db-wal
                if name.ends_with(".db-shm") || name.ends_with(".db-wal") {
                    std::fs::remove_file(entry.path()).ok();
                }
            }
        }
        let init_output = new_command(&binary)
            .args(&["init", "--prefix", "project"])
            .current_dir(&working_dir)
            .env("PATH", get_extended_path())
            .env("BEADS_PATH", &working_dir)
            .output()
            .map_err(|e| format!("Failed to run bd init: {}", e))?;
        if init_output.status.success() {
            log_info!("[bd_migrate] Empty project initialized with Dolt backend");
            return Ok(MigrateResult {
                success: true,
                message: "Migration complete (empty project — initialized with Dolt backend)".to_string(),
            });
        }
        let init_stderr = String::from_utf8_lossy(&init_output.stderr);
        return Err(format!(
            "Migration failed (empty project, bd init also failed): {}. Original error: {}",
            init_stderr.trim(), stderr_migrate.trim()
        ));
    }

    // Detect prefix from JSONL — use the most common prefix
    let jsonl_content = std::fs::read_to_string(&jsonl_path)
        .map_err(|e| format!("Failed to read issues.jsonl: {}", e))?;
    let mut prefix_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for line in jsonl_content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                if let Some(last_dash) = id.rfind('-') {
                    let suffix = &id[last_dash + 1..];
                    if suffix.chars().all(|c| c.is_alphanumeric()) && !suffix.is_empty() {
                        *prefix_counts.entry(id[..last_dash].to_string()).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    let prefix = prefix_counts
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(p, _)| p.clone())
        .ok_or_else(|| "Could not detect issue prefix from issues.jsonl".to_string())?;

    if prefix_counts.len() > 1 {
        log_info!(
            "[bd_migrate] Multiple prefixes found: {:?}. Using most common: {}",
            prefix_counts, prefix
        );
    }
    log_info!("[bd_migrate] Detected prefix: {}", prefix);

    // Clean dolt dir again (bd migrate may have created a partial one)
    if dolt_dir.exists() {
        log_info!("[bd_migrate] Removing dolt/ directory before init");
        if let Err(e) = std::fs::remove_dir_all(&dolt_dir) {
            log_error!("[bd_migrate] Failed to remove dolt/: {}", e);
            return Err(format!("Failed to clean up dolt/ directory: {}", e));
        }
    }
    // Remove dolt-access.lock
    let dolt_lock2 = beads_dir.join("dolt-access.lock");
    if dolt_lock2.exists() {
        std::fs::remove_file(&dolt_lock2).ok();
    }
    // Backup main SQLite .db file (for comment restoration), then remove all SQLite files
    if let Ok(entries) = std::fs::read_dir(&beads_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".db") && !name.ends_with(".db.backup") {
                // Rename to .backup before deleting (preserves comments for Step 6)
                let backup_name = format!("{}.backup", name);
                let backup_path = beads_dir.join(&backup_name);
                if !backup_path.exists() {
                    log_info!("[bd_migrate] Backing up SQLite: {} -> {}", name, backup_name);
                    std::fs::rename(entry.path(), &backup_path).ok();
                } else {
                    log_info!("[bd_migrate] Removing SQLite file: {} (backup already exists)", name);
                    std::fs::remove_file(entry.path()).ok();
                }
            } else if name.ends_with(".db-shm") || name.ends_with(".db-wal") || name.ends_with(".db?mode=ro") {
                log_info!("[bd_migrate] Removing SQLite file: {}", name);
                std::fs::remove_file(entry.path()).ok();
            }
        }
    }

    // Reset metadata.json if it was set to dolt by a previous failed attempt
    let metadata_path = beads_dir.join("metadata.json");
    if metadata_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&metadata_path) {
            if content.contains("\"backend\":\"dolt\"") || content.contains("\"backend\": \"dolt\"") {
                log_info!("[bd_migrate] Resetting metadata.json backend from dolt to sqlite");
                std::fs::remove_file(&metadata_path).ok();
            }
        }
    }

    // Remove .local_version (stale after cleanup)
    let local_version = beads_dir.join(".local_version");
    if local_version.exists() {
        std::fs::remove_file(&local_version).ok();
    }

    // Step 1: bd init --prefix <prefix>
    let init_output = new_command(&binary)
        .args(&["init", "--prefix", &prefix])
        .current_dir(&working_dir)
        .env("PATH", get_extended_path())
        .env("BEADS_PATH", &working_dir)
        .output()
        .map_err(|e| format!("Failed to run bd init: {}", e))?;

    if !init_output.status.success() {
        let stderr = String::from_utf8_lossy(&init_output.stderr);
        return Err(format!("bd init failed: {}", stderr.trim()));
    }
    log_info!("[bd_migrate] bd init successful");

    // Step 2: Filter tombstone issues and sanitize fields for Dolt compatibility
    let temp_jsonl = beads_dir.join("_migrate_clean.jsonl");
    {
        let mut clean_lines = Vec::new();
        let mut skipped = 0u32;
        for line in jsonl_content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<serde_json::Value>(trimmed) {
                Ok(mut v) => {
                    if v.get("status").and_then(|s| s.as_str()) == Some("tombstone") {
                        skipped += 1;
                        continue;
                    }
                    // Re-prefix issues with a different prefix to match the target
                    if let Some(id) = v.get("id").and_then(|i| i.as_str()).map(String::from) {
                        if let Some(last_dash) = id.rfind('-') {
                            let issue_prefix = &id[..last_dash];
                            if issue_prefix != prefix {
                                let suffix = &id[last_dash..]; // includes the '-'
                                let new_id = format!("{}{}", prefix, suffix);
                                let old_prefix = issue_prefix.to_string();
                                log_info!("[bd_migrate] Re-prefixing {} -> {}", id, new_id);
                                let obj = v.as_object_mut().unwrap();
                                obj.insert("id".to_string(), serde_json::Value::String(new_id));
                                // Re-prefix dependency references
                                if let Some(deps) = obj.get_mut("dependencies").and_then(|d| d.as_array_mut()) {
                                    for dep in deps.iter_mut() {
                                        if let Some(dep_obj) = dep.as_object_mut() {
                                            for key in &["issue_id", "depends_on_id"] {
                                                if let Some(val) = dep_obj.get(*key).and_then(|v| v.as_str()).map(String::from) {
                                                    if val.starts_with(&old_prefix) {
                                                        let new_val = format!("{}{}", prefix, &val[old_prefix.len()..]);
                                                        dep_obj.insert(key.to_string(), serde_json::Value::String(new_val));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Truncate external_ref if it contains multiple lines (attachment paths)
                    // Dolt's external_ref column can't hold multi-line values with long paths
                    // Keep only the first line (the meaningful ref: redmine ID, URL, etc.)
                    let needs_truncate = v.get("external_ref")
                        .and_then(|e| e.as_str())
                        .map(|s| s.contains('\n') || s.len() > 100)
                        .unwrap_or(false);
                    if needs_truncate {
                        let ext_ref = v["external_ref"].as_str().unwrap();
                        let first_line = ext_ref.lines().next().unwrap_or("").to_string();
                        let issue_id = v.get("id").and_then(|i| i.as_str()).unwrap_or("?").to_string();
                        let orig_len = ext_ref.len();
                        v.as_object_mut().unwrap().insert(
                            "external_ref".to_string(),
                            serde_json::Value::String(first_line),
                        );
                        log_info!(
                            "[bd_migrate] Truncated external_ref for issue {} (was {} chars)",
                            issue_id, orig_len
                        );
                    }
                    clean_lines.push(serde_json::to_string(&v).unwrap_or_else(|_| trimmed.to_string()));
                }
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            }
        }
        log_info!(
            "[bd_migrate] Filtered JSONL: {} valid, {} skipped (tombstone/malformed)",
            clean_lines.len(),
            skipped
        );

        // Empty project — no issues to import, just init is enough
        if clean_lines.is_empty() {
            log_info!("[bd_migrate] No issues to import — empty project, init-only migration");
            return Ok(MigrateResult {
                success: true,
                message: "Migration complete (empty project — initialized with Dolt backend)".to_string(),
            });
        }

        std::fs::write(&temp_jsonl, clean_lines.join("\n") + "\n")
            .map_err(|e| format!("Failed to write cleaned JSONL: {}", e))?;
    }

    // Step 3: bd import -i <cleaned_jsonl>
    let import_output = new_command(&binary)
        .args(&["import", "-i", &temp_jsonl.to_string_lossy()])
        .current_dir(&working_dir)
        .env("PATH", get_extended_path())
        .env("BEADS_PATH", &working_dir)
        .output()
        .map_err(|e| format!("Failed to run bd import: {}", e))?;

    // Clean up temp file
    std::fs::remove_file(&temp_jsonl).ok();

    if !import_output.status.success() {
        let stderr = String::from_utf8_lossy(&import_output.stderr);
        log_error!("[bd_migrate] Import failed: {}", stderr.trim());
        // Clean up failed migration so the modal will reappear
        if dolt_dir.exists() {
            std::fs::remove_dir_all(&dolt_dir).ok();
        }
        if beads_dir.join("dolt-access.lock").exists() {
            std::fs::remove_file(beads_dir.join("dolt-access.lock")).ok();
        }
        return Err(format!("Import failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&import_output.stdout);
    log_info!("[bd_migrate] Import successful: {}", stdout.trim());

    // Step 4: Restore labels (bd import doesn't preserve them)
    // Re-read JSONL to find issues with labels and apply them via bd update
    let mut labels_restored = 0u32;
    for line in jsonl_content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if v.get("status").and_then(|s| s.as_str()) == Some("tombstone") {
                continue;
            }
            let labels: Vec<String> = v
                .get("labels")
                .and_then(|l| l.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            if labels.is_empty() {
                continue;
            }

            let issue_id = match v.get("id").and_then(|i| i.as_str()) {
                Some(id) => id,
                None => continue,
            };

            // bd update <id> --set-labels label1 --set-labels label2
            let mut args = vec!["update".to_string(), issue_id.to_string()];
            for label in &labels {
                args.push("--set-labels".to_string());
                args.push(label.clone());
            }

            let label_output = new_command(&binary)
                .args(&args.iter().map(|s| s.as_str()).collect::<Vec<_>>())
                .current_dir(&working_dir)
                .env("PATH", get_extended_path())
                .env("BEADS_PATH", &working_dir)
                .output();

            match label_output {
                Ok(o) if o.status.success() => {
                    labels_restored += 1;
                }
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    log_info!("[bd_migrate] Failed to restore labels for {}: {}", issue_id, stderr.trim());
                }
                Err(e) => {
                    log_info!("[bd_migrate] Failed to run bd update for {}: {}", issue_id, e);
                }
            }
        }
    }

    if labels_restored > 0 {
        log_info!("[bd_migrate] Restored labels for {} issues", labels_restored);
    }

    // Step 5: Restore dependencies/relations (bd import doesn't preserve them)
    let mut deps_restored = 0u32;
    for line in jsonl_content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if v.get("status").and_then(|s| s.as_str()) == Some("tombstone") { continue; }
            let dependencies = match v.get("dependencies").and_then(|d| d.as_array()) {
                Some(deps) if !deps.is_empty() => deps,
                _ => continue,
            };

            for dep in dependencies {
                let dep_obj = match dep.as_object() {
                    Some(o) => o,
                    None => continue,
                };

                let issue_id = match dep_obj.get("issue_id").and_then(|v| v.as_str()) {
                    Some(id) => id.to_string(),
                    None => continue,
                };
                let depends_on_id = match dep_obj.get("depends_on_id").and_then(|v| v.as_str()) {
                    Some(id) => id.to_string(),
                    None => continue,
                };
                let dep_type = dep_obj.get("type").and_then(|v| v.as_str()).unwrap_or("blocks").to_string();

                // Re-prefix if needed
                let issue_id = reprefix_id(&issue_id, &prefix, &prefix_counts);
                let depends_on_id = reprefix_id(&depends_on_id, &prefix, &prefix_counts);

                // bd dep add <issue_id> <depends_on_id> --type <type>
                let dep_output = new_command(&binary)
                    .args(&["dep", "add", &issue_id, &depends_on_id, "--type", &dep_type])
                    .current_dir(&working_dir)
                    .env("PATH", get_extended_path())
                    .env("BEADS_PATH", &working_dir)
                    .output();

                match dep_output {
                    Ok(o) if o.status.success() => { deps_restored += 1; }
                    Ok(o) => {
                        let stderr = String::from_utf8_lossy(&o.stderr);
                        log_info!("[bd_migrate] Failed to restore dep {} -> {}: {}", issue_id, depends_on_id, stderr.trim());
                    }
                    Err(e) => {
                        log_info!("[bd_migrate] Failed to run bd dep add: {}", e);
                    }
                }
            }
        }
    }

    if deps_restored > 0 {
        log_info!("[bd_migrate] Restored {} dependencies/relations", deps_restored);
    }

    // Step 6: Restore comments from SQLite backup (if available)
    // bd import doesn't preserve comments, and JSONL only has empty bodies.
    // Look for a .db.backup file with a comments table.
    let mut comments_restored = 0u32;
    let sqlite_backup = {
        let mut found: Option<std::path::PathBuf> = None;
        if let Ok(entries) = std::fs::read_dir(&beads_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".db.backup") {
                    found = Some(entry.path());
                    break;
                }
            }
        }
        found
    };

    if let Some(backup_path) = sqlite_backup {
        log_info!("[bd_migrate] Found SQLite backup: {:?}, restoring comments", backup_path);
        // Use sqlite3 CLI to extract comments as JSON
        let sqlite_output = std::process::Command::new("sqlite3")
            .args(&[
                backup_path.to_string_lossy().as_ref(),
                "-json",
                "SELECT issue_id, author, text FROM comments WHERE text IS NOT NULL AND text != '' ORDER BY created_at ASC",
            ])
            .output();

        if let Ok(output) = sqlite_output {
            if output.status.success() {
                let json_str = String::from_utf8_lossy(&output.stdout);
                if let Ok(rows) = serde_json::from_str::<Vec<serde_json::Value>>(&json_str) {
                    for row in &rows {
                        let issue_id = match row.get("issue_id").and_then(|v| v.as_str()) {
                            Some(id) => id.to_string(),
                            None => continue,
                        };
                        let author = row.get("author").and_then(|v| v.as_str()).unwrap_or("unknown");
                        let text = match row.get("text").and_then(|v| v.as_str()) {
                            Some(t) if !t.is_empty() => t,
                            _ => continue,
                        };

                        // Re-prefix if needed
                        let issue_id = reprefix_id(&issue_id, &prefix, &prefix_counts);

                        // Write comment to temp file to handle multiline text
                        let comment_file = beads_dir.join("_migrate_comment.txt");
                        if std::fs::write(&comment_file, text).is_err() {
                            continue;
                        }

                        let comment_output = new_command(&binary)
                            .args(&["comments", "add", &issue_id, "-f", &comment_file.to_string_lossy(), "--author", author])
                            .current_dir(&working_dir)
                            .env("PATH", get_extended_path())
                            .env("BEADS_PATH", &working_dir)
                            .output();

                        match comment_output {
                            Ok(o) if o.status.success() => { comments_restored += 1; }
                            Ok(o) => {
                                let stderr = String::from_utf8_lossy(&o.stderr);
                                log_info!("[bd_migrate] Failed to restore comment for {}: {}", issue_id, stderr.trim());
                            }
                            Err(e) => {
                                log_info!("[bd_migrate] Failed to run bd comments add: {}", e);
                            }
                        }
                        std::fs::remove_file(&comment_file).ok();
                    }
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                log_info!("[bd_migrate] sqlite3 query failed: {}", stderr.trim());
            }
        }

        if comments_restored > 0 {
            log_info!("[bd_migrate] Restored {} comments from SQLite backup", comments_restored);
        }
    }

    Ok(MigrateResult {
        success: true,
        message: format!(
            "Migration to Dolt completed (via init+import). {} Labels: {}. Deps: {}. Comments: {}.",
            stdout.trim(),
            labels_restored,
            deps_restored,
            comments_restored,
        ),
    })
}

// ============================================================================
// Batched Poll Data
// ============================================================================

/// All data needed for a single poll cycle, fetched in one IPC call.
#[derive(Debug, Serialize)]
pub struct PollData {
    #[serde(rename = "openIssues")]
    pub open_issues: Vec<Issue>,
    #[serde(rename = "closedIssues")]
    pub closed_issues: Vec<Issue>,
    #[serde(rename = "readyIssues")]
    pub ready_issues: Vec<Issue>,
}

/// Batched poll: sync once, then fetch all issues + ready in 2 commands (was 3).
/// Replaces 3 separate IPC calls (bd_list + bd_list(closed) + bd_ready) with one.
#[tauri::command]
async fn bd_poll_data(cwd: Option<String>) -> Result<PollData, String> {
    log_info!("[bd_poll_data] Batched poll starting");

    let cwd_ref = cwd.as_deref();

    // Single sync for the entire poll cycle
    sync_bd_database(cwd_ref);

    // Fetch issues: single --all call for bd >= 0.55, fallback to 2 calls for older versions
    let (raw_open, raw_closed) = if supports_list_all_flag() {
        let all_output = execute_bd("list", &["--all".to_string(), "--limit=0".to_string()], cwd_ref)?;
        let raw_all = parse_issues_tolerant(&all_output, "bd_poll_data_all")?;
        let (open, closed): (Vec<_>, Vec<_>) = raw_all.into_iter()
            .partition(|issue: &BdRawIssue| issue.status != "closed");
        (open, closed)
    } else {
        let open_output = execute_bd("list", &["--limit=0".to_string()], cwd_ref)?;
        let closed_output = execute_bd("list", &["--status=closed".to_string(), "--limit=0".to_string()], cwd_ref)?;
        (
            parse_issues_tolerant(&open_output, "bd_poll_data_open")?,
            parse_issues_tolerant(&closed_output, "bd_poll_data_closed")?,
        )
    };

    // Fetch ready issues
    let ready_output = execute_bd("ready", &[], cwd_ref)?;
    let raw_ready = parse_issues_tolerant(&ready_output, "bd_poll_data_ready")?;

    log_info!("[bd_poll_data] Batched poll done: {} open, {} closed, {} ready",
        raw_open.len(), raw_closed.len(), raw_ready.len());

    // Update mtime AFTER our commands ran, so the next bd_check_changed
    // only detects EXTERNAL changes (not our own poll's side effects)
    {
        let working_dir = cwd_ref
            .map(String::from)
            .or_else(|| env::var("BEADS_PATH").ok())
            .unwrap_or_else(|| {
            env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });
        let beads_dir = std::path::Path::new(&working_dir).join(".beads");

        if let Some(mtime) = get_beads_mtime(&beads_dir) {
            let mut map = LAST_KNOWN_MTIME.lock().unwrap();
            map.insert(working_dir, mtime);
        }
    }

    Ok(PollData {
        open_issues: raw_open.into_iter().map(transform_issue).collect(),
        closed_issues: raw_closed.into_iter().map(transform_issue).collect(),
        ready_issues: raw_ready.into_iter().map(transform_issue).collect(),
    })
}

/// Get the latest mtime across all beads database files.
/// - Dolt backend (bd >= 0.50.0): checks .beads/ dir, .beads/.dolt/ (legacy) or
///   .beads/dolt/<name>/.dolt/ (bd 0.52+ nested layout), and manifest files
/// - SQLite backend: checks beads.db, beads.db-wal, and optionally issues.jsonl
fn get_beads_mtime(beads_dir: &std::path::Path) -> Option<std::time::SystemTime> {
    if project_uses_dolt(beads_dir) {
        // Dolt backend: check directory mtimes and manifest files
        let mut times: Vec<std::time::SystemTime> = Vec::new();

        // .beads/ dir mtime
        if let Ok(m) = fs::metadata(beads_dir) {
            if let Ok(t) = m.modified() { times.push(t); }
        }

        // Collect all .dolt/ directories to check:
        // - Legacy layout: .beads/.dolt/
        // - Nested layout (bd 0.52+): .beads/dolt/<name>/.dolt/
        let mut dolt_dirs: Vec<std::path::PathBuf> = Vec::new();

        let legacy_dolt = beads_dir.join(".dolt");
        if legacy_dolt.is_dir() {
            dolt_dirs.push(legacy_dolt);
        }

        let nested_dolt = beads_dir.join("dolt");
        if nested_dolt.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&nested_dolt) {
                for entry in entries.flatten() {
                    let sub_dolt = entry.path().join(".dolt");
                    if sub_dolt.is_dir() {
                        dolt_dirs.push(sub_dolt);
                    }
                }
            }
        }

        // Check mtime of each .dolt/ dir and its manifest files
        for dolt_dir in &dolt_dirs {
            if let Ok(m) = fs::metadata(dolt_dir) {
                if let Ok(t) = m.modified() { times.push(t); }
            }
            for name in &["manifest", "noms/manifest"] {
                let p = dolt_dir.join(name);
                if let Ok(m) = fs::metadata(&p) {
                    if let Ok(t) = m.modified() { times.push(t); }
                }
            }
        }

        // Also check issues.jsonl (Dolt exports to it for git sync)
        let jsonl_path = beads_dir.join("issues.jsonl");
        if let Ok(m) = fs::metadata(&jsonl_path) {
            if let Ok(t) = m.modified() { times.push(t); }
        }

        times.into_iter().max()
    } else {
        // SQLite backend: check db, WAL, and optionally JSONL
        let mut paths = vec![
            beads_dir.join("beads.db"),
            beads_dir.join("beads.db-wal"),
        ];
        if uses_jsonl_files() {
            paths.push(beads_dir.join("issues.jsonl"));
        }
        paths.iter()
            .filter_map(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
            .max()
    }
}

/// Check if the beads database has changed since last check (via filesystem mtime).
/// Returns true if changes detected or if this is the first check.
/// This is extremely cheap — just a few stat() calls, no bd process spawns.
#[tauri::command]
async fn bd_check_changed(cwd: Option<String>) -> Result<bool, String> {
    let working_dir = cwd
        .or_else(|| env::var("BEADS_PATH").ok())
        .unwrap_or_else(|| {
            env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });

    let beads_dir = std::path::Path::new(&working_dir).join(".beads");
    let current_mtime = get_beads_mtime(&beads_dir);

    let mut map = LAST_KNOWN_MTIME.lock().unwrap();
    let previous = map.get(&working_dir).copied();

    match (current_mtime, previous) {
        (Some(current), Some(prev)) => {
            if current != prev {
                log_info!("[bd_check_changed] mtime changed — data may have been modified");
                map.insert(working_dir, current);
                Ok(true)
            } else {
                log_debug!("[bd_check_changed] mtime unchanged — no changes");
                Ok(false)
            }
        }
        (Some(current), None) => {
            // First check — store mtime, report changed so initial load happens
            map.insert(working_dir, current);
            Ok(true)
        }
        (None, _) => {
            // No database file found
            log_warn!("[bd_check_changed] No beads database found in {}", working_dir);
            Ok(true) // Report changed to let caller handle missing db
        }
    }
}

/// Reset the cached mtime for a specific project (or all projects).
/// Called from the frontend when switching projects to force a fresh poll.
#[tauri::command]
async fn bd_reset_mtime(cwd: Option<String>) -> Result<(), String> {
    let mut map = LAST_KNOWN_MTIME.lock().unwrap();
    if let Some(path) = cwd {
        log_info!("[bd_reset_mtime] Resetting mtime for: {}", path);
        map.remove(&path);
    } else {
        log_info!("[bd_reset_mtime] Resetting all cached mtimes");
        map.clear();
    }
    Ok(())
}

#[tauri::command]
async fn bd_list(options: ListOptions) -> Result<Vec<Issue>, String> {
    log_info!("[bd_list] cwd: {:?}", options.cwd);

    // Sync database before reading to ensure data is up-to-date
    sync_bd_database(options.cwd.as_deref());

    let mut args: Vec<String> = Vec::new();

    // --all flag only works correctly on bd >= 0.55; for older versions, fallback to 2 calls
    let use_all = options.include_all.unwrap_or(false);
    if use_all && !supports_list_all_flag() {
        // Fallback: fetch open + closed separately and merge
        log_info!("[bd_list] --all requested but bd < 0.55 — falling back to 2 calls");
        let mut fallback_args = args.clone();
        fallback_args.push("--limit=0".to_string());

        let open_output = execute_bd("list", &fallback_args, options.cwd.as_deref())?;
        let open_issues = parse_issues_tolerant(&open_output, "bd_list_open")?;

        fallback_args.push("--status=closed".to_string());
        let closed_output = execute_bd("list", &fallback_args, options.cwd.as_deref())?;
        let closed_issues = parse_issues_tolerant(&closed_output, "bd_list_closed")?;

        let mut all_issues = open_issues;
        all_issues.extend(closed_issues);
        log_info!("[bd_list] Found {} issues (fallback)", all_issues.len());
        return Ok(all_issues.into_iter().map(transform_issue).collect());
    }

    if use_all {
        args.push("--all".to_string());
    }
    if let Some(ref statuses) = options.status {
        if !statuses.is_empty() {
            args.push(format!("--status={}", statuses.join(",")));
        }
    }
    if let Some(ref types) = options.issue_type {
        if !types.is_empty() {
            args.push(format!("--type={}", types.join(",")));
        }
    }
    if let Some(ref priorities) = options.priority {
        if !priorities.is_empty() {
            let nums: Vec<String> = priorities.iter().map(|p| priority_to_number(p)).collect();
            args.push(format!("--priority={}", nums.join(",")));
        }
    }
    if let Some(ref assignee) = options.assignee {
        args.push(format!("--assignee={}", assignee));
    }

    // Always disable limit to get all issues (bd defaults to 50)
    args.push("--limit=0".to_string());

    let output = execute_bd("list", &args, options.cwd.as_deref())?;

    let raw_issues = parse_issues_tolerant(&output, "bd_list")?;

    log_info!("[bd_list] Found {} issues", raw_issues.len());
    Ok(raw_issues.into_iter().map(transform_issue).collect())
}

#[tauri::command]
async fn bd_count(options: CwdOptions) -> Result<CountResult, String> {
    // Sync database before reading to ensure data is up-to-date
    sync_bd_database(options.cwd.as_deref());

    // Fetch all issues: single --all call for bd >= 0.55, fallback to 2 calls for older versions
    let raw_issues = if supports_list_all_flag() {
        let all_output = execute_bd("list", &["--all".to_string(), "--limit=0".to_string()], options.cwd.as_deref())?;
        parse_issues_tolerant(&all_output, "bd_count_all")?
    } else {
        let open_output = execute_bd("list", &["--limit=0".to_string()], options.cwd.as_deref())?;
        let closed_output = execute_bd("list", &["--status=closed".to_string(), "--limit=0".to_string()], options.cwd.as_deref())?;
        let mut issues = parse_issues_tolerant(&open_output, "bd_count_open")?;
        issues.extend(parse_issues_tolerant(&closed_output, "bd_count_closed")?);
        issues
    };

    let mut by_type: HashMap<String, usize> = HashMap::new();
    by_type.insert("bug".to_string(), 0);
    by_type.insert("plan".to_string(), 0);
    by_type.insert("task".to_string(), 0);
    by_type.insert("feature".to_string(), 0);
    by_type.insert("epic".to_string(), 0);
    by_type.insert("chore".to_string(), 0);

    let mut by_priority: HashMap<String, usize> = HashMap::new();
    by_priority.insert("p0".to_string(), 0);
    by_priority.insert("p1".to_string(), 0);
    by_priority.insert("p2".to_string(), 0);
    by_priority.insert("p3".to_string(), 0);
    by_priority.insert("p4".to_string(), 0);

    let mut last_updated: Option<String> = None;

    for issue in &raw_issues {
        let issue_type = issue.issue_type.to_lowercase();
        if by_type.contains_key(&issue_type) {
            *by_type.get_mut(&issue_type).unwrap() += 1;
        }

        let priority_key = format!("p{}", issue.priority);
        if by_priority.contains_key(&priority_key) {
            *by_priority.get_mut(&priority_key).unwrap() += 1;
        }

        if last_updated.is_none() || issue.updated_at > *last_updated.as_ref().unwrap() {
            last_updated = Some(issue.updated_at.clone());
        }
    }

    Ok(CountResult {
        count: raw_issues.len(),
        by_type,
        by_priority,
        last_updated,
    })
}

#[tauri::command]
async fn bd_ready(options: CwdOptions) -> Result<Vec<Issue>, String> {
    log_info!("[bd_ready] Called with cwd: {:?}", options.cwd);

    // Sync database before reading to ensure data is up-to-date
    sync_bd_database(options.cwd.as_deref());

    let output = execute_bd("ready", &[], options.cwd.as_deref())?;

    let raw_issues = parse_issues_tolerant(&output, "bd_ready")?;

    log_info!("[bd_ready] Found {} ready issues", raw_issues.len());
    Ok(raw_issues.into_iter().map(transform_issue).collect())
}

#[tauri::command]
async fn bd_status(options: CwdOptions) -> Result<serde_json::Value, String> {
    let output = execute_bd("status", &[], options.cwd.as_deref())?;

    serde_json::from_str(&output)
        .map_err(|e| format!("Failed to parse status: {}", e))
}

#[tauri::command]
async fn bd_show(id: String, options: CwdOptions) -> Result<Option<Issue>, String> {
    log_info!("[bd_show] Called for issue: {} with cwd: {:?}", id, options.cwd);

    // Sync database before reading to ensure data is up-to-date
    sync_bd_database(options.cwd.as_deref());

    let output = match execute_bd("show", std::slice::from_ref(&id), options.cwd.as_deref()) {
        Ok(output) => output,
        Err(e) => {
            // Handle "not found" errors gracefully (future bd versions may use non-zero exit)
            let err_lower = e.to_lowercase();
            if err_lower.contains("no issue found") || err_lower.contains("not found") {
                log_info!("[bd_show] Issue {} not found (error from bd): {}", id, e);
                return Ok(None);
            }
            return Err(e);
        }
    };

    // Handle empty output (current bd behavior for missing issues: exit 0, empty stdout)
    let trimmed = output.trim();
    if trimmed.is_empty() {
        log_info!("[bd_show] Issue {} not found (empty output from bd)", id);
        return Ok(None);
    }

    // bd show can return either a single object or an array
    let result: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|e| {
            log_error!("[bd_show] Failed to parse JSON for {}: {}", id, e);
            format!("Failed to parse issue: {}", e)
        })?;

    let raw_issue: Option<BdRawIssue> = if result.is_array() {
        result.as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    } else {
        serde_json::from_value(result).ok()
    };

    log_info!("[bd_show] Issue {} found: {}", id, raw_issue.is_some());
    Ok(raw_issue.map(transform_issue))
}

#[tauri::command]
async fn bd_create(payload: CreatePayload) -> Result<Option<Issue>, String> {
    log_info!("[bd_create] Creating issue: {:?}", payload.title);
    let mut args: Vec<String> = vec![payload.title.clone()];

    if let Some(ref desc) = payload.description {
        args.push("--description".to_string());
        args.push(desc.clone());
    }
    if let Some(ref t) = payload.issue_type {
        args.push("--type".to_string());
        args.push(t.clone());
    }
    if let Some(ref p) = payload.priority {
        args.push("--priority".to_string());
        args.push(priority_to_number(p));
    }
    if let Some(ref a) = payload.assignee {
        args.push("--assignee".to_string());
        args.push(a.clone());
    }
    if let Some(ref labels) = payload.labels {
        if !labels.is_empty() {
            args.push("--labels".to_string());
            args.push(labels.join(","));
        }
    }
    if let Some(ref ext) = payload.external_ref {
        args.push("--external-ref".to_string());
        args.push(ext.clone());
    }
    if let Some(est) = payload.estimate_minutes {
        args.push("--estimate".to_string());
        args.push(est.to_string());
    }
    if let Some(ref design) = payload.design_notes {
        args.push("--design".to_string());
        args.push(design.clone());
    }
    if let Some(ref acc) = payload.acceptance_criteria {
        args.push("--acceptance".to_string());
        args.push(acc.clone());
    }
    if let Some(ref notes) = payload.working_notes {
        args.push("--notes".to_string());
        args.push(notes.clone());
    }
    if let Some(ref parent) = payload.parent {
        if !parent.is_empty() {
            args.push("--parent".to_string());
            args.push(parent.clone());
        }
    }
    if let Some(ref spec_id) = payload.spec_id {
        if !spec_id.is_empty() {
            args.push("--spec-id".to_string());
            args.push(spec_id.clone());
        }
    }

    let output = execute_bd("create", &args, payload.cwd.as_deref())?;

    let raw_issue: BdRawIssue = serde_json::from_str(&output)
        .map_err(|e| format!("Failed to parse created issue: {}", e))?;

    Ok(Some(transform_issue(raw_issue)))
}

#[tauri::command]
async fn bd_update(id: String, updates: UpdatePayload) -> Result<Option<Issue>, String> {
    // Always log update calls for debugging (regardless of LOGGING_ENABLED)
    log::info!("[bd_update] Updating issue: {} with cwd: {:?}", id, updates.cwd);
    log::info!("[bd_update] Updates: status={:?}, title={:?}, type={:?}", updates.status, updates.title, updates.issue_type);

    let mut args: Vec<String> = vec![id.clone()];

    if let Some(ref title) = updates.title {
        args.push("--title".to_string());
        args.push(title.clone());
    }
    if let Some(ref desc) = updates.description {
        args.push("--description".to_string());
        args.push(desc.clone());
    }
    if let Some(ref t) = updates.issue_type {
        args.push("--type".to_string());
        args.push(t.clone());
    }
    if let Some(ref s) = updates.status {
        args.push("--status".to_string());
        args.push(s.clone());
    }
    if let Some(ref p) = updates.priority {
        args.push("--priority".to_string());
        args.push(priority_to_number(p));
    }
    if let Some(ref a) = updates.assignee {
        args.push("--assignee".to_string());
        args.push(a.clone());
    }
    if let Some(ref labels) = updates.labels {
        args.push("--set-labels".to_string());
        args.push(labels.join(","));
    }
    if let Some(ref ext) = updates.external_ref {
        args.push("--external-ref".to_string());
        args.push(ext.clone());
    }
    if let Some(est) = updates.estimate_minutes {
        args.push("--estimate".to_string());
        args.push(est.to_string());
    }
    if let Some(ref design) = updates.design_notes {
        args.push("--design".to_string());
        args.push(design.clone());
    }
    if let Some(ref acc) = updates.acceptance_criteria {
        args.push("--acceptance".to_string());
        args.push(acc.clone());
    }
    if let Some(ref notes) = updates.working_notes {
        args.push("--notes".to_string());
        args.push(notes.clone());
    }
    if let Some(ref metadata) = updates.metadata {
        args.push("--metadata".to_string());
        args.push(metadata.clone());
    }
    if let Some(ref spec_id) = updates.spec_id {
        args.push("--spec-id".to_string());
        args.push(spec_id.clone());
    }
    if let Some(ref parent) = updates.parent {
        args.push("--parent".to_string());
        args.push(parent.clone());
    }

    log::info!("[bd_update] Executing: bd update {}", args.join(" "));
    let output = execute_bd("update", &args, updates.cwd.as_deref())?;

    log::info!("[bd_update] Raw output: {}", output.chars().take(500).collect::<String>());

    // Handle empty output from bd CLI (some updates return empty response)
    let trimmed_output = output.trim();
    if trimmed_output.is_empty() {
        log::info!("[bd_update] Empty response from bd, fetching issue {} to get updated data", id);
        // Fetch the updated issue directly
        let show_output = execute_bd("show", std::slice::from_ref(&id), updates.cwd.as_deref())?;
        let show_result: serde_json::Value = serde_json::from_str(&show_output)
            .map_err(|e| {
                log::error!("[bd_update] Failed to parse show JSON: {}", e);
                format!("Failed to fetch updated issue: {}", e)
            })?;

        let raw_issue: Option<BdRawIssue> = if show_result.is_array() {
            show_result.as_array()
                .and_then(|arr| arr.first())
                .and_then(|v| serde_json::from_value(v.clone()).ok())
        } else {
            serde_json::from_value(show_result).ok()
        };

        return Ok(raw_issue.map(transform_issue));
    }

    // bd update can return either a single object or an array
    let result: serde_json::Value = serde_json::from_str(trimmed_output)
        .map_err(|e| {
            log::error!("[bd_update] Failed to parse JSON: {}", e);
            format!("Failed to parse updated issue: {}", e)
        })?;

    let raw_issue: Option<BdRawIssue> = if result.is_array() {
        log::info!("[bd_update] Result is array");
        result.as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    } else {
        log::info!("[bd_update] Result is object");
        serde_json::from_value(result.clone()).map_err(|e| {
            log::error!("[bd_update] Failed to parse issue from result: {}", e);
            e
        }).ok()
    };

    if let Some(ref issue) = raw_issue {
        log::info!("[bd_update] Updated issue {} - new status: {}", id, issue.status);
    } else {
        log::warn!("[bd_update] Could not parse updated issue from response");
    }

    Ok(raw_issue.map(transform_issue))
}

#[tauri::command]
async fn cmux_send_prompt(request: CmuxSendPromptRequest) -> Result<CmuxSendPromptResponse, String> {
    let surface = request.surface.trim().trim_start_matches('{').trim_end_matches('}').to_string();
    validate_cmux_surface_id(&surface)?;
    validate_cmux_prompt(&request.prompt)?;

    log::info!("[cmux] [task-terminal] requested prompt send for surface {}", surface);

    let send_args = cmux_send_prompt_command(&surface, &request.prompt);
    log::info!("[cmux] [task-terminal] trying send command: cmux send --surface {}", surface);
    match run_cmux(&send_args) {
        Ok(output) if output.status.success() => Ok(CmuxSendPromptResponse {
            surface,
            command: send_args.join(" "),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        }),
        Ok(output) => Err(format!(
            "cmux {} failed: {}",
            send_args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim(),
        )),
        Err(error) => {
            log::error!("[cmux] [task-terminal] send command execution error: {}", error);
            Err(error)
        }
    }
}

#[tauri::command]
async fn agent_process_status(request: AgentProcessStatusRequest) -> Result<AgentProcessStatusResponse, String> {
    Ok(classify_agent_process_status(request, probe_process_running))
}

#[tauri::command]
async fn cmux_focus_surface(request: CmuxFocusSurfaceRequest) -> Result<CmuxFocusSurfaceResponse, String> {
    let surface = request.surface.trim().trim_start_matches('{').trim_end_matches('}').to_string();
    validate_cmux_surface_id(&surface)?;

    log::info!("[cmux] [task-terminal] requested focus for surface {}", surface);

    if surface.starts_with("workspace:") {
        let select_args = cmux_select_workspace_command(&surface);
        log::info!("[cmux] [task-terminal] selecting workspace ref: {}", select_args.join(" "));
        return match run_cmux(&select_args) {
            Ok(output) if output.status.success() => Ok(CmuxFocusSurfaceResponse {
                surface,
                command: select_args.join(" "),
                stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            }),
            Ok(output) => Err(format!(
                "cmux {} failed: {}",
                select_args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim(),
            )),
            Err(error) => Err(error),
        };
    }

    let focus_args = cmux_focus_surface_command(&surface);
    log::info!("[cmux] [task-terminal] trying primary command: {}", focus_args.join(" "));
    match run_cmux(&focus_args) {
        Ok(output) if output.status.success() => {
            log::info!("[cmux] [task-terminal] primary command succeeded: {}", focus_args.join(" "));
            return Ok(CmuxFocusSurfaceResponse {
                surface,
                command: focus_args.join(" "),
                stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            });
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            log::warn!("[cmux] [task-terminal] primary command failed: {} stderr={}", focus_args.join(" "), stderr.trim());
        }
        Err(error) => {
            log::warn!("[cmux] [task-terminal] primary command not available, trying fallbacks: {}", error);
        }
    }

    let rpc_args = cmux_focus_surface_rpc_command(&surface);
    log::info!("[cmux] [task-terminal] trying rpc command: {}", rpc_args.join(" "));
    match run_cmux(&rpc_args) {
        Ok(output) if output.status.success() => {
            log::info!("[cmux] [task-terminal] rpc command succeeded: {}", rpc_args.join(" "));
            return Ok(CmuxFocusSurfaceResponse {
                surface,
                command: rpc_args.join(" "),
                stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            });
        }
        Ok(output) => {
            log::warn!(
                "[cmux] [task-terminal] rpc command failed: {} stderr={}",
                rpc_args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Err(error) => {
            log::warn!("[cmux] [task-terminal] rpc command execution error: {}", error);
        }
    }

    let identify_args = cmux_identify_surface_command(&surface);
    log::info!(
        "[cmux] [task-terminal] trying identify command: {}",
        identify_args.join(" ")
    );
    match run_cmux(&identify_args) {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let workspace_ref =
                parse_workspace_ref_from_cmux_identify_output(&stdout).filter(|value| {
                    value.starts_with("workspace:")
                });
            if let Some(workspace_ref) = workspace_ref {
                let select_args = cmux_select_workspace_command(&workspace_ref);
                log::info!(
                    "[cmux] [task-terminal] trying focus with workspace: {}",
                    select_args.join(" ")
                );
                match run_cmux(&select_args) {
                    Ok(output) if output.status.success() => {
                        log::info!(
                            "[cmux] [task-terminal] workspace command succeeded: {}",
                            select_args.join(" ")
                        );
                        return Ok(CmuxFocusSurfaceResponse {
                            surface,
                            command: select_args.join(" "),
                            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
                        });
                    }
                    Ok(output) => {
                        log::warn!(
                            "[cmux] [task-terminal] workspace command failed: {} stderr={}",
                            select_args.join(" "),
                            String::from_utf8_lossy(&output.stderr).trim()
                        );
                    }
                    Err(error) => {
                        log::warn!(
                            "[cmux] [task-terminal] workspace command execution error: {}",
                            error
                        );
                    }
                }
            } else {
                log::warn!(
                    "[cmux] [task-terminal] could not parse workspace ref from identify output"
                );
            }
        }
        Ok(output) => {
            log::warn!(
                "[cmux] [task-terminal] identify command failed: {} stderr={}",
                identify_args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Err(error) => {
            log::warn!("[cmux] [task-terminal] identify command execution error: {}", error);
        }
    }

    let fallback_args = cmux_focus_surface_fallback_command(&surface);
    log::info!("[cmux] [task-terminal] trying fallback command: {}", fallback_args.join(" "));
    match run_cmux(&fallback_args) {
        Ok(output) if output.status.success() => Ok(CmuxFocusSurfaceResponse {
            surface,
            command: fallback_args.join(" "),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        }),
        Ok(output) => Err(format!(
            "cmux {} failed: {}",
            fallback_args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim(),
        )),
        Err(error) => {
            log::error!("[cmux] [task-terminal] fallback command execution error: {}", error);
            Err(error)
        }
    }
}

fn extract_worktree_path_from_error(stderr: &str) -> Option<String> {
    // Parse: "already used by worktree at '/path/to/worktree'"
    let marker = "at '";
    let start = stderr.find(marker)?;
    let rest = &stderr[start + marker.len()..];
    let end = rest.find('\'')?;
    let path = rest[..end].trim();
    if !path.is_empty() && std::path::Path::new(path).exists() {
        Some(path.to_string())
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AutoModeDispatchScope {
    branch: String,
    worktree_name: String,
    workspace_name: String,
    is_epic: bool,
}

fn auto_mode_dispatch_scope(epic_id: &str, issue_id: &str) -> AutoModeDispatchScope {
    let epic_branch = format!("epic/{}", epic_id);
    let epic_worktree_name = format!("epic-{}", epic_id);
    if issue_id == epic_id {
        AutoModeDispatchScope {
            branch: epic_branch,
            worktree_name: epic_worktree_name,
            workspace_name: format!("epic:{}", epic_id),
            is_epic: true,
        }
    } else {
        AutoModeDispatchScope {
            branch: epic_branch,
            worktree_name: epic_worktree_name,
            workspace_name: format!("task:{}", issue_id),
            is_epic: false,
        }
    }
}

fn find_latest_claude_session_id(worktree_path: &str) -> Option<String> {
    let sanitized = worktree_path.replace('/', "-");
    let trimmed = sanitized.trim_start_matches('-');
    let sessions_dir = dirs::home_dir()?.join(".claude").join("projects").join(trimmed);

    if !sessions_dir.is_dir() {
        log::info!("[session-resume] No Claude sessions dir at {}", sessions_dir.display());
        return None;
    }

    let mut newest: Option<(String, std::time::SystemTime)> = None;
    if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".jsonl") {
                let session_id = name.trim_end_matches(".jsonl").to_string();
                if let Ok(meta) = entry.metadata() {
                    if let Ok(modified) = meta.modified() {
                        if newest.as_ref().map_or(true, |(_, t)| modified > *t) {
                            newest = Some((session_id, modified));
                        }
                    }
                }
            }
        }
    }

    if let Some((id, _)) = &newest {
        log::info!("[session-resume] Found latest session {} for {}", id, worktree_path);
    }
    newest.map(|(id, _)| id)
}

fn build_auto_mode_resume_command(worktree_path: &str) -> Option<String> {
    let session_id = find_latest_claude_session_id(worktree_path)?;
    Some(format!("claude --resume {}", shell_single_quote(&session_id)))
}

fn build_auto_mode_executor_command(
    _epic_id: &str,
    issue_id: &str,
    _issue_title: &str,
    _cmux_ref: &str,
) -> String {
    let prompt = format!(
        "Read .claude/auto-mode-prompt.md and follow the instructions exactly. You are executing a SINGLE task only: {issue_id}. Run `br show {issue_id} --json` first, then claim it with `br update --actor auto-mode {issue_id} --status in_progress --claim --json`. Implement the task, run tests, commit, then close with `br close --actor auto-mode {issue_id} --reason \"<what you did>\"` and `br sync --flush-only`. Do NOT loop or pick other tasks.",
    );

    format!("claude {}", shell_single_quote(&prompt))
}

fn build_auto_mode_epic_orchestrator_command(epic_id: &str) -> String {
    let prompt = format!(
        "Read .claude/auto-mode-prompt.md and follow the instructions exactly. Replace EPIC_ID with {epic_id}. The br CLI is your issue tracker — use `br` (never `bd`). Always pass `--actor auto-mode` and `--json` flags. Loop through all open tasks in the epic, claim each before working, close each when done with evidence. Do NOT skip the claim or close steps.",
    );

    format!("claude {}", shell_single_quote(&prompt))
}

fn build_auto_mode_reviewer_command(
    epic_id: &str,
    issue_id: &str,
    issue_title: &str,
    task_branch: &str,
    executor_commit: &str,
    cmux_ref: &str,
) -> String {
    let assignee = format!("cmux:{}", cmux_ref);
    let prompt = format!(
        "You are an independent reviewer for Beads task {issue_id} in epic {epic_id}. Task title: {issue_title}. You have fresh context — no bias from the executor. Branch: `{task_branch}`, executor commit: {executor_commit}. Steps: 1) Run `br show {issue_id}` to understand requirements. 2) Run `git log --oneline master..{task_branch}` and `git diff master...{task_branch}` to see all changes. 3) Run quality gates: `pnpm test` and `npx vue-tsc --noEmit`. 4) Review the diff against the task requirements — check correctness, test coverage, no unrelated changes, no security issues. 5) Add a structured BR comment with your verdict: `br comments add --actor auto-mode {issue_id} --message 'REVIEW_VERDICT: APPROVED|CHANGES_REQUESTED\\nSummary: ...\\nFindings: ...' --json`. Use assignee {assignee}. If tests or type-check fail, verdict must be CHANGES_REQUESTED with failure details. Do not modify code — only validate and report.",
    );

    format!("claude {}", shell_single_quote(&prompt))
}

fn build_auto_mode_agent_command(
    epic_id: &str,
    issue_id: &str,
    issue_title: &str,
    cmux_ref: &str,
) -> String {
    if issue_id == epic_id {
        build_auto_mode_epic_orchestrator_command(epic_id)
    } else {
        build_auto_mode_executor_command(epic_id, issue_id, issue_title, cmux_ref)
    }
}

fn mark_auto_mode_issue_dispatched(project_path: &str, issue_id: &str, cmux_ref: &str) -> Result<(), String> {
    let assignee = format!("cmux:{}", cmux_ref);
    execute_bd(
        "update",
        &[
            issue_id.to_string(),
            "--status=in_progress".to_string(),
            format!("--assignee={}", assignee),
        ],
        Some(project_path),
    )?;
    Ok(())
}

fn has_in_progress_non_epic_issue(issues: &[BdRawIssue]) -> bool {
    issues
        .iter()
        .any(|issue| issue.status == "in_progress" && issue.issue_type != "epic")
}

fn project_has_in_progress_auto_mode_task(project_path: &str) -> Result<bool, String> {
    let output = execute_bd(
        "list",
        &["--all".to_string(), "--limit=0".to_string()],
        Some(project_path),
    )?;
    let issues = parse_issues_tolerant(&output, "auto_mode_dispatch_in_progress")?;

    Ok(has_in_progress_non_epic_issue(&issues))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModeDispatchRequest {
    pub project_path: String,
    pub issue_id: String,
    pub issue_title: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModeDispatchResponse {
    pub surface: String,
    pub worktree_path: String,
    pub branch: String,
}

#[tauri::command]
fn caffeinate_start() -> Result<bool, String> {
    let mut guard = CAFFEINATE_CHILD.lock().map_err(|e| e.to_string())?;
    if let Some(ref mut child) = *guard {
        match child.try_wait() {
            Ok(Some(_)) => { /* exited, will replace below */ }
            Ok(None) => {
                log::info!("[caffeinate] Already running (pid {})", child.id());
                return Ok(false);
            }
            Err(_) => { /* assume dead, replace */ }
        }
    }
    let child = new_command("caffeinate")
        .arg("-i")
        .spawn()
        .map_err(|e| format!("Failed to spawn caffeinate: {}", e))?;
    log::info!("[caffeinate] Started (pid {})", child.id());
    *guard = Some(child);
    Ok(true)
}

#[tauri::command]
fn caffeinate_stop() -> Result<bool, String> {
    let mut guard = CAFFEINATE_CHILD.lock().map_err(|e| e.to_string())?;
    if let Some(mut child) = guard.take() {
        let pid = child.id();
        let _ = child.kill();
        let _ = child.wait();
        log::info!("[caffeinate] Stopped (pid {})", pid);
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
async fn auto_mode_dispatch(request: AutoModeDispatchRequest) -> Result<AutoModeDispatchResponse, String> {
    let issue_id = &request.issue_id;

    // Derive epic ID: "borabr-unf.1" → "borabr-unf", epic itself stays as-is
    let epic_id = issue_id.rfind('.').map(|pos| &issue_id[..pos]).unwrap_or(issue_id);
    let scope = auto_mode_dispatch_scope(epic_id, issue_id);
    let branch = scope.branch.clone();

    // Resolve project root (follow worktree back to main repo)
    let project_path_raw = &request.project_path;
    let git_root_output = new_command("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(project_path_raw)
        .env("PATH", get_extended_path())
        .output()
        .map_err(|e| format!("Failed to find git root: {}", e))?;
    let git_root = if git_root_output.status.success() {
        String::from_utf8_lossy(&git_root_output.stdout).trim().to_string()
    } else {
        project_path_raw.clone()
    };
    let project_path = &git_root;

    if project_has_in_progress_auto_mode_task(project_path)? {
        return Err("auto-mode dispatch blocked: project already has in-progress work".to_string());
    }

    // Worktree path: worktrees/<scope> (task work uses task-<issue-id>)
    let worktrees_parent = format!("{}/../worktrees", project_path);
    let worktrees_parent_path = std::path::Path::new(&worktrees_parent);
    if !worktrees_parent_path.exists() {
        std::fs::create_dir_all(worktrees_parent_path)
            .map_err(|e| format!("Failed to create worktrees dir: {}", e))?;
    }
    let canonical_parent = worktrees_parent_path.canonicalize()
        .map_err(|e| format!("Failed to canonicalize worktrees dir: {}", e))?;
    let canonical_worktree = canonical_parent.join(&scope.worktree_name);
    let mut worktree_dir = canonical_worktree.to_string_lossy().to_string();

    log::info!("[auto-mode] Dispatching epic {} (task {}) to worktree {}", epic_id, issue_id, worktree_dir);

    // 1. Ensure scoped worktree exists.
    if canonical_worktree.exists() {
        log::info!("[auto-mode] Scoped worktree already exists, reusing: {}", worktree_dir);
    } else {
        let worktree_output = new_command("git")
            .args(["worktree", "add", "-b", &branch, &worktree_dir, "HEAD"])
            .current_dir(project_path)
            .env("PATH", get_extended_path())
            .output()
            .map_err(|e| format!("Failed to create worktree: {}", e))?;

        if !worktree_output.status.success() {
            let stderr = String::from_utf8_lossy(&worktree_output.stderr).to_string();
            if let Some(existing) = extract_worktree_path_from_error(&stderr) {
                log::info!("[auto-mode] Reusing existing worktree at {}", existing);
                worktree_dir = existing;
            } else if stderr.contains("already exists") {
                let retry = new_command("git")
                    .args(["worktree", "add", &worktree_dir, &branch])
                    .current_dir(project_path)
                    .env("PATH", get_extended_path())
                    .output()
                    .map_err(|e| format!("Failed to create worktree (retry): {}", e))?;
                if !retry.status.success() {
                    let retry_err = String::from_utf8_lossy(&retry.stderr).to_string();
                    if let Some(existing) = extract_worktree_path_from_error(&retry_err) {
                        worktree_dir = existing;
                    } else {
                        return Err(format!("git worktree failed: {}", retry_err.trim()));
                    }
                }
            } else {
                return Err(format!("git worktree failed: {}", stderr.trim()));
            }
        }
        log::info!("[auto-mode] Scoped worktree ready: {}", worktree_dir);
    }

    // 2. Symlink .beads/ from main repo so worktree shares issue state
    let worktree_beads = std::path::Path::new(&worktree_dir).join(".beads");
    let main_beads = std::path::Path::new(project_path).join(".beads");
    if main_beads.exists() && !worktree_beads.exists() {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&main_beads, &worktree_beads)
                .map_err(|e| format!("Failed to symlink .beads: {}", e))?;
            log::info!("[auto-mode] Symlinked .beads/ → {}", main_beads.display());
        }
    } else if worktree_beads.is_dir() && !worktree_beads.is_symlink() {
        log::info!("[auto-mode] Replacing .beads/ dir with symlink to main repo");
        std::fs::remove_dir_all(&worktree_beads)
            .map_err(|e| format!("Failed to remove worktree .beads/: {}", e))?;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&main_beads, &worktree_beads)
                .map_err(|e| format!("Failed to symlink .beads: {}", e))?;
        }
    }

    // 3. Check if cmux workspace already exists for this scope.
    let workspace_name = scope.workspace_name.clone();
    let mut workspace_ref: Option<String> = None;
    let mut reused_workspace = false;
    let list_output = run_cmux(&["list-workspaces".to_string()]);
    if let Ok(ref output) = list_output {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(existing_ref) = stdout.lines()
                .find(|line| line.contains(&workspace_name) || line.contains(issue_id))
                .and_then(|line| line.split_whitespace().find(|w| w.starts_with("workspace:")))
            {
                let existing_ref = existing_ref.to_string();
                log::info!("[auto-mode] Scoped workspace already exists: {}", existing_ref);
                let _ = run_cmux(&[
                    "select-workspace".to_string(),
                    "--workspace".to_string(), existing_ref.clone(),
                ]);
                reused_workspace = true;
                workspace_ref = Some(existing_ref);
            }
        }
    }

    // 4. Create cmux workspace for this dispatch scope.
    let workspace_ref = match workspace_ref {
        Some(existing_ref) => existing_ref,
        None => {
            let cmux_create_args = vec![
                "new-workspace".to_string(),
                "--name".to_string(), workspace_name,
                "--cwd".to_string(), worktree_dir.clone(),
            ];
            let cmux_output = run_cmux(&cmux_create_args)?;
            if !cmux_output.status.success() {
                let stderr = String::from_utf8_lossy(&cmux_output.stderr);
                return Err(format!("cmux new-workspace failed: {}", stderr.trim()));
            }
            let stdout = String::from_utf8_lossy(&cmux_output.stdout).trim().to_string();
            log::info!("[auto-mode] cmux workspace created: {}", stdout);

            stdout.split_whitespace()
                .find(|s| s.starts_with("workspace:"))
                .ok_or_else(|| format!("cmux new-workspace returned no workspace ref: {}", stdout))?
                .to_string()
        }
    };

    // 5. Send Claude command for the selected dispatch scope.
    let fresh_command = build_auto_mode_agent_command(epic_id, issue_id, &request.issue_title, &workspace_ref);
    let orchestrator_command = if reused_workspace {
        build_auto_mode_resume_command(&worktree_dir).unwrap_or(fresh_command)
    } else {
        fresh_command
    };
    let cmux_send_args = vec![
        "send".to_string(),
        "--workspace".to_string(), workspace_ref.clone(),
        format!("{}\\n", orchestrator_command),
    ];
    log::info!("[auto-mode] Sending orchestrator to {}", workspace_ref);
    let send_output = run_cmux(&cmux_send_args);
    match &send_output {
        Ok(o) if o.status.success() => log::info!("[auto-mode] cmux send succeeded"),
        Ok(o) => return Err(format!("cmux send failed: {}", String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) => return Err(format!("cmux send error: {}", e)),
    }

    if !scope.is_epic {
        mark_auto_mode_issue_dispatched(project_path, issue_id, &workspace_ref)?;
    }

    let surface = workspace_ref;

    Ok(AutoModeDispatchResponse {
        surface,
        worktree_path: worktree_dir,
        branch,
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModeDispatchReviewRequest {
    pub project_path: String,
    pub issue_id: String,
    pub issue_title: String,
    pub task_branch: String,
    pub executor_commit: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModeDispatchReviewResponse {
    pub surface: String,
    pub workspace_name: String,
}

#[tauri::command]
async fn auto_mode_dispatch_review(request: AutoModeDispatchReviewRequest) -> Result<AutoModeDispatchReviewResponse, String> {
    let issue_id = &request.issue_id;
    let epic_id = issue_id.rfind('.').map(|pos| &issue_id[..pos]).unwrap_or(issue_id);

    let project_path_raw = &request.project_path;
    let git_root_output = new_command("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(project_path_raw)
        .env("PATH", get_extended_path())
        .output()
        .map_err(|e| format!("Failed to find git root: {}", e))?;
    let project_path = if git_root_output.status.success() {
        String::from_utf8_lossy(&git_root_output.stdout).trim().to_string()
    } else {
        project_path_raw.clone()
    };

    let scope = auto_mode_dispatch_scope(epic_id, issue_id);
    let worktrees_parent = format!("{}/../worktrees", project_path);
    let canonical_parent = std::path::Path::new(&worktrees_parent).canonicalize()
        .map_err(|e| format!("Failed to canonicalize worktrees dir: {}", e))?;
    let worktree_dir = canonical_parent.join(&scope.worktree_name);
    if !worktree_dir.exists() {
        return Err(format!("Task worktree does not exist: {}", worktree_dir.display()));
    }
    let worktree_dir_str = worktree_dir.to_string_lossy().to_string();

    let review_workspace_name = format!("review:{}", issue_id);
    log::info!("[auto-mode] [review-gate] Dispatching reviewer for {} in {}", issue_id, worktree_dir_str);

    // Check if review workspace already exists
    let mut workspace_ref: Option<String> = None;
    let list_output = run_cmux(&["list-workspaces".to_string()]);
    if let Ok(ref output) = list_output {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(existing_ref) = stdout.lines()
                .find(|line| line.contains(&review_workspace_name))
                .and_then(|line| line.split_whitespace().find(|w| w.starts_with("workspace:")))
            {
                let existing_ref = existing_ref.to_string();
                log::info!("[auto-mode] [review-gate] Reusing review workspace: {}", existing_ref);
                workspace_ref = Some(existing_ref);
            }
        }
    }

    // Create review workspace if needed
    let workspace_ref = match workspace_ref {
        Some(existing) => existing,
        None => {
            let cmux_create_args = vec![
                "new-workspace".to_string(),
                "--name".to_string(), review_workspace_name.clone(),
                "--cwd".to_string(), worktree_dir_str.clone(),
            ];
            let cmux_output = run_cmux(&cmux_create_args)?;
            if !cmux_output.status.success() {
                let stderr = String::from_utf8_lossy(&cmux_output.stderr);
                return Err(format!("cmux new-workspace failed for review: {}", stderr.trim()));
            }
            let stdout = String::from_utf8_lossy(&cmux_output.stdout).trim().to_string();
            log::info!("[auto-mode] [review-gate] Review workspace created: {}", stdout);

            stdout.split_whitespace()
                .find(|s| s.starts_with("workspace:"))
                .ok_or_else(|| format!("cmux new-workspace (review) returned no workspace ref: {}", stdout))?
                .to_string()
        }
    };

    // Send reviewer command
    let reviewer_command = build_auto_mode_reviewer_command(
        epic_id,
        issue_id,
        &request.issue_title,
        &request.task_branch,
        &request.executor_commit,
        &workspace_ref,
    );
    let cmux_send_args = vec![
        "send".to_string(),
        "--workspace".to_string(), workspace_ref.clone(),
        format!("{}\\n", reviewer_command),
    ];
    log::info!("[auto-mode] [review-gate] Sending reviewer to {}", workspace_ref);
    let send_output = run_cmux(&cmux_send_args);
    match &send_output {
        Ok(o) if o.status.success() => log::info!("[auto-mode] [review-gate] cmux send succeeded"),
        Ok(o) => return Err(format!("cmux send failed for review: {}", String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) => return Err(format!("cmux send error for review: {}", e)),
    }

    Ok(AutoModeDispatchReviewResponse {
        surface: workspace_ref,
        workspace_name: review_workspace_name,
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModeMergeApprovedRequest {
    pub project_path: String,
    pub issue_id: String,
    pub task_branch: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModeMergeApprovedResponse {
    pub merged: bool,
    pub closed: bool,
    pub worktree_removed: bool,
}

#[tauri::command]
async fn auto_mode_merge_approved(request: AutoModeMergeApprovedRequest) -> Result<AutoModeMergeApprovedResponse, String> {
    let issue_id = &request.issue_id;
    let task_branch = &request.task_branch;

    let project_path_raw = &request.project_path;
    let git_root_output = new_command("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(project_path_raw)
        .env("PATH", get_extended_path())
        .output()
        .map_err(|e| format!("Failed to find git root: {}", e))?;
    let project_path = if git_root_output.status.success() {
        String::from_utf8_lossy(&git_root_output.stdout).trim().to_string()
    } else {
        project_path_raw.clone()
    };

    log::info!("[auto-mode] [merge] Merging approved branch {} for {}", task_branch, issue_id);

    // 1. Merge task branch into current branch (main)
    let merge_output = new_command("git")
        .args(["merge", "--no-ff", task_branch, "-m", &format!("merge: {} from branch {}", issue_id, task_branch)])
        .current_dir(&project_path)
        .env("PATH", get_extended_path())
        .output()
        .map_err(|e| format!("git merge failed to execute: {}", e))?;

    if !merge_output.status.success() {
        let stderr = String::from_utf8_lossy(&merge_output.stderr).to_string();
        // Abort the merge if it conflicted
        let _ = new_command("git")
            .args(["merge", "--abort"])
            .current_dir(&project_path)
            .env("PATH", get_extended_path())
            .output();
        return Err(format!("Merge failed (aborted): {}", stderr.trim()));
    }
    log::info!("[auto-mode] [merge] Branch {} merged successfully", task_branch);

    // 2. Close BR issue
    let epic_id = issue_id.rfind('.').map(|pos| &issue_id[..pos]).unwrap_or(issue_id);
    let close_result = execute_bd(
        "close",
        &[issue_id.to_string(), "--reason".to_string(), format!("Merged {} into main", task_branch)],
        Some(&project_path),
    );
    let closed = close_result.is_ok();
    if closed {
        log::info!("[auto-mode] [merge] Issue {} closed", issue_id);
    } else {
        log::warn!("[auto-mode] [merge] Failed to close {}: {:?}", issue_id, close_result.err());
    }

    // 3. Remove worktree
    let scope = auto_mode_dispatch_scope(epic_id, issue_id);
    let worktrees_parent = format!("{}/../worktrees", project_path);
    let worktree_removed = if let Ok(canonical_parent) = std::path::Path::new(&worktrees_parent).canonicalize() {
        let worktree_path = canonical_parent.join(&scope.worktree_name);
        if worktree_path.exists() {
            let remove_output = new_command("git")
                .args(["worktree", "remove", &worktree_path.to_string_lossy()])
                .current_dir(&project_path)
                .env("PATH", get_extended_path())
                .output();
            match remove_output {
                Ok(o) if o.status.success() => {
                    log::info!("[auto-mode] [merge] Worktree {} removed", scope.worktree_name);
                    true
                }
                _ => {
                    log::warn!("[auto-mode] [merge] Failed to remove worktree {}", scope.worktree_name);
                    false
                }
            }
        } else {
            true
        }
    } else {
        false
    };

    // 4. Delete task branch (safe — already merged)
    let _ = new_command("git")
        .args(["branch", "-d", task_branch])
        .current_dir(&project_path)
        .env("PATH", get_extended_path())
        .output();

    // 5. Flush BR sync
    let _ = execute_bd("sync", &["--flush-only".to_string()], Some(&project_path));

    Ok(AutoModeMergeApprovedResponse {
        merged: true,
        closed,
        worktree_removed,
    })
}

// ============================================================================
// Auto-Mode Activity Log
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModeLogEntry {
    pub timestamp: String,
    pub issue_id: String,
    pub event_type: String,
    pub detail: String,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModeLogRecord {
    pub timestamp: String,
    pub issue_id: String,
    pub event_type: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn auto_mode_log_path(project_path: &str) -> PathBuf {
    std::path::Path::new(project_path).join(".beads").join("auto-mode-log.jsonl")
}

#[tauri::command]
async fn auto_mode_log_append(project_path: String, entry: AutoModeLogEntry) -> Result<(), String> {
    let log_path = auto_mode_log_path(&project_path);

    if let Some(parent) = log_path.parent() {
        if !parent.exists() {
            return Err("Project .beads/ directory does not exist".to_string());
        }
    }

    let record = AutoModeLogRecord {
        timestamp: entry.timestamp,
        issue_id: entry.issue_id,
        event_type: entry.event_type,
        detail: entry.detail,
        surface: entry.surface,
        error: entry.error,
    };

    let mut line = serde_json::to_string(&record)
        .map_err(|e| format!("Failed to serialize log entry: {}", e))?;
    line.push('\n');

    use std::io::Write;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("Failed to open log file: {}", e))?;
    let mut writer = std::io::BufWriter::new(file);
    writer.write_all(line.as_bytes())
        .map_err(|e| format!("Failed to write log entry: {}", e))?;
    writer.flush()
        .map_err(|e| format!("Failed to flush log file: {}", e))?;

    Ok(())
}

#[tauri::command]
async fn auto_mode_log_read(project_path: String, limit: Option<usize>) -> Result<Vec<AutoModeLogRecord>, String> {
    let log_path = auto_mode_log_path(&project_path);
    if !log_path.exists() {
        return Ok(vec![]);
    }

    let content = fs::read_to_string(&log_path)
        .map_err(|e| format!("Failed to read log file: {}", e))?;

    let mut records: Vec<AutoModeLogRecord> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    if let Some(limit) = limit {
        let skip = records.len().saturating_sub(limit);
        records = records.into_iter().skip(skip).collect();
    }

    Ok(records)
}

#[tauri::command]
async fn auto_mode_log_clear(project_path: String) -> Result<(), String> {
    let log_path = auto_mode_log_path(&project_path);
    if log_path.exists() {
        fs::write(&log_path, "")
            .map_err(|e| format!("Failed to clear log file: {}", e))?;
    }
    Ok(())
}


#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModeCancelTaskRequest {
    pub project_path: String,
    pub issue_id: String,
    pub surface: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModeCancelTaskResponse {
    pub workspace_closed: bool,
    pub issue_reset: bool,
}

#[tauri::command]
async fn auto_mode_cancel_task(request: AutoModeCancelTaskRequest) -> Result<AutoModeCancelTaskResponse, String> {
    let issue_id = &request.issue_id;

    let project_path_raw = &request.project_path;
    let git_root_output = new_command("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(project_path_raw)
        .env("PATH", get_extended_path())
        .output()
        .map_err(|e| format!("Failed to find git root: {}", e))?;
    let project_path = if git_root_output.status.success() {
        String::from_utf8_lossy(&git_root_output.stdout).trim().to_string()
    } else {
        project_path_raw.clone()
    };

    log::info!("[auto-mode] [cancel] Cancelling task {}", issue_id);

    // 1. Close cmux workspace (try provided surface first, then search by workspace name)
    let mut workspace_closed = false;
    let workspace_ref = if let Some(ref surface) = request.surface {
        Some(surface.clone())
    } else {
        let epic_id = issue_id.rfind('.').map(|pos| &issue_id[..pos]).unwrap_or(issue_id);
        let scope = auto_mode_dispatch_scope(epic_id, issue_id);
        let list_output = run_cmux(&["list-workspaces".to_string()]);
        if let Ok(ref output) = list_output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.lines()
                    .find(|line| line.contains(&scope.workspace_name) || line.contains(issue_id))
                    .and_then(|line| line.split_whitespace().find(|w| w.starts_with("workspace:")))
                    .map(|s| s.to_string())
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(ref ws_ref) = workspace_ref {
        let close_output = run_cmux(&[
            "close-workspace".to_string(),
            "--workspace".to_string(),
            ws_ref.clone(),
        ]);
        match close_output {
            Ok(o) if o.status.success() => {
                log::info!("[auto-mode] [cancel] Closed workspace {}", ws_ref);
                workspace_closed = true;
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                log::warn!("[auto-mode] [cancel] Failed to close workspace {}: {}", ws_ref, stderr.trim());
            }
            Err(e) => {
                log::warn!("[auto-mode] [cancel] cmux close error: {}", e);
            }
        }
    } else {
        log::info!("[auto-mode] [cancel] No workspace ref found for {}, skipping close", issue_id);
    }

    // 2. Reset issue status back to open
    let mut issue_reset = false;
    let reset_result = execute_bd(
        "update",
        &[
            issue_id.to_string(),
            "--status=open".to_string(),
            "--assignee=".to_string(),
        ],
        Some(&project_path),
    );
    match reset_result {
        Ok(_) => {
            log::info!("[auto-mode] [cancel] Issue {} reset to open", issue_id);
            issue_reset = true;
        }
        Err(e) => {
            log::warn!("[auto-mode] [cancel] Failed to reset issue {}: {}", issue_id, e);
        }
    }

    // 3. Flush BR sync
    let _ = execute_bd("sync", &["--flush-only".to_string()], Some(&project_path));

    Ok(AutoModeCancelTaskResponse {
        workspace_closed,
        issue_reset,
    })
}

#[tauri::command]
async fn terminal_native_renderer_capabilities() -> Result<TerminalNativeRendererCapabilitiesResponse, String> {
    Ok(detect_native_terminal_renderer_capabilities())
}

#[tauri::command]
async fn terminal_open_native_renderer(
    request: OpenNativeTerminalRendererRequest,
) -> Result<OpenNativeTerminalRendererResponse, String> {
    let cwd = validate_native_terminal_cwd(&request.cwd)?;
    let cwd = cwd.to_string_lossy().to_string();
    let shell = request.shell.unwrap_or_else(default_native_shell);
    let session_id = native_terminal_session_id();
    let app_path = if cfg!(target_os = "macos") {
        find_ghostty_app()
    } else {
        None
    };
    let plan = build_native_terminal_launch_plan(
        std::env::consts::OS,
        app_path,
        &cwd,
        request.issue_id.as_deref(),
        &shell,
        &session_id,
    )?;

    let command_summary = format!("{} {}", plan.program, plan.args.join(" "));
    let mut child = new_command(&plan.program)
        .args(&plan.args)
        .env("PATH", get_extended_path())
        .spawn()
        .map_err(|e| format!("Failed to launch Ghostty native renderer: {}", e))?;
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(OpenNativeTerminalRendererResponse {
        renderer: "ghostty-external".to_string(),
        session_id,
        command: command_summary,
        pid: Some(pid),
    })
}

#[tauri::command]
async fn bd_close(id: String, options: CwdOptions) -> Result<serde_json::Value, String> {
    log_info!("[bd_close] Closing issue: {} with cwd: {:?}", id, options.cwd);

    let mut args = vec![id.clone()];
    // br supports --suggest-next for showing newly unblocked issues
    if matches!(get_cli_client_info(), Some((CliClient::Br, _, _, _))) {
        args.push("--suggest-next".to_string());
    }

    let output = execute_bd("close", &args, options.cwd.as_deref())?;

    log_info!("[bd_close] Raw output: {}", output.chars().take(500).collect::<String>());

    let result: serde_json::Value = serde_json::from_str(&output)
        .map_err(|e| {
            log_error!("[bd_close] Failed to parse JSON: {}", e);
            format!("Failed to parse close result: {}", e)
        })?;

    log_info!("[bd_close] Issue {} closed successfully", id);
    Ok(result)
}

#[tauri::command]
async fn bd_search(query: String, options: CwdOptions) -> Result<Vec<Issue>, String> {
    log_info!("[bd_search] Searching for: {} with cwd: {:?}", query, options.cwd);

    let args = vec![query];
    let output = execute_bd("search", &args, options.cwd.as_deref())?;

    log_info!("[bd_search] Raw output: {}", output.chars().take(500).collect::<String>());

    let trimmed = output.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(vec![]);
    }

    let raw: Vec<BdRawIssue> = serde_json::from_str(trimmed)
        .map_err(|e| {
            log_error!("[bd_search] Failed to parse JSON: {}", e);
            format!("Failed to parse search results: {}", e)
        })?;

    Ok(raw.into_iter().map(transform_issue).collect())
}

#[tauri::command]
async fn bd_label_add(id: String, label: String, options: CwdOptions) -> Result<(), String> {
    log_info!("[bd_label_add] Adding label '{}' to issue {}", label, id);
    let args = vec![id, label];
    execute_bd("label add", &args, options.cwd.as_deref())?;
    Ok(())
}

#[tauri::command]
async fn bd_label_remove(id: String, label: String, options: CwdOptions) -> Result<(), String> {
    log_info!("[bd_label_remove] Removing label '{}' from issue {}", label, id);
    let args = vec![id, label];
    execute_bd("label remove", &args, options.cwd.as_deref())?;
    Ok(())
}

#[tauri::command]
async fn bd_delete(id: String, options: CwdOptions) -> Result<serde_json::Value, String> {
    let mut args = vec![id.clone(), "--force".to_string()];
    if supports_delete_hard_flag() {
        args.push("--hard".to_string());
    }
    log::info!("[bd_delete] Deleting issue: {} with args: {:?}", id, args);
    execute_bd("delete", &args, options.cwd.as_deref())?;

    // Sync after delete to push deletion to remote and prevent resurrection
    sync_bd_database(options.cwd.as_deref());

    // Clean up attachments folder for this issue
    let project_path = options.cwd.as_deref().unwrap_or(".");
    let abs_project_path = if project_path == "." || project_path.is_empty() {
        env::current_dir().ok()
    } else {
        let p = PathBuf::from(project_path);
        if p.is_relative() {
            env::current_dir().ok().map(|cwd| cwd.join(&p))
        } else {
            Some(p)
        }
    };

    if let Some(path) = abs_project_path {
        if let Ok(abs_path) = path.canonicalize() {
            let att_dir = abs_path.join(".beads").join("attachments").join(issue_short_id(&id));
            if att_dir.exists() && att_dir.is_dir() {
                if let Err(e) = fs::remove_dir_all(&att_dir) {
                    log::warn!("[bd_delete] Failed to remove attachments folder: {}", e);
                } else {
                    log::info!("[bd_delete] Removed attachments folder: {:?}", att_dir);
                }
            }
        }
    }

    Ok(serde_json::json!({ "success": true, "id": id }))
}

#[tauri::command]
async fn bd_comments_add(id: String, content: String, options: CwdOptions) -> Result<serde_json::Value, String> {
    let args = vec![id, content];

    execute_bd("comments add", &args, options.cwd.as_deref())?;

    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
async fn bd_dep_add(issue_id: String, blocker_id: String, options: CwdOptions) -> Result<serde_json::Value, String> {
    let args = vec![issue_id, blocker_id];

    execute_bd("dep add", &args, options.cwd.as_deref())?;

    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
async fn bd_dep_remove(issue_id: String, blocker_id: String, options: CwdOptions) -> Result<serde_json::Value, String> {
    let args = vec![issue_id, blocker_id];

    execute_bd("dep remove", &args, options.cwd.as_deref())?;

    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
async fn bd_dep_add_relation(id1: String, id2: String, relation_type: String, options: CwdOptions) -> Result<serde_json::Value, String> {
    let args = vec![id1, id2, "--type".to_string(), relation_type];

    execute_bd("dep add", &args, options.cwd.as_deref())?;

    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
async fn bd_dep_remove_relation(id1: String, id2: String, options: CwdOptions) -> Result<serde_json::Value, String> {
    let args = vec![id1, id2];

    execute_bd("dep remove", &args, options.cwd.as_deref())?;

    Ok(serde_json::json!({ "success": true }))
}

#[tauri::command]
async fn bd_available_relation_types() -> Vec<serde_json::Value> {
    let common: Vec<(&str, &str)> = vec![
        ("relates-to", "Relates To"),
        ("related", "Related"),
        ("discovered-from", "Discovered From"),
        ("duplicates", "Duplicates"),
        ("supersedes", "Supersedes"),
        ("caused-by", "Caused By"),
        ("replies-to", "Replies To"),
    ];
    let bd_only: Vec<(&str, &str)> = vec![
        ("tracks", "Tracks"),
        ("until", "Until"),
        ("validates", "Validates"),
    ];

    let types = match get_cli_client_info() {
        Some((CliClient::Br, _, _, _)) => common,
        _ => {
            let mut all = common;
            all.extend(bd_only);
            all
        }
    };

    types.into_iter().map(|(v, l)| serde_json::json!({ "value": v, "label": l })).collect()
}

fn expand_user_path(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn run_git(cwd: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let output = new_command("git")
        .args(args)
        .current_dir(cwd)
        .env("PATH", get_extended_path())
        .output()
        .map_err(|e| format!("Failed to execute git: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("git {} failed with status {}", args.join(" "), output.status)
        } else {
            stderr
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_git_worktree_branch(raw: &str) -> Option<String> {
    let branch = raw.strip_prefix("refs/heads/").unwrap_or(raw).trim();
    if branch.is_empty() { None } else { Some(branch.to_string()) }
}

fn parse_git_worktree_porcelain(output: &str) -> Vec<ParsedGitWorktree> {
    let mut worktrees = Vec::new();
    let mut current: Option<ParsedGitWorktree> = None;

    for line in output.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            if let Some(worktree) = current.take() {
                worktrees.push(worktree);
            }
            continue;
        }

        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(worktree) = current.take() {
                worktrees.push(worktree);
            }
            current = Some(ParsedGitWorktree {
                path: path.to_string(),
                branch: None,
                head: None,
                prunable: false,
            });
            continue;
        }

        let Some(worktree) = current.as_mut() else { continue; };
        if let Some(head) = line.strip_prefix("HEAD ") {
            worktree.head = Some(head.to_string());
        } else if let Some(branch) = line.strip_prefix("branch ") {
            worktree.branch = parse_git_worktree_branch(branch);
        } else if line == "prunable" || line.starts_with("prunable ") {
            worktree.prunable = true;
        }
    }

    if let Some(worktree) = current.take() {
        worktrees.push(worktree);
    }
    worktrees
}

fn normalize_git_remote_url(remote: &str) -> String {
    let mut value = remote.trim().trim_end_matches('/').to_string();
    if let Some(stripped) = value.strip_suffix(".git") {
        value = stripped.to_string();
    }

    if let Some(rest) = value.strip_prefix("git@github.com:") {
        return format!("github.com/{}", rest).to_lowercase();
    }
    if let Some(rest) = value.strip_prefix("ssh://git@github.com/") {
        return format!("github.com/{}", rest).to_lowercase();
    }
    if let Some(rest) = value.strip_prefix("https://github.com/") {
        return format!("github.com/{}", rest).to_lowercase();
    }
    if let Some(rest) = value.strip_prefix("http://github.com/") {
        return format!("github.com/{}", rest).to_lowercase();
    }
    value.to_lowercase()
}

fn github_owner_repo_from_remote(remote: &str) -> Option<(String, String)> {
    let normalized = normalize_git_remote_url(remote);
    let rest = normalized.strip_prefix("github.com/")?;
    let mut parts = rest.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() {
        None
    } else {
        Some((owner.to_string(), repo.to_string()))
    }
}

fn git_remote_url(cwd: &std::path::Path) -> Option<String> {
    run_git(cwd, &["config", "--get", "remote.origin.url"])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn git_short_branch(cwd: &std::path::Path) -> Option<String> {
    run_git(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn git_head(cwd: &std::path::Path) -> Option<String> {
    run_git(cwd, &["rev-parse", "HEAD"])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

const WORKTREE_ACTIVITY_SCAN_MAX_ENTRIES: usize = 5_000;
const WORKTREE_ACTIVITY_SCAN_MAX_MS: u64 = 150;
const RECENT_ACTIVITY_WORKTREE_LIMIT: usize = 5;
const GITHUB_PR_SIGNAL_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const GITHUB_PR_SIGNAL_MAX_BRANCHES_PER_DISCOVERY: usize = 20;
const GITHUB_RECENT_MERGED_WINDOW_MS: u64 = 14 * 24 * 60 * 60 * 1_000;
const JS_MAX_SAFE_INTEGER_MILLIS: u64 = 9_007_199_254_740_991;
const LINEAR_ACTION_CENTER_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

fn system_time_epoch_millis(time: std::time::SystemTime) -> Option<u64> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

fn is_ignored_worktree_activity_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | "dist"
            | "build"
            | "target"
            | ".next"
            | ".nuxt"
            | ".output"
            | ".cache"
            | ".turbo"
            | "coverage"
            | "vendor"
            | "tmp"
            | "temp"
    )
}

fn newest_relevant_file_activity(path: &std::path::Path) -> (Option<u64>, bool) {
    let started = Instant::now();
    let mut scanned_entries = 0usize;
    let mut limited = false;
    let mut newest: Option<u64> = None;
    let mut stack = vec![path.to_path_buf()];

    while let Some(dir) = stack.pop() {
        if scanned_entries >= WORKTREE_ACTIVITY_SCAN_MAX_ENTRIES
            || started.elapsed() >= Duration::from_millis(WORKTREE_ACTIVITY_SCAN_MAX_MS)
        {
            limited = true;
            break;
        }

        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            if scanned_entries >= WORKTREE_ACTIVITY_SCAN_MAX_ENTRIES
                || started.elapsed() >= Duration::from_millis(WORKTREE_ACTIVITY_SCAN_MAX_MS)
            {
                limited = true;
                break;
            }

            scanned_entries += 1;
            let name = entry.file_name().to_string_lossy().to_string();
            if name == ".git" {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };

            if file_type.is_dir() {
                if !is_ignored_worktree_activity_dir(&name) {
                    stack.push(entry.path());
                }
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(system_time_epoch_millis);

            if let Some(timestamp) = modified {
                newest = Some(newest.map_or(timestamp, |current| current.max(timestamp)));
            }
        }
    }

    (newest, limited)
}

fn git_head_timestamp_millis(cwd: &std::path::Path) -> Option<u64> {
    run_git(cwd, &["log", "-1", "--format=%ct", "HEAD"])
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .and_then(|seconds| seconds.checked_mul(1_000))
}

fn git_metadata_timestamp_millis(cwd: &std::path::Path) -> Option<u64> {
    fs::metadata(cwd.join(".git"))
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(system_time_epoch_millis)
}

fn resolve_worktree_activity(path: &std::path::Path) -> (Option<u64>, Option<String>, bool) {
    let (file_activity, limited) = newest_relevant_file_activity(path);
    if let Some(timestamp) = file_activity {
        return (Some(timestamp), Some("file-mtime".to_string()), limited);
    }

    if let Some(timestamp) = git_head_timestamp_millis(path) {
        return (Some(timestamp), Some("git-head".to_string()), limited);
    }

    let git_metadata_activity = git_metadata_timestamp_millis(path);
    (
        git_metadata_activity,
        git_metadata_activity.map(|_| "git-metadata".to_string()),
        limited,
    )
}

fn parse_decimal_i32(value: &str) -> Option<i32> {
    value.parse::<i32>().ok()
}

fn parse_decimal_u32(value: &str) -> Option<u32> {
    value.parse::<u32>().ok()
}

fn days_from_civil(mut year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    year -= i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month_prime = month as i32 + if month > 2 { -3 } else { 9 };
    let doy = (153 * month_prime + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) as i64)
}

fn parse_github_timestamp_millis(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.len() < 20 || !value.ends_with('Z') {
        return None;
    }

    let year = parse_decimal_i32(value.get(0..4)?)?;
    let month = parse_decimal_u32(value.get(5..7)?)?;
    let day = parse_decimal_u32(value.get(8..10)?)?;
    let hour = parse_decimal_u32(value.get(11..13)?)?;
    let minute = parse_decimal_u32(value.get(14..16)?)?;
    let second = parse_decimal_u32(value.get(17..19)?)?;

    if value.get(4..5)? != "-"
        || value.get(7..8)? != "-"
        || value.get(10..11)? != "T"
        || value.get(13..14)? != ":"
        || value.get(16..17)? != ":"
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }

    let days = days_from_civil(year, month, day)?;
    if days < 0 {
        return None;
    }

    let seconds = days as u64 * 86_400 + hour as u64 * 3_600 + minute as u64 * 60 + second as u64;
    seconds.checked_mul(1_000)
}

fn current_epoch_millis() -> Option<u64> {
    system_time_epoch_millis(std::time::SystemTime::now())
}

fn is_recent_github_merged_at(merged_at: &str, now_millis: u64) -> bool {
    let Some(merged_millis) = parse_github_timestamp_millis(merged_at) else {
        return false;
    };
    let cutoff = now_millis.saturating_sub(GITHUB_RECENT_MERGED_WINDOW_MS);
    merged_millis >= cutoff && merged_millis <= now_millis.saturating_add(60_000)
}

fn pull_request_signal_from_github_pr(
    pr: &GitHubPullRequest,
    now_millis: u64,
) -> Option<ProjectWorktreePullRequest> {
    if pr.state == "open" {
        return Some(ProjectWorktreePullRequest {
            number: pr.number,
            title: pr.title.clone(),
            url: pr.url.clone(),
            state: "open".to_string(),
            merged_at: pr.merged_at.clone(),
            updated_at: pr.updated_at.clone(),
        });
    }

    if let Some(merged_at) = &pr.merged_at {
        if is_recent_github_merged_at(merged_at, now_millis) {
            return Some(ProjectWorktreePullRequest {
                number: pr.number,
                title: pr.title.clone(),
                url: pr.url.clone(),
                state: "merged".to_string(),
                merged_at: pr.merged_at.clone(),
                updated_at: pr.updated_at.clone(),
            });
        }
    }

    None
}

fn select_project_worktree_pull_request(
    pull_requests: &[GitHubPullRequest],
    now_millis: u64,
) -> Option<ProjectWorktreePullRequest> {
    pull_requests
        .iter()
        .find_map(|pr| {
            if pr.state == "open" {
                pull_request_signal_from_github_pr(pr, now_millis)
            } else {
                None
            }
        })
        .or_else(|| {
            pull_requests
                .iter()
                .find_map(|pr| pull_request_signal_from_github_pr(pr, now_millis))
        })
}

async fn fetch_github_pull_request_signal(
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<Option<ProjectWorktreePullRequest>, String> {
    let client = github_client()?;
    let url = format!("https://api.github.com/repos/{}/{}/pulls", owner, repo);
    let head = format!("{}:{}", owner, branch);
    let response = with_github_auth(client.get(url))
        .query(&[
            ("state", "all"),
            ("head", head.as_str()),
            ("sort", "updated"),
            ("direction", "desc"),
            ("per_page", "10"),
        ])
        .send()
        .await
        .map_err(|e| format!("GitHub PR request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("GitHub PR request returned status: {}", response.status()));
    }

    let pull_requests: Vec<GitHubPullRequest> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitHub PR response: {}", e))?;
    Ok(select_project_worktree_pull_request(
        &pull_requests,
        current_epoch_millis().unwrap_or(u64::MAX),
    ))
}

async fn cached_github_pull_request_signal(
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<Option<ProjectWorktreePullRequest>, String> {
    let cache_key = format!("{}/{}/{}", owner.to_lowercase(), repo.to_lowercase(), branch);
    if let Some(entry) = GITHUB_PR_SIGNAL_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(&cache_key).cloned())
    {
        if entry.fetched_at.elapsed() <= GITHUB_PR_SIGNAL_CACHE_TTL {
            return Ok(entry.pull_request);
        }
    }

    let pull_request = fetch_github_pull_request_signal(owner, repo, branch).await?;
    if let Ok(mut cache) = GITHUB_PR_SIGNAL_CACHE.lock() {
        cache.insert(
            cache_key,
            GitHubPullRequestCacheEntry {
                fetched_at: Instant::now(),
                pull_request: pull_request.clone(),
            },
        );
    }
    Ok(pull_request)
}

fn derive_action_center_github_review_state(
    is_draft: bool,
    requested_reviewers: u64,
    comments: u64,
    review_comments: u64,
    reviews: &[GitHubPullRequestReview],
) -> String {
    if is_draft {
        return "draft".to_string();
    }

    let mut latest_by_author: HashMap<String, String> = HashMap::new();
    for (index, review) in reviews.iter().enumerate() {
        let state = review.state.to_uppercase();
        if !matches!(state.as_str(), "APPROVED" | "CHANGES_REQUESTED" | "COMMENTED") {
            continue;
        }

        let author = review
            .user
            .as_ref()
            .and_then(|user| user.login.as_deref())
            .filter(|login| !login.is_empty())
            .map(|login| login.to_string())
            .unwrap_or_else(|| format!("unknown-reviewer-{}", index));
        latest_by_author.insert(author, state);
    }

    if latest_by_author.values().any(|state| state == "CHANGES_REQUESTED") {
        return "changes_requested".to_string();
    }

    if latest_by_author.values().any(|state| state == "APPROVED") {
        return "approved".to_string();
    }

    if requested_reviewers > 0 {
        return "review_requested".to_string();
    }

    if latest_by_author.values().any(|state| state == "COMMENTED")
        || comments > 0
        || review_comments > 0
    {
        return "commented".to_string();
    }

    "pending_review".to_string()
}

fn action_center_github_pr_timestamp(pr: &GitHubRepoPullRequest) -> u64 {
    pr.created_at
        .as_deref()
        .and_then(parse_github_timestamp_millis)
        .or_else(|| pr.updated_at.as_deref().and_then(parse_github_timestamp_millis))
        .unwrap_or(JS_MAX_SAFE_INTEGER_MILLIS)
}

fn action_center_github_pr_from_github(
    owner: &str,
    repo: &str,
    pr: &GitHubRepoPullRequest,
    reviews: &[GitHubPullRequestReview],
) -> ActionCenterGitHubPullRequest {
    let requested_reviewers = pr.requested_reviewers.len() as u64;
    let comments = pr.comments.unwrap_or(0);
    let review_comments = pr.review_comments.unwrap_or(0);

    ActionCenterGitHubPullRequest {
        repo_full_name: format!("{}/{}", owner, repo),
        owner: owner.to_string(),
        repo: repo.to_string(),
        number: pr.number,
        title: pr.title.clone(),
        url: pr.url.clone(),
        state: pr.state.clone(),
        branch: pr
            .head
            .as_ref()
            .and_then(|head| head.ref_name.clone())
            .unwrap_or_default(),
        author: pr
            .user
            .as_ref()
            .and_then(|user| user.login.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        is_draft: pr.draft,
        review_state: derive_action_center_github_review_state(
            pr.draft,
            requested_reviewers,
            comments,
            review_comments,
            reviews,
        ),
        comments,
        review_comments,
        requested_reviewers,
        created_at: pr.created_at.clone(),
        updated_at: pr.updated_at.clone(),
        action_timestamp: action_center_github_pr_timestamp(pr),
    }
}

fn is_action_center_github_pr_relevant(pr: &GitHubRepoPullRequest, viewer_login: &str) -> bool {
    let viewer_login = viewer_login.trim();
    if viewer_login.is_empty() {
        return false;
    }

    let is_author = pr
        .user
        .as_ref()
        .and_then(|user| user.login.as_deref())
        .is_some_and(|login| login.eq_ignore_ascii_case(viewer_login));
    if is_author {
        return true;
    }

    pr.requested_reviewers.iter().any(|reviewer| {
        reviewer
            .login
            .as_deref()
            .is_some_and(|login| login.eq_ignore_ascii_case(viewer_login))
    })
}

async fn fetch_action_center_github_viewer_login(
    client: &reqwest::Client,
) -> Result<String, String> {
    let response = with_github_auth(client.get("https://api.github.com/user"))
        .send()
        .await
        .map_err(|e| format!("GitHub user request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("GitHub user request returned status: {}", response.status()));
    }

    let user: GitHubAuthenticatedUser = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitHub user response: {}", e))?;
    if user.login.trim().is_empty() {
        return Err("GitHub user response did not include a login".to_string());
    }
    Ok(user.login)
}

async fn fetch_action_center_github_pull_request_reviews(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<Vec<GitHubPullRequestReview>, String> {
    let url = format!("https://api.github.com/repos/{}/{}/pulls/{}/reviews", owner, repo, number);
    let response = with_github_auth(client.get(url))
        .query(&[("per_page", "100")])
        .send()
        .await
        .map_err(|e| format!("GitHub PR reviews request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("GitHub PR reviews request returned status: {}", response.status()));
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitHub PR reviews response: {}", e))
}

async fn fetch_action_center_github_pull_request_detail(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<GitHubRepoPullRequest, String> {
    let url = format!("https://api.github.com/repos/{}/{}/pulls/{}", owner, repo, number);
    let response = with_github_auth(client.get(url))
        .send()
        .await
        .map_err(|e| format!("GitHub PR detail request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("GitHub PR detail request returned status: {}", response.status()));
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitHub PR detail response: {}", e))
}

async fn fetch_action_center_github_pull_requests(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    viewer_login: &str,
) -> Result<Vec<ActionCenterGitHubPullRequest>, String> {
    let url = format!("https://api.github.com/repos/{}/{}/pulls", owner, repo);
    let response = with_github_auth(client.get(url))
        .query(&[
            ("state", "open"),
            ("sort", "created"),
            ("direction", "asc"),
            ("per_page", "20"),
        ])
        .send()
        .await
        .map_err(|e| format!("GitHub PR request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("GitHub PR request returned status: {}", response.status()));
    }

    let pull_requests: Vec<GitHubRepoPullRequest> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitHub PR response: {}", e))?;

    let mut signals = Vec::with_capacity(pull_requests.len());
    for pr in pull_requests {
        let pr = match fetch_action_center_github_pull_request_detail(
            &client,
            owner,
            repo,
            pr.number,
        )
        .await
        {
            Ok(pr_detail) => pr_detail,
            Err(error) => {
                log_warn!(
                    "[action-center] GitHub PR detail unavailable for {}/{}#{}: {}",
                    owner,
                    repo,
                    pr.number,
                    error
                );
                pr
            }
        };
        if !is_action_center_github_pr_relevant(&pr, viewer_login) {
            continue;
        }

        let reviews = match fetch_action_center_github_pull_request_reviews(
            client,
            owner,
            repo,
            pr.number,
        )
        .await
        {
            Ok(reviews) => reviews,
            Err(error) => {
                log_warn!(
                    "[action-center] GitHub PR review state unavailable for {}/{}#{}: {}",
                    owner,
                    repo,
                    pr.number,
                    error
                );
                Vec::new()
            }
        };
        signals.push(action_center_github_pr_from_github(owner, repo, &pr, &reviews));
    }

    signals.sort_by(|a, b| {
        a.action_timestamp
            .cmp(&b.action_timestamp)
            .then_with(|| a.number.cmp(&b.number))
    });
    Ok(signals)
}

async fn cached_action_center_github_pull_requests(
    owner: &str,
    repo: &str,
) -> Result<(String, Vec<ActionCenterGitHubPullRequest>), String> {
    let client = github_client()?;
    let viewer_login = fetch_action_center_github_viewer_login(&client).await?;
    let repo_full_name = format!("{}/{}", owner, repo);
    let cache_key = format!("{}@{}", repo_full_name.to_lowercase(), viewer_login.to_lowercase());
    if let Some(entry) = GITHUB_ACTION_CENTER_PR_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(&cache_key).cloned())
    {
        if entry.fetched_at.elapsed() <= GITHUB_PR_SIGNAL_CACHE_TTL {
            return Ok((entry.repo_full_name, entry.pull_requests));
        }
    }

    let pull_requests =
        fetch_action_center_github_pull_requests(&client, owner, repo, &viewer_login).await?;
    if let Ok(mut cache) = GITHUB_ACTION_CENTER_PR_CACHE.lock() {
        cache.insert(
            cache_key,
            GitHubActionCenterPullRequestCacheEntry {
                fetched_at: Instant::now(),
                repo_full_name: repo_full_name.clone(),
                pull_requests: pull_requests.clone(),
            },
        );
    }
    Ok((repo_full_name, pull_requests))
}

fn get_linear_api_key() -> Option<String> {
    if let Ok(token) = env::var("LINEAR_API_KEY") {
        if !token.trim().is_empty() {
            return Some(token.trim().to_string());
        }
    }

    let credentials_path = dirs::home_dir()?.join(".linear-credentials");
    let content = fs::read_to_string(credentials_path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim().trim_start_matches("export ").trim();
        let Some(value) = trimmed.strip_prefix("LINEAR_API_KEY=") else {
            continue;
        };
        let token = value.trim().trim_matches('"').trim_matches('\'');
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    None
}

fn json_str(value: &serde_json::Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            return String::new();
        };
        current = next;
    }
    current.as_str().unwrap_or_default().to_string()
}

fn json_optional_str(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let value = json_str(value, path);
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn json_nodes<'a>(value: &'a serde_json::Value, path: &[&str]) -> Vec<&'a serde_json::Value> {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            return Vec::new();
        };
        current = next;
    }
    current
        .as_array()
        .map(|items| items.iter().collect())
        .or_else(|| current.get("nodes").and_then(|nodes| nodes.as_array()).map(|items| items.iter().collect()))
        .unwrap_or_default()
}

fn json_collection_len(value: &serde_json::Value, key: &str) -> usize {
    let Some(collection) = value.get(key) else {
        return 0;
    };
    if let Some(items) = collection.as_array() {
        return items.len();
    }
    collection
        .get("nodes")
        .and_then(|nodes| nodes.as_array())
        .map(|items| items.len())
        .unwrap_or(0)
}

fn collect_github_pull_request_urls(value: &str) -> Vec<String> {
    value
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '(' | ',' | '<' | '>'))
        .filter_map(|part| {
            let trimmed = part.trim_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | ')' | '('));
            if trimmed.contains("github.com/") && trimmed.contains("/pull/") {
                Some(trimmed.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn action_center_linear_timestamp(issue: &serde_json::Value) -> u64 {
    let state_started_at = issue
        .get("stateHistory")
        .and_then(|history| history.get("nodes"))
        .and_then(|nodes| nodes.as_array())
        .and_then(|nodes| nodes.first())
        .and_then(|state| state.get("startedAt"))
        .and_then(|value| value.as_str());

    state_started_at
        .and_then(parse_github_timestamp_millis)
        .or_else(|| json_optional_str(issue, &["updatedAt"]).as_deref().and_then(parse_github_timestamp_millis))
        .unwrap_or(JS_MAX_SAFE_INTEGER_MILLIS)
}

fn action_center_linear_unacked_comments(issue: &serde_json::Value) -> usize {
    json_nodes(issue, &["comments", "nodes"])
        .into_iter()
        .filter(|comment| {
            let is_me = comment
                .get("user")
                .and_then(|user| user.get("isMe"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            !is_me
                && json_collection_len(comment, "reactions") == 0
                && comment
                    .get("children")
                    .and_then(|children| children.get("nodes"))
                    .and_then(|nodes| nodes.as_array())
                    .map(|nodes| nodes.is_empty())
                    .unwrap_or(true)
        })
        .count()
}

fn action_center_linear_issue_from_value(issue: &serde_json::Value) -> Option<ActionCenterLinearIssue> {
    let identifier = json_str(issue, &["identifier"]);
    if identifier.is_empty() {
        return None;
    }

    let status = json_str(issue, &["state", "name"]);
    let labels = json_nodes(issue, &["labels", "nodes"])
        .into_iter()
        .filter_map(|label| json_optional_str(label, &["name"]))
        .collect();

    let mut pull_request_urls = Vec::new();
    for attachment in json_nodes(issue, &["attachments", "nodes"]) {
        if let Some(url) = json_optional_str(attachment, &["url"]) {
            pull_request_urls.extend(collect_github_pull_request_urls(&url));
        }
        if let Some(title) = json_optional_str(attachment, &["title"]) {
            pull_request_urls.extend(collect_github_pull_request_urls(&title));
        }
    }
    pull_request_urls.sort();
    pull_request_urls.dedup();

    Some(ActionCenterLinearIssue {
        identifier,
        title: json_str(issue, &["title"]),
        url: json_str(issue, &["url"]),
        status: status.clone(),
        state_type: json_str(issue, &["state", "type"]),
        is_uat: status.eq_ignore_ascii_case("UAT"),
        assignee: json_optional_str(issue, &["assignee", "displayName"])
            .or_else(|| json_optional_str(issue, &["assignee", "name"])),
        labels,
        pull_request_urls,
        unacked_comments: action_center_linear_unacked_comments(issue),
        updated_at: json_optional_str(issue, &["updatedAt"]),
        action_timestamp: action_center_linear_timestamp(issue),
    })
}

async fn fetch_action_center_linear_issues() -> Result<ActionCenterLinearIssueResponse, String> {
    let Some(api_key) = get_linear_api_key() else {
        return Err("Linear API key not configured".to_string());
    };

    let team_key = env::var("BORABR_LINEAR_TEAM").unwrap_or_else(|_| "ENG".to_string());
    let team_key_json = serde_json::to_string(&team_key).unwrap_or_else(|_| "\"ENG\"".to_string());
    let query = format!(
        r#"
query {{
  viewer {{
    name
    displayName
    assignedIssues(
      filter: {{
        team: {{ key: {{ eq: {} }} }}
        state: {{ type: {{ in: ["started", "unstarted"] }} }}
      }}
      first: 50
    ) {{
      nodes {{
        identifier
        title
        url
        updatedAt
        state {{ name type }}
        stateHistory(first: 1) {{ nodes {{ startedAt }} }}
        priority
        priorityLabel
        assignee {{ name displayName }}
        labels {{ nodes {{ name }} }}
        attachments {{ nodes {{ title url }} }}
        comments {{ nodes {{ user {{ isMe }} reactions {{ id }} children {{ nodes {{ id }} }} createdAt }} }}
      }}
    }}
  }}
}}
"#,
        team_key_json
    );

    let client = reqwest::Client::builder()
        .user_agent("BoraBR")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
    let response = client
        .post("https://api.linear.app/graphql")
        .header("Authorization", api_key)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "query": query }))
        .send()
        .await
        .map_err(|e| format!("Linear API request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Linear API request returned status: {}", response.status()));
    }

    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Linear API response: {}", e))?;
    if let Some(errors) = payload.get("errors").and_then(|errors| errors.as_array()) {
        if !errors.is_empty() {
            return Err(format!("Linear API returned {} error(s)", errors.len()));
        }
    }

    let viewer = payload
        .get("data")
        .and_then(|data| data.get("viewer"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let assignee = json_optional_str(&viewer, &["displayName"])
        .or_else(|| json_optional_str(&viewer, &["name"]));
    let mut issues: Vec<ActionCenterLinearIssue> = json_nodes(&viewer, &["assignedIssues", "nodes"])
        .into_iter()
        .filter_map(action_center_linear_issue_from_value)
        .collect();
    issues.sort_by(|a, b| {
        a.action_timestamp
            .cmp(&b.action_timestamp)
            .then_with(|| a.identifier.cmp(&b.identifier))
    });

    Ok(ActionCenterLinearIssueResponse {
        team_key,
        assignee,
        error: None,
        issues,
    })
}

async fn cached_action_center_linear_issues() -> Result<ActionCenterLinearIssueResponse, String> {
    if let Some(entry) = LINEAR_ACTION_CENTER_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.clone())
    {
        if entry.fetched_at.elapsed() <= LINEAR_ACTION_CENTER_CACHE_TTL {
            return Ok(entry.response);
        }
    }

    let response = fetch_action_center_linear_issues().await?;
    if let Ok(mut cache) = LINEAR_ACTION_CENTER_CACHE.lock() {
        *cache = Some(LinearActionCenterCacheEntry {
            fetched_at: Instant::now(),
            response: response.clone(),
        });
    }
    Ok(response)
}

fn candidate_from_path(
    root_path: &str,
    path: &std::path::Path,
    branch: Option<String>,
    head: Option<String>,
    repo_remote: Option<String>,
    inclusion_reason: &str,
) -> Option<ProjectWorktreeCandidate> {
    let canonical_path = path.canonicalize().ok()?;
    if !canonical_path.exists() {
        return None;
    }
    let canonical_string = canonical_path.to_string_lossy().to_string();

    Some(ProjectWorktreeCandidate {
        root_path: root_path.to_string(),
        worktree_path: path.to_string_lossy().to_string(),
        canonical_path: canonical_string.clone(),
        branch,
        head,
        repo_remote,
        is_root: canonical_string == root_path,
        inclusion_reason: inclusion_reason.to_string(),
        last_activity_at: None,
        last_activity_source: None,
        activity_scan_limited: false,
        recent_activity_rank: None,
        pull_request: None,
        pr_promoted: false,
    })
}

fn scan_github_worktrees_dir(root_path: &str, root_remote: &str, root_remote_key: &str) -> Vec<ProjectWorktreeCandidate> {
    let Some((owner, repo)) = github_owner_repo_from_remote(root_remote) else {
        return Vec::new();
    };
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let scan_root = home.join("worktrees").join("github.com").join(owner).join(repo);
    let Ok(entries) = fs::read_dir(scan_root) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }

            let candidate_root = run_git(&path, &["rev-parse", "--show-toplevel"])
                .ok()
                .map(|value| PathBuf::from(value.trim()))
                .and_then(|value| value.canonicalize().ok())?;

            let candidate_remote = git_remote_url(&candidate_root)?;
            if normalize_git_remote_url(&candidate_remote) != root_remote_key {
                return None;
            }

            candidate_from_path(
                root_path,
                &candidate_root,
                git_short_branch(&candidate_root),
                git_head(&candidate_root),
                Some(candidate_remote),
                "worktrees-directory-scan",
            )
        })
        .collect()
}

fn dedupe_project_worktree_candidates(candidates: Vec<ProjectWorktreeCandidate>) -> Vec<ProjectWorktreeCandidate> {
    let mut seen_paths = HashSet::new();
    let mut seen_repo_branches = HashSet::new();
    let mut deduped = Vec::new();

    for candidate in candidates {
        if !seen_paths.insert(candidate.canonical_path.clone()) {
            continue;
        }

        if let (Some(remote), Some(branch)) = (&candidate.repo_remote, &candidate.branch) {
            let repo_branch = format!("{}::{}", normalize_git_remote_url(remote), branch);
            if !seen_repo_branches.insert(repo_branch) {
                continue;
            }
        }

        deduped.push(candidate);
    }

    deduped.sort_by(|a, b| b.is_root.cmp(&a.is_root).then_with(|| a.canonical_path.cmp(&b.canonical_path)));
    deduped
}

fn apply_worktree_activity(candidates: &mut [ProjectWorktreeCandidate]) {
    for candidate in candidates {
        let path = PathBuf::from(&candidate.canonical_path);
        let (last_activity_at, last_activity_source, activity_scan_limited) =
            resolve_worktree_activity(&path);
        candidate.last_activity_at = last_activity_at;
        candidate.last_activity_source = last_activity_source;
        candidate.activity_scan_limited = activity_scan_limited;
    }
}

async fn apply_github_pull_request_signals(
    candidates: &mut [ProjectWorktreeCandidate],
    root_remote: Option<&str>,
) {
    let Some(remote) = root_remote else {
        return;
    };
    let Some((owner, repo)) = github_owner_repo_from_remote(remote) else {
        return;
    };

    let mut checked_branches = 0usize;
    for candidate in candidates.iter_mut().filter(|candidate| !candidate.is_root) {
        candidate.pull_request = None;
        candidate.pr_promoted = false;

        let Some(branch) = candidate.branch.as_deref() else {
            continue;
        };
        if branch.trim().is_empty() {
            continue;
        }
        if checked_branches >= GITHUB_PR_SIGNAL_MAX_BRANCHES_PER_DISCOVERY {
            break;
        }
        checked_branches += 1;

        match cached_github_pull_request_signal(&owner, &repo, branch).await {
            Ok(Some(pull_request)) => {
                candidate.pull_request = Some(pull_request);
                candidate.pr_promoted = true;
            }
            Ok(None) => {}
            Err(error) => {
                log_warn!("[worktrees] GitHub PR signal unavailable for {}: {}", branch, error);
            }
        }
    }
}

fn assign_recent_activity_ranks(
    candidates: &mut [ProjectWorktreeCandidate],
    limit: usize,
    promoted_canonical_paths: &HashSet<String>,
) {
    for candidate in candidates.iter_mut() {
        candidate.recent_activity_rank = None;
    }

    let mut ranked: Vec<(usize, u64, String)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| !candidate.is_root)
        .filter(|(_, candidate)| !promoted_canonical_paths.contains(&candidate.canonical_path))
        .filter_map(|(index, candidate)| {
            candidate
                .last_activity_at
                .map(|timestamp| (index, timestamp, candidate.canonical_path.clone()))
        })
        .collect();

    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.2.cmp(&b.2)));

    for (rank, (index, _, _)) in ranked.into_iter().take(limit).enumerate() {
        candidates[index].recent_activity_rank = Some(rank + 1);
    }
}

impl From<ProjectWorktreeCandidate> for ProjectWorktree {
    fn from(candidate: ProjectWorktreeCandidate) -> Self {
        Self {
            root_path: candidate.root_path,
            worktree_path: candidate.worktree_path,
            canonical_path: candidate.canonical_path,
            branch: candidate.branch,
            head: candidate.head,
            repo_remote: candidate.repo_remote,
            is_root: candidate.is_root,
            inclusion_reason: candidate.inclusion_reason,
            last_activity_at: candidate.last_activity_at,
            last_activity_source: candidate.last_activity_source,
            activity_scan_limited: candidate.activity_scan_limited,
            recent_activity_rank: candidate.recent_activity_rank,
            pull_request: candidate.pull_request,
            pr_promoted: candidate.pr_promoted,
        }
    }
}

#[tauri::command]
async fn discover_project_worktrees(project_path: String) -> Result<Vec<ProjectWorktree>, String> {
    let project_path = expand_user_path(&project_path)
        .canonicalize()
        .map_err(|e| format!("Cannot resolve project path: {}", e))?;

    let root_output = run_git(&project_path, &["rev-parse", "--show-toplevel"])
        .map_err(|e| format!("Cannot resolve git root: {}", e))?;
    let root_path_buf = PathBuf::from(root_output.trim())
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize git root: {}", e))?;
    let root_path = root_path_buf.to_string_lossy().to_string();
    let root_remote = git_remote_url(&root_path_buf);

    let worktree_output = run_git(&root_path_buf, &["worktree", "list", "--porcelain"])
        .map_err(|e| format!("Cannot list git worktrees: {}", e))?;

    let mut candidates: Vec<ProjectWorktreeCandidate> = parse_git_worktree_porcelain(&worktree_output)
        .into_iter()
        .filter(|worktree| !worktree.prunable)
        .filter_map(|worktree| {
            candidate_from_path(
                &root_path,
                &expand_user_path(&worktree.path),
                worktree.branch,
                worktree.head,
                root_remote.clone(),
                "git-worktree-list",
            )
        })
        .collect();

    if let Some(remote) = &root_remote {
        let root_remote_key = normalize_git_remote_url(remote);
        candidates.extend(scan_github_worktrees_dir(&root_path, remote, &root_remote_key));
    }

    let mut candidates = dedupe_project_worktree_candidates(candidates);
    apply_worktree_activity(&mut candidates);
    apply_github_pull_request_signals(&mut candidates, root_remote.as_deref()).await;
    let promoted_canonical_paths: HashSet<String> = candidates
        .iter()
        .filter(|candidate| candidate.pr_promoted)
        .map(|candidate| candidate.canonical_path.clone())
        .collect();
    assign_recent_activity_ranks(
        &mut candidates,
        RECENT_ACTIVITY_WORKTREE_LIMIT,
        &promoted_canonical_paths,
    );

    Ok(candidates.into_iter().map(ProjectWorktree::from).collect())
}

#[tauri::command]
async fn list_project_github_pull_requests(
    project_path: String,
) -> Result<ActionCenterGitHubPullRequestResponse, String> {
    let project_path_buf = expand_user_path(&project_path)
        .canonicalize()
        .map_err(|e| format!("Cannot resolve project path: {}", e))?;

    let root_output = run_git(&project_path_buf, &["rev-parse", "--show-toplevel"])
        .map_err(|e| format!("Cannot resolve git root: {}", e))?;
    let root_path_buf = PathBuf::from(root_output.trim())
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize git root: {}", e))?;

    let Some(remote) = git_remote_url(&root_path_buf) else {
        return Ok(ActionCenterGitHubPullRequestResponse {
            project_path,
            repo_full_name: None,
            error: None,
            pull_requests: Vec::new(),
        });
    };

    let Some((owner, repo)) = github_owner_repo_from_remote(&remote) else {
        return Ok(ActionCenterGitHubPullRequestResponse {
            project_path,
            repo_full_name: None,
            error: None,
            pull_requests: Vec::new(),
        });
    };

    let repo_full_name = format!("{}/{}", owner, repo);
    match cached_action_center_github_pull_requests(&owner, &repo).await {
        Ok((repo_full_name, pull_requests)) => Ok(ActionCenterGitHubPullRequestResponse {
            project_path,
            repo_full_name: Some(repo_full_name),
            error: None,
            pull_requests,
        }),
        Err(error) => {
            log_warn!(
                "[action-center] GitHub PR list unavailable for {}: {}",
                repo_full_name,
                error
            );
            Ok(ActionCenterGitHubPullRequestResponse {
                project_path,
                repo_full_name: Some(repo_full_name),
                error: Some(error),
                pull_requests: Vec::new(),
            })
        }
    }
}

#[tauri::command]
async fn list_action_center_linear_issues() -> Result<ActionCenterLinearIssueResponse, String> {
    let team_key = env::var("BORABR_LINEAR_TEAM").unwrap_or_else(|_| "ENG".to_string());
    match cached_action_center_linear_issues().await {
        Ok(response) => Ok(response),
        Err(error) => {
            log_warn!("[action-center] Linear issue list unavailable: {}", error);
            Ok(ActionCenterLinearIssueResponse {
                team_key,
                assignee: None,
                error: Some(error),
                issues: Vec::new(),
            })
        }
    }
}

#[tauri::command]
async fn fs_exists(path: String) -> Result<bool, String> {
    Ok(std::path::Path::new(&path).exists())
}

#[tauri::command]
async fn fs_list(path: Option<String>) -> Result<FsListResult, String> {
    use std::fs;

    let target_path = match path {
        Some(p) if p == "~" => dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
        Some(p) => PathBuf::from(p),
        None => dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
    };

    let target_path = target_path.canonicalize()
        .map_err(|e| format!("Cannot resolve path: {}", e))?;

    let entries = fs::read_dir(&target_path)
        .map_err(|e| format!("Cannot read directory: {}", e))?;

    let mut directories: Vec<DirectoryEntry> = Vec::new();

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files
        if name.starts_with('.') {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        if metadata.is_dir() {
            let full_path = entry.path();
            let beads_path = full_path.join(".beads");
            let has_beads = beads_path.is_dir();
            let uses_dolt = has_beads && project_uses_dolt(&beads_path);

            directories.push(DirectoryEntry {
                name,
                path: full_path.to_string_lossy().to_string(),
                is_directory: true,
                has_beads,
                uses_dolt,
            });
        }
    }

    // Sort: beads projects first, then alphabetically
    directories.sort_by(|a, b| {
        match (a.has_beads, b.has_beads) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });

    let current_beads_path = target_path.join(".beads");
    let current_has_beads = current_beads_path.is_dir();
    let current_uses_dolt = current_has_beads && project_uses_dolt(&current_beads_path);

    Ok(FsListResult {
        current_path: target_path.to_string_lossy().to_string(),
        has_beads: current_has_beads,
        uses_dolt: current_uses_dolt,
        entries: directories,
    })
}

// File watcher commands removed - replaced by frontend polling for lower CPU usage

// ============================================================================
// Update Checker
// ============================================================================

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const GITHUB_RELEASES_URL: &str = "https://api.github.com/repos/aryrabelo/BoraBR/releases/latest";

/// Get a GitHub token from `gh auth token` (if gh CLI is installed and authenticated).
/// Raises the API rate limit from 60/hour (anonymous) to 5,000/hour (authenticated).
fn get_github_token() -> Option<String> {
    // Check GITHUB_TOKEN env var first
    if let Ok(token) = env::var("GITHUB_TOKEN") {
        if !token.is_empty() {
            return Some(token);
        }
    }
    // Fall back to gh CLI
    let output = new_command("gh")
        .args(&["auth", "token"])
        .output()
        .ok()?;
    if output.status.success() {
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !token.is_empty() {
            return Some(token);
        }
    }
    None
}

/// Build a reqwest client with GitHub auth if available.
fn github_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("BoraBR")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

/// Add GitHub auth header to a request if a token is available.
fn with_github_auth(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    match get_github_token() {
        Some(token) => req.bearer_auth(token),
        None => req,
    }
}

fn get_platform_string() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

fn find_platform_asset(assets: &[GitHubAsset]) -> Option<&GitHubAsset> {
    let suffix = if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "_macOS-ARM64.dmg"
        } else {
            "_macOS-Intel.dmg"
        }
    } else if cfg!(target_os = "windows") {
        "_Windows.msi"
    } else {
        "_Linux-amd64.AppImage"
    };

    assets.iter().find(|a| a.name.ends_with(suffix))
}

fn compare_versions(current: &str, latest: &str) -> bool {
    // Remove 'v' prefix if present
    let current = current.trim_start_matches('v');
    let latest = latest.trim_start_matches('v');

    let parse_version = |v: &str| -> Vec<u32> {
        v.split('.')
            .filter_map(|s| s.parse::<u32>().ok())
            .collect()
    };

    let current_parts = parse_version(current);
    let latest_parts = parse_version(latest);

    for i in 0..3 {
        let c = current_parts.get(i).copied().unwrap_or(0);
        let l = latest_parts.get(i).copied().unwrap_or(0);
        if l > c {
            return true;
        }
        if c > l {
            return false;
        }
    }
    false
}

#[tauri::command]
async fn check_for_updates() -> Result<UpdateInfo, String> {
    let client = github_client()?;

    let response = with_github_auth(client.get(GITHUB_RELEASES_URL))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch releases: {}", e))?;

    // Handle 404 (no published releases yet)
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(UpdateInfo {
            current_version: CURRENT_VERSION.to_string(),
            latest_version: CURRENT_VERSION.to_string(),
            has_update: false,
            release_url: "https://github.com/aryrabelo/BoraBR/releases".to_string(),
            download_url: None,
            platform: get_platform_string().to_string(),
            release_notes: None,
        });
    }

    if !response.status().is_success() {
        return Err(format!("GitHub API returned status: {}", response.status()));
    }

    let release: GitHubRelease = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse release info: {}", e))?;

    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    let has_update = compare_versions(CURRENT_VERSION, &latest_version);

    let download_url = find_platform_asset(&release.assets)
        .map(|a| a.browser_download_url.clone());

    // Fetch CHANGELOG.md via GitHub API (raw.githubusercontent CDN ignores query params for caching)
    let changelog = with_github_auth(
            client
            .get("https://api.github.com/repos/aryrabelo/BoraBR/contents/CHANGELOG.md")
            .header("Accept", "application/vnd.github.raw+json")
    )
        .send()
        .await
        .ok()
        .and_then(|r| if r.status().is_success() { Some(r) } else { None });
    let changelog_text = match changelog {
        Some(r) => r.text().await.ok(),
        None => None,
    };

    Ok(UpdateInfo {
        current_version: CURRENT_VERSION.to_string(),
        latest_version,
        has_update,
        release_url: release.html_url,
        download_url,
        platform: get_platform_string().to_string(),
        release_notes: changelog_text.or(release.body),
    })
}

#[tauri::command]
async fn check_for_updates_demo() -> Result<UpdateInfo, String> {
    let client = github_client()?;

    let response = with_github_auth(client.get(GITHUB_RELEASES_URL))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch releases: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("GitHub API returned status: {}", response.status()));
    }

    let release: GitHubRelease = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse release info: {}", e))?;

    let latest_version = release.tag_name.trim_start_matches('v').to_string();

    let download_url = find_platform_asset(&release.assets)
        .map(|a| a.browser_download_url.clone());

    // Fetch CHANGELOG.md via GitHub API (raw.githubusercontent CDN ignores query params for caching)
    let changelog = with_github_auth(
            client
            .get("https://api.github.com/repos/aryrabelo/BoraBR/contents/CHANGELOG.md")
            .header("Accept", "application/vnd.github.raw+json")
    )
        .send()
        .await
        .ok()
        .and_then(|r| if r.status().is_success() { Some(r) } else { None });
    let changelog_text = match changelog {
        Some(r) => r.text().await.ok(),
        None => None,
    };

    // Demo mode: force has_update = true, fake current version as 0.0.0
    Ok(UpdateInfo {
        current_version: "0.0.0".to_string(),
        latest_version,
        has_update: true,
        release_url: release.html_url,
        download_url,
        platform: get_platform_string().to_string(),
        release_notes: changelog_text.or(release.body),
    })
}

#[tauri::command]
async fn check_bd_cli_update() -> Result<BdCliUpdateInfo, String> {
    // Get current bd version
    let version_str = get_bd_version().await;
    if version_str.contains("not found") {
        return Err("bd CLI not found".to_string());
    }

    // Parse semver from version string
    let current_tuple = parse_bd_version(&version_str)
        .ok_or_else(|| format!("Could not parse version from: {}", version_str))?;
    let current_version = format!("{}.{}.{}", current_tuple.0, current_tuple.1, current_tuple.2);

    // Determine the correct GitHub repo based on client type (bd vs br)
    let client_type = detect_cli_client(&version_str);
    let api_url = match client_type {
        CliClient::Br => "https://api.github.com/repos/Dicklesworthstone/beads_rust/releases/latest",
        _ => "https://api.github.com/repos/steveyegge/beads/releases/latest",
    };
    let releases_url = match client_type {
        CliClient::Br => "https://github.com/Dicklesworthstone/beads_rust/releases",
        _ => "https://github.com/steveyegge/beads/releases",
    };

    let client = github_client()?;

    let response = with_github_auth(client.get(api_url))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch releases: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("GitHub API returned status: {}", response.status()));
    }

    let release: GitHubRelease = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse release info: {}", e))?;

    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    let has_update = compare_versions(&current_version, &latest_version);

    Ok(BdCliUpdateInfo {
        current_version,
        latest_version,
        has_update,
        release_url: releases_url.to_string(),
    })
}

#[tauri::command]
async fn download_and_install_update(download_url: String) -> Result<String, String> {
    log::info!("[download_update] Starting download from: {}", download_url);

    // Extract filename from URL
    let filename = download_url
        .rsplit('/')
        .next()
        .unwrap_or("update-download")
        .to_string();
    log::info!("[download_update] Target filename: {}", filename);

    // Download the file
    let client = reqwest::Client::builder()
        .user_agent("BoraBR")
        .build()
        .map_err(|e| {
            log::error!("[download_update] Failed to create HTTP client: {}", e);
            format!("Failed to create HTTP client: {}", e)
        })?;

    log::info!("[download_update] Sending GET request...");
    let response = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| {
            log::error!("[download_update] HTTP request failed: {} (url: {})", e, download_url);
            format!("Failed to download update: {}", e)
        })?;

    let status = response.status();
    let final_url = response.url().to_string();
    log::info!("[download_update] Response status: {} (final URL: {})", status, final_url);

    if !status.is_success() {
        log::error!("[download_update] Download failed with status: {} (url: {})", status, final_url);
        return Err(format!("Download failed with status: {}", status));
    }

    log::info!("[download_update] Reading response bytes...");
    let bytes = response
        .bytes()
        .await
        .map_err(|e| {
            log::error!("[download_update] Failed to read response bytes: {}", e);
            format!("Failed to read download bytes: {}", e)
        })?;
    log::info!("[download_update] Downloaded {} bytes", bytes.len());

    // Save to ~/Downloads
    let download_dir = dirs::download_dir()
        .ok_or_else(|| {
            log::error!("[download_update] Could not find Downloads directory");
            "Could not find Downloads directory".to_string()
        })?;

    let dest_path = download_dir.join(&filename);
    log::info!("[download_update] Saving to: {}", dest_path.display());
    fs::write(&dest_path, &bytes)
        .map_err(|e| {
            log::error!("[download_update] Failed to save file to {}: {}", dest_path.display(), e);
            format!("Failed to save file: {}", e)
        })?;

    let dest_str = dest_path.to_string_lossy().to_string();
    log::info!("[download_update] Saved successfully: {} ({} bytes)", dest_str, bytes.len());

    // On macOS, mount the DMG
    #[cfg(target_os = "macos")]
    {
        if filename.ends_with(".dmg") {
            log::info!("[download_update] Mounting DMG: {}", dest_str);
            Command::new("open")
                .arg(&dest_path)
                .spawn()
                .map_err(|e| {
                    log::error!("[download_update] Failed to open DMG: {}", e);
                    format!("Failed to open DMG: {}", e)
                })?;
        }
    }

    Ok(dest_str)
}

// ============================================================================
// Debug / Logging Commands
// ============================================================================

fn get_log_path() -> PathBuf {
    // Match tauri-plugin-log's LogDir resolution per platform:
    //   macOS:   ~/Library/Logs/com.aryrabelo.borabr/
    //   Linux:   ~/.local/share/com.aryrabelo.borabr/logs/  (XDG_DATA_HOME)
    //   Windows: %APPDATA%/com.aryrabelo.borabr/logs/
    #[cfg(target_os = "macos")]
    {
        let home = env::var("HOME").unwrap_or_default();
        PathBuf::from(home)
            .join("Library/Logs/com.aryrabelo.borabr/beads.log")
    }
    #[cfg(target_os = "linux")]
    {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.aryrabelo.borabr")
            .join("logs")
            .join("beads.log")
    }
    #[cfg(target_os = "windows")]
    {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.aryrabelo.borabr")
            .join("logs")
            .join("beads.log")
    }
}

#[tauri::command]
async fn get_logging_enabled() -> bool {
    LOGGING_ENABLED.load(Ordering::Relaxed)
}

#[tauri::command]
async fn set_logging_enabled(enabled: bool) {
    LOGGING_ENABLED.store(enabled, Ordering::Relaxed);
    if enabled {
        log_info!("[debug] Logging enabled");
    }
}

#[tauri::command]
async fn get_verbose_logging() -> bool {
    VERBOSE_LOGGING.load(Ordering::Relaxed)
}

#[tauri::command]
async fn set_verbose_logging(enabled: bool) {
    VERBOSE_LOGGING.store(enabled, Ordering::Relaxed);
    log_info!("[debug] Verbose logging: {}", if enabled { "ON" } else { "OFF" });
}

#[tauri::command]
async fn clear_logs() -> Result<(), String> {
    let log_path = get_log_path();
    if log_path.exists() {
        fs::write(&log_path, "").map_err(|e| format!("Failed to clear logs: {}", e))?;
        log_info!("[debug] Logs cleared");
    }
    Ok(())
}

#[tauri::command]
async fn export_logs() -> Result<String, String> {
    let log_path = get_log_path();
    if !log_path.exists() {
        return Err("No logs to export".to_string());
    }

    // Get export folder: Downloads > Documents > Home
    let export_dir = dirs::download_dir()
        .or_else(dirs::document_dir)
        .or_else(dirs::home_dir)
        .ok_or_else(|| "Could not find a folder to export logs".to_string())?;

    // Generate filename with timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let export_filename = format!("beads-logs-{}.log", now);
    let export_path = export_dir.join(&export_filename);

    // Copy log file
    fs::copy(&log_path, &export_path)
        .map_err(|e| format!("Failed to export logs: {}", e))?;

    Ok(export_path.to_string_lossy().to_string())
}

#[tauri::command]
async fn read_logs(tail_lines: Option<usize>) -> Result<String, String> {
    let log_path = get_log_path();
    if !log_path.exists() {
        return Ok(String::new());
    }

    let content = fs::read_to_string(&log_path)
        .map_err(|e| format!("Failed to read logs: {}", e))?;

    // If tail_lines is specified, return only the last N lines
    if let Some(n) = tail_lines {
        let lines: Vec<&str> = content.lines().collect();
        let start = if lines.len() > n { lines.len() - n } else { 0 };
        Ok(lines[start..].join("\n"))
    } else {
        Ok(content)
    }
}

#[tauri::command]
async fn get_log_path_string() -> String {
    get_log_path().to_string_lossy().to_string()
}

#[tauri::command]
async fn log_frontend(level: String, message: String) {
    match level.as_str() {
        "error" => log::error!("[frontend] {}", message),
        "warn" => log::warn!("[frontend] {}", message),
        _ => log::info!("[frontend] {}", message),
    }
}

#[tauri::command]
async fn get_bd_version() -> String {
    let binary = get_cli_binary();
    match new_command(&binary)
        .arg("--version")
        .env("PATH", get_extended_path())
        .output()
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if binary != "bd" {
                format!("{} ({})", version, binary)
            } else {
                version
            }
        }
        _ => format!("{} not found", binary),
    }
}

#[derive(Debug, Serialize)]
struct CompatibilityInfo {
    version: String,
    /// "bd", "br", or "unknown"
    #[serde(rename = "clientType")]
    client_type: String,
    #[serde(rename = "versionTuple")]
    version_tuple: Option<Vec<u32>>,
    #[serde(rename = "supportsDaemonFlag")]
    supports_daemon_flag: bool,
    #[serde(rename = "usesJsonlFiles")]
    uses_jsonl_files: bool,
    #[serde(rename = "usesDoltBackend")]
    uses_dolt_backend: bool,
    #[serde(rename = "supportsListAllFlag")]
    supports_list_all_flag: bool,
    warnings: Vec<String>,
}

#[tauri::command]
async fn check_bd_compatibility() -> CompatibilityInfo {
    let version_string = get_bd_version().await;
    let info = get_cli_client_info();

    let mut warnings = Vec::new();

    let (client, tuple) = match info {
        Some((client, major, minor, patch)) => (client, Some((major, minor, patch))),
        None => {
            warnings.push(format!("Could not detect CLI client from: {}", version_string));
            (CliClient::Unknown, None)
        }
    };

    let client_type_str = match client {
        CliClient::Bd => "bd",
        CliClient::Br => "br",
        CliClient::Unknown => "unknown",
    };

    if client == CliClient::Br {
        warnings.push("br (beads_rust) detected: frozen on classic SQLite+JSONL architecture, no daemon support".to_string());
    }

    if let Some((major, minor, _)) = tuple {
        if client == CliClient::Bd && major == 0 && minor >= 50 {
            warnings.push("bd >= 0.50.0 detected: daemon and JSONL systems have been removed".to_string());
        }
    }

    CompatibilityInfo {
        version: version_string,
        client_type: client_type_str.to_string(),
        version_tuple: tuple.map(|(a, b, c)| vec![a, b, c]),
        supports_daemon_flag: supports_daemon_flag(),
        uses_jsonl_files: uses_jsonl_files(),
        uses_dolt_backend: uses_dolt_backend(),
        supports_list_all_flag: supports_list_all_flag(),
        warnings,
    }
}

// ============================================================================
// CLI Binary Configuration Commands
// ============================================================================

#[tauri::command]
async fn get_cli_binary_path() -> String {
    get_cli_binary()
}

#[tauri::command]
async fn set_cli_binary_path(path: String) -> Result<String, String> {
    let binary = if path.trim().is_empty() { "bd".to_string() } else { path.trim().to_string() };

    // Validate the binary first
    let version = validate_cli_binary_internal(&binary)?;

    // Update global state and reset version cache (new binary may be different version)
    *CLI_BINARY.lock().unwrap() = binary.clone();
    reset_bd_version_cache();

    // Persist to config file
    let mut config = load_config();
    config.cli_binary = binary.clone();
    save_config(&config)?;

    log_info!("[config] CLI binary set to: {} ({})", binary, version);
    Ok(version)
}

#[tauri::command]
async fn validate_cli_binary(path: String) -> Result<String, String> {
    let binary = if path.trim().is_empty() { "bd".to_string() } else { path.trim().to_string() };
    validate_cli_binary_internal(&binary)
}

fn validate_cli_binary_internal(binary: &str) -> Result<String, String> {
    // Security: reject shell metacharacters — Command::new() doesn't use a shell,
    // but defense-in-depth prevents any future misuse
    let forbidden = [';', '|', '&', '$', '`', '>', '<', '(', ')', '{', '}', '!', '\n', '\r'];
    if binary.chars().any(|c| forbidden.contains(&c)) {
        return Err("Invalid binary path: contains shell metacharacters".to_string());
    }
    if binary.contains("..") {
        return Err("Invalid binary path: directory traversal not allowed".to_string());
    }

    match new_command(binary)
        .arg("--version")
        .env("PATH", get_extended_path())
        .output()
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if version.is_empty() {
                Err(format!("'{}' returned empty version output", binary))
            } else {
                Ok(version)
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(format!("'{}' failed: {}", binary, if stderr.is_empty() { "unknown error".to_string() } else { stderr }))
        }
        Err(e) => {
            Err(format!("'{}' not found or not executable: {}", binary, e))
        }
    }
}

#[tauri::command]
async fn open_image_file(path: String) -> Result<(), String> {
    log_info!("[open_image_file] Opening: {}", path);

    // Security: Only allow image file extensions
    let allowed_extensions = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico", "tiff", "tif"];
    let path_lower = path.to_lowercase();
    let is_image = allowed_extensions.iter().any(|ext| path_lower.ends_with(&format!(".{}", ext)));

    if !is_image {
        return Err("Only image files are allowed".to_string());
    }

    // Verify file exists
    if !std::path::Path::new(&path).exists() {
        return Err(format!("File not found: {}", path));
    }

    // Security: Canonicalize to resolve symlinks/.. and verify inside .beads/attachments/
    let canonical = std::path::Path::new(&path).canonicalize()
        .map_err(|e| format!("Failed to resolve path: {}", e))?;
    let canonical_str = canonical.to_string_lossy();
    if !canonical_str.contains("/.beads/attachments/") {
        log_warn!("[open_image_file] Refusing to open file outside attachments: {} (resolved: {})", path, canonical_str);
        return Err("Can only open files inside .beads/attachments/".to_string());
    }

    // Use platform-specific command to open file with default application
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        new_command("cmd")
            .args(["/C", "start", "", &path])
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
    }

    Ok(())
}

#[derive(Debug, Serialize)]
pub struct ImageData {
    pub base64: String,
    pub mime_type: String,
}

#[tauri::command]
async fn read_image_file(path: String) -> Result<ImageData, String> {
    log_info!("[read_image_file] Reading: {}", path);

    // Security: Only allow image file extensions
    let allowed_extensions: &[(&str, &str)] = &[
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        ("jpeg", "image/jpeg"),
        ("gif", "image/gif"),
        ("webp", "image/webp"),
        ("bmp", "image/bmp"),
        ("svg", "image/svg+xml"),
        ("ico", "image/x-icon"),
        ("tiff", "image/tiff"),
        ("tif", "image/tiff"),
    ];

    let path_lower = path.to_lowercase();
    let mime_type = allowed_extensions
        .iter()
        .find(|(ext, _)| path_lower.ends_with(&format!(".{}", ext)))
        .map(|(_, mime)| *mime);

    let mime_type = match mime_type {
        Some(m) => m.to_string(),
        None => return Err("Only image files are allowed".to_string()),
    };

    // Verify file exists
    if !std::path::Path::new(&path).exists() {
        return Err(format!("File not found: {}", path));
    }

    // Security: Canonicalize to resolve symlinks/.. and verify inside .beads/attachments/
    let canonical = std::path::Path::new(&path).canonicalize()
        .map_err(|e| format!("Failed to resolve path: {}", e))?;
    let canonical_str = canonical.to_string_lossy();
    if !canonical_str.contains("/.beads/attachments/") {
        log_warn!("[read_image_file] Refusing to read file outside attachments: {} (resolved: {})", path, canonical_str);
        return Err("Can only read files inside .beads/attachments/".to_string());
    }

    // Read file and encode as base64
    let data = fs::read(&path).map_err(|e| format!("Failed to read file: {}", e))?;
    let base64 = base64_encode(&data);

    Ok(ImageData { base64, mime_type })
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);

    for chunk in data.chunks(3) {
        let mut buf = [0u8; 3];
        buf[..chunk.len()].copy_from_slice(chunk);

        let n = ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32);

        result.push(ALPHABET[(n >> 18) as usize & 0x3F] as char);
        result.push(ALPHABET[(n >> 12) as usize & 0x3F] as char);

        if chunk.len() > 1 {
            result.push(ALPHABET[(n >> 6) as usize & 0x3F] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(ALPHABET[n as usize & 0x3F] as char);
        } else {
            result.push('=');
        }
    }

    result
}

#[tauri::command]
async fn purge_orphan_attachments(project_path: String) -> Result<PurgeResult, String> {
    log::info!("[purge_orphan_attachments] project: {}", project_path);

    // Calculate absolute project path (reusing pattern from bd_delete)
    let abs_project_path = if project_path == "." || project_path.is_empty() {
        env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?
    } else {
        let p = PathBuf::from(&project_path);
        if p.is_relative() {
            let cwd = env::current_dir()
                .map_err(|e| format!("Failed to get current directory: {}", e))?;
            cwd.join(&p)
        } else {
            p
        }
    };

    let abs_project_path = abs_project_path
        .canonicalize()
        .map_err(|e| format!("Failed to resolve project path: {}", e))?;

    // Build attachments directory path
    let attachments_dir = abs_project_path.join(".beads").join("attachments");

    // If attachments directory doesn't exist, nothing to purge
    if !attachments_dir.exists() || !attachments_dir.is_dir() {
        log::info!("[purge_orphan_attachments] No attachments directory found");
        return Ok(PurgeResult {
            deleted_count: 0,
            deleted_folders: vec![],
        });
    }

    // Get list of all existing issue IDs via bd list --all
    let existing_ids: std::collections::HashSet<String> = {
        let output = execute_bd("list", &["--all".to_string(), "--limit=0".to_string()], Some(&abs_project_path.to_string_lossy()))?;
        let issues = parse_issues_tolerant(&output, "purge_orphan_attachments")?;
        issues.into_iter().map(|i| i.id).collect()
    };

    log::info!("[purge_orphan_attachments] Found {} existing issues", existing_ids.len());

    // List all subdirectories in attachments folder
    let entries = fs::read_dir(&attachments_dir)
        .map_err(|e| format!("Failed to read attachments directory: {}", e))?;

    let mut deleted_folders: Vec<String> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let folder_name = match path.file_name() {
            Some(name) => name.to_string_lossy().to_string(),
            None => continue,
        };

        // Check if this folder corresponds to an existing issue (folders use short IDs)
        let is_owned = existing_ids.iter().any(|id| issue_short_id(id) == folder_name);
        if !is_owned {
            log::info!("[purge_orphan_attachments] Deleting orphan folder: {}", folder_name);
            if let Err(e) = fs::remove_dir_all(&path) {
                log::warn!("[purge_orphan_attachments] Failed to delete {}: {}", folder_name, e);
            } else {
                deleted_folders.push(folder_name);
            }
        }
    }

    let deleted_count = deleted_folders.len();
    log::info!("[purge_orphan_attachments] Purged {} orphan folders", deleted_count);

    Ok(PurgeResult {
        deleted_count,
        deleted_folders,
    })
}

/// Sanitize a filename for safe storage and br JSONL compatibility.
/// Converts to kebab-case, strips diacritics, removes unsafe chars.
/// Example: "Screenshot 2026-02-24 à 10.30.png" → "screenshot-2026-02-24-a-10-30.png"
fn sanitize_filename(filename: &str) -> String {
    // Split into stem and extension
    let (stem, ext) = match filename.rfind('.') {
        Some(pos) => (&filename[..pos], &filename[pos..]),
        None => (filename, ""),
    };

    // Strip diacritics by replacing common accented chars, then lowercase + kebab-case
    let mut sanitized = String::with_capacity(stem.len());
    for c in stem.chars() {
        let replacement = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => "a",
            'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => "e",
            'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => "i",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => "o",
            'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => "u",
            'ñ' | 'Ñ' => "n",
            'ç' | 'Ç' => "c",
            'ß' => "ss",
            'æ' | 'Æ' => "ae",
            'œ' | 'Œ' => "oe",
            'ý' | 'ÿ' | 'Ý' => "y",
            'A'..='Z' => { sanitized.push((c as u8 + 32) as char); continue; },
            'a'..='z' | '0'..='9' => { sanitized.push(c); continue; },
            '-' => { sanitized.push('-'); continue; },
            ' ' | '_' | '.' => { sanitized.push('-'); continue; },
            _ => "-",
        };
        sanitized.push_str(replacement);
    }

    // Collapse multiple consecutive dashes and trim
    let mut result = String::with_capacity(sanitized.len());
    let mut prev_dash = false;
    for c in sanitized.chars() {
        if c == '-' {
            if !prev_dash {
                result.push('-');
            }
            prev_dash = true;
        } else {
            result.push(c);
            prev_dash = false;
        }
    }
    let result = result.trim_matches('-');

    let ext_lower = ext.to_lowercase();
    if result.is_empty() {
        format!("file{}", ext_lower)
    } else {
        format!("{}{}", result, ext_lower)
    }
}

// ============================================================================
// Attachment helpers
// ============================================================================

/// Image file extensions supported for attachment preview
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico", "tiff", "tif"];

/// Markdown file extensions supported for attachment preview
const MARKDOWN_EXTENSIONS: &[&str] = &["md", "markdown"];

/// Extract the short ID from a full issue ID by stripping the project prefix.
/// e.g. "beads-manager-2qk" → "2qk", "kybio-1pxe" → "1pxe",
///      "kybio-front-nuxt-4-466d" → "466d", "beads-manager-02e.1" → "02e.1"
/// Falls back to the full ID if no prefix separator is found.
fn issue_short_id(full_id: &str) -> &str {
    // The short ID is after the last '-' that isn't followed by another segment
    // containing only digits (which would be part of the prefix like "nuxt-4").
    // Strategy: find the last '-' where everything after it matches [a-z0-9.]+ (the short ID).
    // But "kybio-front-nuxt-4-466d": after last '-' is "466d" ✓
    // "kybio-front-nuxt-4": after last '-' is "4" which could be a short ID or prefix part.
    // Since we only call this with real issue IDs (not project names), the last segment is always the short ID.
    match full_id.rfind('-') {
        Some(pos) => &full_id[pos + 1..],
        None => full_id,
    }
}

/// Resolve the attachment directory for an issue.
/// Always uses short ID: .beads/attachments/{short_id}/
fn resolve_attachment_dir(attachments_dir: &std::path::Path, issue_id: &str) -> PathBuf {
    attachments_dir.join(issue_short_id(issue_id))
}

/// Classify a filename as "image", "markdown", or "other"
fn classify_attachment(filename: &str) -> &'static str {
    let lower = filename.to_lowercase();
    if IMAGE_EXTENSIONS.iter().any(|ext| lower.ends_with(&format!(".{}", ext))) {
        "image"
    } else if MARKDOWN_EXTENSIONS.iter().any(|ext| lower.ends_with(&format!(".{}", ext))) {
        "markdown"
    } else {
        "other"
    }
}

/// Resolve a duplicate filename: image.png → image-1.png → image-2.png
fn resolve_duplicate_filename(dir: &std::path::Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(pos) => (&name[..pos], &name[pos..]),
        None => (name, ""),
    };
    for i in 1..1000 {
        let candidate = format!("{}-{}{}", stem, i, ext);
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    // Fallback with timestamp
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}-{}{}", stem, ts, ext)
}

// ============================================================================
// Attachment Refs Migration v3 — filesystem-only
// ============================================================================

/// Check if a ref is a "real" external reference (Redmine, GitHub, or other URL/ID).
/// Returns false for att: refs, local file paths, cleared: sentinels.
fn is_real_external_ref(r: &str) -> bool {
    let trimmed = r.trim();
    if trimmed.is_empty() { return false; }
    if trimmed.starts_with("cleared:") { return false; }
    if trimmed.starts_with("att:") { return false; }
    // Local file paths (absolute or relative .beads/)
    if trimmed.starts_with('/') { return false; }
    if trimmed.starts_with(".beads/") { return false; }
    // Anything with path separators inside .beads or attachments is local
    if trimmed.contains("/attachments/") || trimmed.contains("/.beads/") { return false; }
    true
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RefsMigrationStatus {
    needs_migration: bool,
    ref_count: u32,
    just_migrated: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MigrateRefsResult {
    success: bool,
    refs_updated: u32,
}

/// Check if a project needs attachment refs migration v3.
/// v3 strips all attachment refs (att:, local paths) from external_ref,
/// keeping only real external refs (Redmine, GitHub, URLs).
/// Returns quickly if the .migrated-attachments marker file exists.
#[tauri::command]
async fn check_refs_migration(cwd: Option<String>) -> Result<RefsMigrationStatus, String> {
    let working_dir = cwd
        .or_else(|| env::var("BEADS_PATH").ok())
        .unwrap_or_else(|| {
            env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });

    let beads_dir = PathBuf::from(&working_dir).join(".beads");
    if !beads_dir.exists() {
        return Ok(RefsMigrationStatus { needs_migration: false, ref_count: 0, just_migrated: false });
    }

    // Already migrated to v3?
    if beads_dir.join(".migrated-attachments").exists() {
        // Check if auto-migration just ran (notify signal)
        let notify_path = beads_dir.join(".migrated-attachments-notify");
        let just_migrated = notify_path.exists();
        if just_migrated {
            let _ = std::fs::remove_file(&notify_path);
        }
        return Ok(RefsMigrationStatus { needs_migration: false, ref_count: 0, just_migrated });
    }

    let jsonl_path = beads_dir.join("issues.jsonl");
    if !jsonl_path.exists() {
        let _ = std::fs::write(beads_dir.join(".migrated-attachments"), "");
        return Ok(RefsMigrationStatus { needs_migration: false, ref_count: 0, just_migrated: false });
    }

    // Scan JSONL for refs that need cleanup (non-real external refs)
    let content = std::fs::read_to_string(&jsonl_path)
        .map_err(|e| format!("Failed to read issues.jsonl: {}", e))?;

    let mut ref_count: u32 = 0;

    for line in content.lines() {
        if line.trim().is_empty() { continue; }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(ext_ref) = v.get("external_ref").and_then(|r| r.as_str()) {
            if ext_ref.is_empty() { continue; }
            let refs: Vec<&str> = ext_ref.split(|c: char| c == '\n' || c == '|').collect();
            for r in &refs {
                let trimmed = r.trim();
                if !trimmed.is_empty() && !is_real_external_ref(trimmed) {
                    ref_count += 1;
                    break; // One bad ref per issue is enough to flag it
                }
            }
        }
    }

    // Also check if attachment folders need renaming (full-id → short-id)
    let mut folder_work_count: u32 = 0;
    let attachments_dir = beads_dir.join("attachments");
    if attachments_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&attachments_dir) {
            for entry in entries.flatten() {
                if !entry.path().is_dir() { continue; }
                let name = entry.file_name().to_string_lossy().to_string();
                if issue_short_id(&name) != name {
                    folder_work_count += 1;
                }
            }
        }
    }

    let total = ref_count + folder_work_count;
    if total == 0 {
        let _ = std::fs::write(beads_dir.join(".migrated-attachments"), "");
        return Ok(RefsMigrationStatus { needs_migration: false, ref_count: 0, just_migrated: false });
    }

    log_info!("[refs_migration_v3] Project needs migration: {} ref(s) to clean, {} folder(s) to update", ref_count, folder_work_count);
    Ok(RefsMigrationStatus { needs_migration: true, ref_count: total, just_migrated: false })
}

/// Perform the attachment refs migration v3 (filesystem-only).
/// Delegates to ensure_refs_migrated_v3 which handles backup, cleanup, dedup, and marker.
/// The br sync is NOT called here — it will happen naturally after via sync_bd_database.
#[tauri::command]
async fn migrate_attachment_refs(cwd: Option<String>) -> Result<MigrateRefsResult, String> {
    let working_dir = cwd
        .or_else(|| env::var("BEADS_PATH").ok())
        .unwrap_or_else(|| {
            env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });

    let beads_dir = PathBuf::from(&working_dir).join(".beads");
    ensure_refs_migrated_v3(&beads_dir, &working_dir);
    Ok(MigrateRefsResult { success: true, refs_updated: 0 })
}

#[tauri::command]
async fn copy_file_to_attachments(
    project_path: String,
    source_path: String,
    issue_id: String,
) -> Result<String, String> {
    log::info!(
        "[copy_file_to_attachments] project: {}, source: {}, issue: {}",
        project_path,
        source_path,
        issue_id
    );

    // Validate file extension (images + markdown)
    let source_lower = source_path.to_lowercase();
    let is_allowed = IMAGE_EXTENSIONS.iter().chain(MARKDOWN_EXTENSIONS.iter())
        .any(|ext| source_lower.ends_with(&format!(".{}", ext)));

    if !is_allowed {
        return Err("Only image and markdown files are allowed".to_string());
    }

    // Verify source file exists
    let source = PathBuf::from(&source_path);
    if !source.exists() {
        return Err(format!("Source file not found: {}", source_path));
    }

    // Calculate absolute project path
    let abs_project_path = if project_path == "." || project_path.is_empty() {
        env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?
    } else {
        let p = PathBuf::from(&project_path);
        if p.is_relative() {
            let cwd = env::current_dir()
                .map_err(|e| format!("Failed to get current directory: {}", e))?;
            cwd.join(&p)
        } else {
            p
        }
    };

    let abs_project_path = abs_project_path
        .canonicalize()
        .map_err(|e| format!("Failed to resolve project path: {}", e))?;

    // Build destination directory: {project}/.beads/attachments/{short_id}/
    let attachments_dir = abs_project_path.join(".beads").join("attachments");
    let dest_dir = resolve_attachment_dir(&attachments_dir, &issue_id);

    // Create directory if needed
    fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("Failed to create attachments directory: {}", e))?;

    // Sanitize the original filename and handle duplicates
    let raw_filename = source
        .file_name()
        .ok_or_else(|| "Invalid source filename".to_string())?
        .to_string_lossy()
        .to_string();
    let sanitized = sanitize_filename(&raw_filename);
    let dest_filename = resolve_duplicate_filename(&dest_dir, &sanitized);
    let dest_path = dest_dir.join(&dest_filename);

    // Copy the file
    fs::copy(&source, &dest_path).map_err(|e| format!("Failed to copy file: {}", e))?;

    log::info!("[copy_file_to_attachments] Copied to: {}", dest_path.display());

    // Return just the filename (frontend doesn't need to store it in external_ref)
    Ok(dest_filename)
}

// ============================================================================
// Filesystem-based Attachment Commands
// ============================================================================

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentFile {
    pub filename: String,
    pub file_type: String,   // "image" or "markdown"
    pub path: String,        // absolute path
    pub modified: u64,       // mtime in epoch seconds (for sorting)
}

/// List all attachments for an issue by reading the filesystem directly.
/// Returns images and markdown files sorted by modification time (newest first).
#[tauri::command]
async fn list_attachments(project_path: String, issue_id: String) -> Result<Vec<AttachmentFile>, String> {
    let abs_project_path = if project_path == "." || project_path.is_empty() {
        env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?
    } else {
        let p = PathBuf::from(&project_path);
        if p.is_relative() {
            let cwd = env::current_dir()
                .map_err(|e| format!("Failed to get current directory: {}", e))?;
            cwd.join(&p)
        } else {
            p
        }
    };

    let attachments_dir = abs_project_path.join(".beads").join("attachments");
    let issue_dir = resolve_attachment_dir(&attachments_dir, &issue_id);

    if !issue_dir.exists() || !issue_dir.is_dir() {
        return Ok(vec![]);
    }

    let mut files: Vec<AttachmentFile> = Vec::new();

    let entries = fs::read_dir(&issue_dir)
        .map_err(|e| format!("Failed to read attachment directory: {}", e))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }

        let name = entry.file_name().to_string_lossy().to_string();
        // Skip legacy index.json files
        if name == "index.json" { continue; }

        let file_type = classify_attachment(&name);
        // Only return images and markdown
        if file_type == "other" { continue; }

        let modified = entry.metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        files.push(AttachmentFile {
            filename: name,
            file_type: file_type.to_string(),
            path: path.to_string_lossy().to_string(),
            modified,
        });
    }

    // Sort by mtime descending (newest first)
    files.sort_by(|a, b| b.modified.cmp(&a.modified));

    Ok(files)
}

/// Delete an attachment file by filename within an issue's attachment directory.
#[tauri::command]
async fn delete_attachment(project_path: String, issue_id: String, filename: String) -> Result<(), String> {
    log::info!("[delete_attachment] project: {}, issue: {}, file: {}", project_path, issue_id, filename);

    // Security: reject path traversal
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err("Invalid filename".to_string());
    }

    let abs_project_path = if project_path == "." || project_path.is_empty() {
        env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?
    } else {
        let p = PathBuf::from(&project_path);
        if p.is_relative() {
            let cwd = env::current_dir()
                .map_err(|e| format!("Failed to get current directory: {}", e))?;
            cwd.join(&p)
        } else {
            p
        }
    };

    let abs_project_path = abs_project_path
        .canonicalize()
        .map_err(|e| format!("Failed to resolve project path: {}", e))?;

    let attachments_dir = abs_project_path.join(".beads").join("attachments");
    let issue_dir = resolve_attachment_dir(&attachments_dir, &issue_id);
    let file_path = issue_dir.join(&filename);

    if !file_path.exists() {
        log::info!("[delete_attachment] File does not exist: {:?}", file_path);
        return Ok(());
    }

    // Security: verify file is inside .beads/attachments/
    let canonical = file_path.canonicalize()
        .map_err(|e| format!("Failed to resolve path: {}", e))?;
    let canonical_str = canonical.to_string_lossy();
    if !canonical_str.contains("/.beads/attachments/") {
        return Err("Can only delete files inside .beads/attachments/".to_string());
    }

    fs::remove_file(&file_path)
        .map_err(|e| format!("Failed to delete file: {}", e))?;

    log::info!("[delete_attachment] Deleted: {:?}", file_path);

    // Cleanup empty folder (issue_dir already resolved above via resolve_attachment_dir)
    if issue_dir.exists() {
        if let Ok(entries) = fs::read_dir(&issue_dir) {
            // Count non-index.json entries
            let count = entries.flatten()
                .filter(|e| e.file_name().to_string_lossy() != "index.json")
                .count();
            if count == 0 {
                // Remove index.json if present, then the directory
                let _ = fs::remove_file(issue_dir.join("index.json"));
                let _ = fs::remove_dir(&issue_dir);
                log::info!("[delete_attachment] Cleaned up empty folder: {:?}", issue_dir);
            }
        }
    }

    Ok(())
}

#[derive(Debug, Serialize)]
pub struct TextData {
    pub content: String,
}

#[tauri::command]
async fn read_text_file(path: String) -> Result<TextData, String> {
    log_info!("[read_text_file] Reading: {}", path);

    // Security: Only allow markdown file extensions
    let path_lower = path.to_lowercase();
    let is_markdown = path_lower.ends_with(".md") || path_lower.ends_with(".markdown");

    if !is_markdown {
        return Err("Only markdown files are allowed".to_string());
    }

    // Verify file exists
    if !std::path::Path::new(&path).exists() {
        return Err(format!("File not found: {}", path));
    }

    // Security: Canonicalize to resolve symlinks/.. and verify inside .beads/attachments/
    let canonical = std::path::Path::new(&path).canonicalize()
        .map_err(|e| format!("Failed to resolve path: {}", e))?;
    let canonical_str = canonical.to_string_lossy();
    if !canonical_str.contains("/.beads/attachments/") {
        log_warn!("[read_text_file] Refusing to read file outside attachments: {} (resolved: {})", path, canonical_str);
        return Err("Can only read files inside .beads/attachments/".to_string());
    }

    // Read file as UTF-8
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read file: {}", e))?;

    Ok(TextData { content })
}

#[tauri::command]
async fn write_text_file(path: String, content: String) -> Result<(), String> {
    log_info!("[write_text_file] Writing: {}", path);

    // Security: Only allow markdown file extensions
    let path_lower = path.to_lowercase();
    let is_markdown = path_lower.ends_with(".md") || path_lower.ends_with(".markdown");

    if !is_markdown {
        return Err("Only markdown files are allowed".to_string());
    }

    // Verify file exists (no creation of new files)
    if !std::path::Path::new(&path).exists() {
        return Err(format!("File not found: {}", path));
    }

    // Security: Canonicalize to resolve symlinks/.. and verify inside .beads/attachments/
    let canonical = std::path::Path::new(&path).canonicalize()
        .map_err(|e| format!("Failed to resolve path: {}", e))?;
    let canonical_str = canonical.to_string_lossy();
    if !canonical_str.contains("/.beads/attachments/") {
        log_warn!("[write_text_file] Refusing to write file outside attachments: {} (resolved: {})", path, canonical_str);
        return Err("Can only write files inside .beads/attachments/".to_string());
    }

    // Write content to file
    fs::write(&path, &content)
        .map_err(|e| format!("Failed to write file: {}", e))?;

    log_info!("[write_text_file] Written {} bytes to {}", content.len(), path);
    Ok(())
}

// ============================================================================
// File Watcher Commands
// ============================================================================

#[tauri::command]
fn start_watching(
    path: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, Mutex<WatcherState>>,
) -> Result<(), String> {
    let mut watcher_state = state.lock().map_err(|e| format!("Lock error: {}", e))?;

    // Stop existing watcher if any
    if watcher_state.debouncer.is_some() {
        log::info!("[watcher] Stopping previous watcher for: {:?}", watcher_state.watched_path);
        watcher_state.debouncer = None;
        watcher_state.watched_path = None;
    }

    let beads_dir = PathBuf::from(&path).join(".beads");
    if !beads_dir.exists() {
        return Err(format!(".beads directory not found at: {}", beads_dir.display()));
    }

    let project_path = path.clone();
    let app_handle = app.clone();

    let mut debouncer = new_debouncer(
        Duration::from_millis(1000),
        move |res: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
            match res {
                Ok(events) => {
                    // Filter: only emit if we have actual data-change events
                    let has_data_events = events.iter().any(|e| {
                        matches!(e.kind, DebouncedEventKind::Any | DebouncedEventKind::AnyContinuous)
                    });
                    if has_data_events {
                        log::info!("[watcher] Change detected in .beads/ ({} events)", events.len());
                        let _ = app_handle.emit(
                            "beads-changed",
                            BeadsChangedPayload { path: project_path.clone() },
                        );
                    }
                }
                Err(e) => {
                    log::error!("[watcher] Error: {:?}", e);
                }
            }
        },
    ).map_err(|e| format!("Failed to create watcher: {}", e))?;

    // Watch .beads/ directory
    // Dolt backend: recursive (changes happen in .dolt/ subdirectories)
    // SQLite backend: non-recursive (all target files are at root level)
    let watch_mode = if project_uses_dolt(&beads_dir) {
        notify::RecursiveMode::Recursive
    } else {
        notify::RecursiveMode::NonRecursive
    };
    debouncer.watcher().watch(
        beads_dir.as_path(),
        watch_mode,
    ).map_err(|e| format!("Failed to watch .beads/: {}", e))?;

    log::info!("[watcher] Started watching: {}", beads_dir.display());
    watcher_state.debouncer = Some(debouncer);
    watcher_state.watched_path = Some(path);

    Ok(())
}

#[tauri::command]
fn stop_watching(
    state: tauri::State<'_, Mutex<WatcherState>>,
) -> Result<(), String> {
    let mut watcher_state = state.lock().map_err(|e| format!("Lock error: {}", e))?;

    if watcher_state.debouncer.is_some() {
        log::info!("[watcher] Stopped watching: {:?}", watcher_state.watched_path);
        watcher_state.debouncer = None;
        watcher_state.watched_path = None;
    }

    Ok(())
}

#[derive(Debug, Serialize)]
struct WatcherStatusInfo {
    active: bool,
    #[serde(rename = "watchedPath")]
    watched_path: Option<String>,
}

#[tauri::command]
fn get_watcher_status(
    state: tauri::State<'_, Mutex<WatcherState>>,
) -> Result<WatcherStatusInfo, String> {
    let watcher_state = state.lock().map_err(|e| format!("Lock error: {}", e))?;

    Ok(WatcherStatusInfo {
        active: watcher_state.debouncer.is_some(),
        watched_path: watcher_state.watched_path.clone(),
    })
}

// ============================================================================
// External Data Source Commands
// ============================================================================

#[tauri::command]
async fn fetch_external_data(url: String) -> Result<String, String> {
    log_info!("[probe] GET {}", url);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        let err = format!("HTTP {}: {}", response.status().as_u16(), response.status().canonical_reason().unwrap_or("Unknown"));
        log_error!("[probe] GET failed: {}", err);
        return Err(err);
    }

    response.text().await.map_err(|e| format!("Failed to read response: {}", e))
}

#[tauri::command]
async fn check_external_health(url: String) -> Result<bool, String> {
    let health_url = format!("{}/health", url.trim_end_matches('/'));
    log_info!("[probe] Health check: {}", health_url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    match client.get(&health_url).send().await {
        Ok(response) => Ok(response.status().is_success()),
        Err(_) => Ok(false),
    }
}

#[tauri::command]
async fn post_external_data(url: String, body: String) -> Result<String, String> {
    log_info!("[probe] POST {}", url);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("HTTP {}: {}", status.as_u16(), text));
    }

    response.text().await.map_err(|e| format!("Failed to read response: {}", e))
}

#[tauri::command]
async fn delete_external_data(url: String) -> Result<String, String> {
    log_info!("[probe] DELETE {}", url);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let response = client
        .delete(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("HTTP {}: {}", status.as_u16(), text));
    }

    response.text().await.map_err(|e| format!("Failed to read response: {}", e))
}

#[tauri::command]
async fn patch_external_data(url: String, body: String) -> Result<String, String> {
    log_info!("[probe] PATCH {}", url);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let response = client
        .patch(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("HTTP {}: {}", status.as_u16(), text));
    }

    response.text().await.map_err(|e| format!("Failed to read response: {}", e))
}

// ============================================================================
// Probe Launcher
// ============================================================================

#[tauri::command]
async fn launch_probe(port: u16) -> Result<String, String> {
    use std::process::Stdio;

    let health_url = format!("http://127.0.0.1:{}/health", port);

    // Check if probe is already reachable via HTTP health endpoint
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    if let Ok(resp) = client.get(&health_url).send().await {
        if resp.status().is_success() {
            log_info!("[probe] Already running on port {}", port);
            return Ok("already running".to_string());
        }
    }

    // Determine binary: BEADS_PROBE_BIN env var, fallback to "beads-probe"
    let bin = env::var("BEADS_PROBE_BIN").unwrap_or_else(|_| "beads-probe".to_string());
    log_info!("[probe] Launching: {} --port {}", bin, port);

    let child = Command::new(&bin)
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn {}: {}", bin, e))?;

    // Store child handle so it lives as long as the app
    if let Ok(mut guard) = PROBE_CHILD.lock() {
        *guard = Some(child);
    }

    log_info!("[probe] Launched on port {}", port);
    Ok("launched".to_string())
}

// ============================================================================
// App Entry Point
// ============================================================================

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Load .env file (dev only — in prod there's no .env, env vars come from the system)
    let _ = dotenvy::dotenv();

    tauri::Builder::default()
        .manage(Mutex::new(WatcherState::default()))
        .manage(terminal::TerminalManager::default())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Enable logging in both debug and release builds
            let log_level = if cfg!(debug_assertions) {
                log::LevelFilter::Debug
            } else {
                log::LevelFilter::Info
            };
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(log_level)
                    .max_file_size(5_000_000) // 5 MB max per log file
                    .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne) // Keep only one backup
                    .target(tauri_plugin_log::Target::new(
                        tauri_plugin_log::TargetKind::LogDir { file_name: Some("beads.log".into()) },
                    ))
                    .target(tauri_plugin_log::Target::new(
                        tauri_plugin_log::TargetKind::Stdout,
                    ))
                    .build(),
            )?;

            // Log startup info
            log::info!("=== BoraBR starting ===");
            log::info!("[startup] Extended PATH: {}", get_extended_path());

            // Load config and set CLI binary (auto-detects br→bd if no config exists)
            let config = load_config();
            log::info!("[startup] CLI binary: {}", config.cli_binary);
            *CLI_BINARY.lock().unwrap() = config.cli_binary.clone();

            // Check if CLI binary is accessible
            // IMPORTANT: Run from /tmp to avoid bd auto-migrating projects in cwd
            let binary = get_cli_binary();
            match new_command(&binary)
                .arg("--version")
                .current_dir(std::env::temp_dir())
                .env("PATH", get_extended_path())
                .output()
            {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout);
                    log::info!("[startup] {} found: {}", binary, version.trim());
                }
                Ok(output) => {
                    log::warn!("[startup] {} command failed: {}", binary, String::from_utf8_lossy(&output.stderr));
                }
                Err(e) => {
                    log::error!("[startup] {} not found or not executable: {}", binary, e);
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bd_sync,
            bd_repair_database,
            bd_migrate_to_dolt,
            bd_check_needs_migration,
            bd_cleanup_stale_locks,
            bd_check_changed,
            bd_reset_mtime,
            bd_poll_data,
            bd_list,
            bd_count,
            bd_ready,
            bd_status,
            bd_show,
            bd_create,
            get_logging_enabled,
            set_logging_enabled,
            get_verbose_logging,
            set_verbose_logging,
            clear_logs,
            export_logs,
            read_logs,
            get_log_path_string,
            log_frontend,
            get_bd_version,
            check_bd_compatibility,
            get_cli_binary_path,
            set_cli_binary_path,
            validate_cli_binary,
            bd_update,
            bd_close,
            bd_search,
            bd_label_add,
            bd_label_remove,
            bd_delete,
            bd_comments_add,
            bd_dep_add,
            bd_dep_remove,
            bd_dep_add_relation,
            bd_dep_remove_relation,
            bd_available_relation_types,
            discover_project_worktrees,
            list_project_github_pull_requests,
            list_action_center_linear_issues,
            fs_exists,
            fs_list,
            check_for_updates,
            check_for_updates_demo,
            check_bd_cli_update,
            download_and_install_update,
            open_image_file,
            read_image_file,
            copy_file_to_attachments,
            list_attachments,
            delete_attachment,
            read_text_file,
            write_text_file,
            purge_orphan_attachments,
            check_refs_migration,
            migrate_attachment_refs,
            start_watching,
            stop_watching,
            get_watcher_status,
            fetch_external_data,
            check_external_health,
            post_external_data,
            delete_external_data,
            patch_external_data,
            launch_probe,
            agent_process_status,
            cmux_focus_surface,
            cmux_send_prompt,
            caffeinate_start,
            caffeinate_stop,
            auto_mode_dispatch,
            auto_mode_dispatch_review,
            auto_mode_merge_approved,
            auto_mode_log_append,
            auto_mode_log_read,
            auto_mode_log_clear,
            auto_mode_cancel_task,
            terminal_native_renderer_capabilities,
            terminal_open_native_renderer,
            terminal::terminal_create,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_restart,
            terminal::terminal_close,
            terminal::terminal_list,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_issue_json(id: &str, title: &str) -> String {
        format!(
            r#"{{"id":"{}","title":"{}","description":null,"status":"open","priority":3,"issue_type":"task","owner":null,"assignee":null,"labels":[],"created_at":"2025-01-01T00:00:00Z","created_by":null,"updated_at":"2025-01-01T00:00:00Z","closed_at":null,"close_reason":null,"blocked_by":null,"blocks":null,"comments":null,"external_ref":null,"estimate":null,"design":null,"acceptance_criteria":null,"notes":null,"parent":null,"dependents":null,"dependencies":null,"dependency_count":null,"dependent_count":null,"metadata":null,"spec_id":null,"comment_count":null}}"#,
            id, title
        )
    }

    #[test]
    fn parse_flat_array() {
        let json = format!("[{}]", minimal_issue_json("abc-123", "Bug fix"));
        let result = parse_issues_tolerant(&json, "test_flat");
        assert!(result.is_ok());
        let issues = result.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, "abc-123");
        assert_eq!(issues[0].title, "Bug fix");
    }

    #[test]
    fn parse_paginated_envelope() {
        let json = format!(
            r#"{{"issues":[{},{}],"total":2,"offset":0,"limit":50,"has_more":false}}"#,
            minimal_issue_json("abc-123", "First"),
            minimal_issue_json("def-456", "Second")
        );
        let result = parse_issues_tolerant(&json, "test_envelope");
        assert!(result.is_ok());
        let issues = result.unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].id, "abc-123");
        assert_eq!(issues[1].id, "def-456");
    }

    #[test]
    fn normalize_issue_status_accepts_in_review() {
        assert_eq!(normalize_issue_status("in_review"), "in_review");
    }

    #[test]
    fn agent_process_status_is_unknown_without_pid() {
        let request = AgentProcessStatusRequest {
            tool: Some("codex".to_string()),
            pid: None,
            session_id: Some("sess-1".to_string()),
        };

        let response = classify_agent_process_status(request, |_| None);

        assert_eq!(response.status, "unknown");
        assert_eq!(response.tool.as_deref(), Some("codex"));
        assert_eq!(response.session_id.as_deref(), Some("sess-1"));
    }

    #[test]
    fn agent_process_status_uses_best_effort_probe() {
        let request = AgentProcessStatusRequest {
            tool: Some("claude".to_string()),
            pid: Some(4242),
            session_id: None,
        };

        let response = classify_agent_process_status(request, |pid| Some(pid == 4242));

        assert_eq!(response.status, "running");
        assert_eq!(response.pid, Some(4242));
    }

    #[test]
    fn cmux_surface_validation_accepts_uuid_and_refs() {
        assert!(validate_cmux_surface_id("7DCCBE94-C09F-4E40-80D6-23FAEFD7D116").is_ok());
        assert!(validate_cmux_surface_id("surface:139").is_ok());
        assert!(validate_cmux_surface_id("pane_1").is_ok());
    }

    #[test]
    fn cmux_surface_validation_rejects_shell_like_values() {
        assert!(validate_cmux_surface_id("").is_err());
        assert!(validate_cmux_surface_id("surface:1;rm").is_err());
        assert!(validate_cmux_surface_id("surface 1").is_err());
    }

    #[test]
    fn cmux_focus_uses_rpc_and_legacy_fallback() {
        let surface = "7DCCBE94-C09F-4E40-80D6-23FAEFD7D116";

        assert_eq!(
            cmux_focus_surface_command(surface),
            vec!["focus-surface", "--surface", surface],
        );
        assert_eq!(
            cmux_focus_surface_rpc_command(surface),
            vec!["rpc", "surface.focus", "{\"surface_id\":\"7DCCBE94-C09F-4E40-80D6-23FAEFD7D116\"}"],
        );
        assert_eq!(
            cmux_identify_surface_command(surface),
            vec!["identify", "--surface", surface],
        );
        assert_eq!(
            cmux_select_workspace_command("workspace:25"),
            vec!["select-workspace", "--workspace", "workspace:25"],
        );
        assert_eq!(
            cmux_focus_surface_fallback_command(surface),
            vec!["move-surface", "--surface", surface, "--focus", "true"],
        );
        assert_eq!(
            cmux_send_prompt_command(surface, "Continuar a tarefa aawk usando a skill BR"),
            vec!["send", "--surface", surface, "Continuar a tarefa aawk usando a skill BR\\n"],
        );
        assert!(should_fallback_cmux_focus("Error: Unknown command: focus-surface"));
        assert!(!should_fallback_cmux_focus("permission denied"));
    }

    #[test]
    fn parse_workspace_ref_from_cmux_identify_output_handles_text_and_json() {
        let json = r#"{"surface_id":"surface:139","workspace_ref":"workspace:25"}"#;
        let plain = "surface_ref: surface:139\nworkspace_ref: workspace:25\n";
        let quoted = "workspace_ref='workspace:25'";

        assert_eq!(
            parse_workspace_ref_from_cmux_identify_output(json),
            Some("workspace:25".to_string())
        );
        assert_eq!(
            parse_workspace_ref_from_cmux_identify_output(plain),
            Some("workspace:25".to_string())
        );
        assert_eq!(
            parse_workspace_ref_from_cmux_identify_output(quoted),
            Some("workspace:25".to_string())
        );
        assert_eq!(parse_workspace_ref_from_cmux_identify_output("no workspace"), None);
    }

    #[test]
    fn native_ghostty_launch_plan_uses_safe_shell_context() {
        let plan = build_native_terminal_launch_plan(
            "macos",
            Some("/Applications/Ghostty.app".into()),
            "/tmp/project with space",
            Some("borabr-m0z.13"),
            "/bin/zsh",
            "native-term-1",
        )
        .unwrap();

        assert_eq!(plan.program, "open");
        assert_eq!(
            plan.args,
            vec![
                "-n",
                "/Applications/Ghostty.app",
                "--args",
                "-e",
                "/bin/zsh",
                "-lc",
                "cd '/tmp/project with space' || exit\nexport BEADS_PATH='/tmp/project with space'\nexport BORABR_TERMINAL_SESSION_ID='native-term-1'\nexport BORABR_ISSUE_ID='borabr-m0z.13'\nexec '/bin/zsh' -l",
            ]
        );
    }

    #[test]
    fn native_ghostty_launch_plan_escapes_shell_values() {
        let plan = build_native_terminal_launch_plan(
            "linux",
            None,
            "/tmp/project's dir",
            Some("issue'one"),
            "/bin/zsh",
            "native-term-2",
        )
        .unwrap();

        assert_eq!(plan.program, "ghostty");
        assert_eq!(plan.args[0], "-e");
        assert!(
            plan.args[3].contains("cd '/tmp/project'\"'\"'s dir' || exit"),
            "expected shell-escaped cwd, got {:?}",
            plan.args[3]
        );
        assert!(
            plan.args[3].contains("export BORABR_ISSUE_ID='issue'\"'\"'one'"),
            "expected shell-escaped issue id, got {:?}",
            plan.args[3]
        );
    }

    #[test]
    fn parse_git_worktree_porcelain_keeps_branch_head_and_prunable_state() {
        let output = "\
worktree /repos/app
HEAD aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
branch refs/heads/main

worktree /worktrees/app-feature
HEAD bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
branch refs/heads/feature/sidebar

worktree /worktrees/app-old
HEAD cccccccccccccccccccccccccccccccccccccccc
prunable gitdir file points to non-existent location
";

        let parsed = parse_git_worktree_porcelain(output);

        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].path, "/repos/app");
        assert_eq!(parsed[0].branch.as_deref(), Some("main"));
        assert_eq!(parsed[0].head.as_deref(), Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
        assert!(!parsed[0].prunable);
        assert_eq!(parsed[1].branch.as_deref(), Some("feature/sidebar"));
        assert!(parsed[2].prunable);
    }

    #[test]
    fn github_remote_parser_handles_common_remote_forms() {
        assert_eq!(
            github_owner_repo_from_remote("git@github.com:entrc/entrc-backend.git"),
            Some(("entrc".to_string(), "entrc-backend".to_string())),
        );
        assert_eq!(
            github_owner_repo_from_remote("https://github.com/entrc/entrc-backend"),
            Some(("entrc".to_string(), "entrc-backend".to_string())),
        );
        assert_eq!(
            github_owner_repo_from_remote("ssh://git@github.com/entrc/entrc-backend.git"),
            Some(("entrc".to_string(), "entrc-backend".to_string())),
        );
    }

    #[test]
    fn github_pull_request_signal_prefers_open_prs() {
        fn pr(number: u64, state: &str, merged_at: Option<&str>) -> GitHubPullRequest {
            GitHubPullRequest {
                number,
                title: format!("PR {number}"),
                url: format!("https://github.com/entrc/entrc-backend/pull/{number}"),
                state: state.to_string(),
                merged_at: merged_at.map(String::from),
                updated_at: Some("2026-05-01T12:00:00Z".to_string()),
            }
        }

        let now = parse_github_timestamp_millis("2026-05-01T12:00:00Z").unwrap();
        let signal = select_project_worktree_pull_request(
            &[
                pr(10, "closed", Some("2026-04-30T12:00:00Z")),
                pr(11, "open", None),
            ],
            now,
        )
        .unwrap();

        assert_eq!(signal.number, 11);
        assert_eq!(signal.state, "open");
    }

    #[test]
    fn github_pull_request_signal_accepts_recent_merged_prs() {
        let now = parse_github_timestamp_millis("2026-05-01T12:00:00Z").unwrap();
        let pull_requests = vec![GitHubPullRequest {
            number: 12,
            title: "Recent merge".to_string(),
            url: "https://github.com/entrc/entrc-backend/pull/12".to_string(),
            state: "closed".to_string(),
            merged_at: Some("2026-04-25T12:00:00Z".to_string()),
            updated_at: Some("2026-04-25T12:00:00Z".to_string()),
        }];

        let signal = select_project_worktree_pull_request(&pull_requests, now).unwrap();

        assert_eq!(signal.number, 12);
        assert_eq!(signal.state, "merged");
        assert_eq!(signal.merged_at.as_deref(), Some("2026-04-25T12:00:00Z"));
    }

    #[test]
    fn github_pull_request_signal_rejects_old_merged_prs() {
        let now = parse_github_timestamp_millis("2026-05-01T12:00:00Z").unwrap();
        let pull_requests = vec![GitHubPullRequest {
            number: 13,
            title: "Old merge".to_string(),
            url: "https://github.com/entrc/entrc-backend/pull/13".to_string(),
            state: "closed".to_string(),
            merged_at: Some("2026-04-01T12:00:00Z".to_string()),
            updated_at: Some("2026-04-01T12:00:00Z".to_string()),
        }];

        assert_eq!(select_project_worktree_pull_request(&pull_requests, now), None);
    }

    #[test]
    fn action_center_github_review_state_prefers_changes_requested() {
        let reviews = vec![
            GitHubPullRequestReview {
                state: "APPROVED".to_string(),
                user: Some(GitHubRepoPullRequestUser {
                    login: Some("reviewer-a".to_string()),
                }),
            },
            GitHubPullRequestReview {
                state: "CHANGES_REQUESTED".to_string(),
                user: Some(GitHubRepoPullRequestUser {
                    login: Some("reviewer-b".to_string()),
                }),
            },
        ];

        assert_eq!(
            derive_action_center_github_review_state(false, 0, 0, 0, &reviews),
            "changes_requested",
        );
    }

    #[test]
    fn action_center_github_pull_request_normalizes_fifo_signal() {
        let pr = GitHubRepoPullRequest {
            number: 42,
            title: "ENG-123 Add UAT action".to_string(),
            url: "https://github.com/entrc/entrc-backend/pull/42".to_string(),
            state: "open".to_string(),
            draft: false,
            user: Some(GitHubRepoPullRequestUser {
                login: Some("aryrabelo".to_string()),
            }),
            head: Some(GitHubRepoPullRequestHead {
                ref_name: Some("ENG-123-uat-action".to_string()),
            }),
            comments: Some(2),
            review_comments: Some(1),
            requested_reviewers: Vec::new(),
            created_at: Some("2026-05-01T10:00:00Z".to_string()),
            updated_at: Some("2026-05-01T12:00:00Z".to_string()),
        };
        let reviews = vec![GitHubPullRequestReview {
            state: "APPROVED".to_string(),
            user: Some(GitHubRepoPullRequestUser {
                login: Some("reviewer".to_string()),
            }),
        }];

        let signal = action_center_github_pr_from_github(
            "entrc",
            "entrc-backend",
            &pr,
            &reviews,
        );

        assert_eq!(signal.repo_full_name, "entrc/entrc-backend");
        assert_eq!(signal.branch, "ENG-123-uat-action");
        assert_eq!(signal.author, "aryrabelo");
        assert_eq!(signal.review_state, "approved");
        assert_eq!(
            signal.action_timestamp,
            parse_github_timestamp_millis("2026-05-01T10:00:00Z").unwrap(),
        );
    }

    #[test]
    fn action_center_github_pull_request_relevance_requires_author_or_requested_reviewer() {
        let unrelated_pr = GitHubRepoPullRequest {
            number: 9865,
            title: "fix: Fix package source harvest definition".to_string(),
            url: "https://github.com/entrc/entrc-backend/pull/9865".to_string(),
            state: "open".to_string(),
            draft: false,
            user: Some(GitHubRepoPullRequestUser {
                login: Some("guimello".to_string()),
            }),
            head: Some(GitHubRepoPullRequestHead {
                ref_name: Some("fix/source-harvests".to_string()),
            }),
            comments: Some(0),
            review_comments: Some(0),
            requested_reviewers: Vec::new(),
            created_at: Some("2025-02-10T11:12:58Z".to_string()),
            updated_at: Some("2026-04-08T00:32:16Z".to_string()),
        };
        assert!(!is_action_center_github_pr_relevant(&unrelated_pr, "aryrabelo"));

        let authored_pr = GitHubRepoPullRequest {
            user: Some(GitHubRepoPullRequestUser {
                login: Some("AryRabelo".to_string()),
            }),
            ..unrelated_pr
        };
        assert!(is_action_center_github_pr_relevant(&authored_pr, "aryrabelo"));

        let review_requested_pr = GitHubRepoPullRequest {
            user: Some(GitHubRepoPullRequestUser {
                login: Some("teammate".to_string()),
            }),
            requested_reviewers: vec![GitHubRepoPullRequestUser {
                login: Some("aryrabelo".to_string()),
            }],
            ..authored_pr
        };
        assert!(is_action_center_github_pr_relevant(
            &review_requested_pr,
            "aryrabelo",
        ));
    }

    #[test]
    fn action_center_linear_issue_normalizes_status_links_and_comments() {
        let issue = serde_json::json!({
            "identifier": "ENG-123",
            "title": "Move approved PR to UAT",
            "url": "https://linear.app/canix/issue/ENG-123/move-approved-pr-to-uat",
            "updatedAt": "2026-05-01T12:30:00Z",
            "state": { "name": "UAT", "type": "started" },
            "stateHistory": { "nodes": [{ "startedAt": "2026-05-01T12:00:00Z" }] },
            "assignee": { "displayName": "Ary Rabelo" },
            "labels": { "nodes": [{ "name": "backend" }] },
            "attachments": {
                "nodes": [{
                    "title": "GitHub PR",
                    "url": "https://github.com/entrc/entrc-backend/pull/42"
                }]
            },
            "comments": {
                "nodes": [
                    {
                        "user": { "isMe": false },
                        "reactions": [],
                        "children": { "nodes": [] }
                    },
                    {
                        "user": { "isMe": true },
                        "reactions": [],
                        "children": { "nodes": [] }
                    }
                ]
            }
        });

        let signal = action_center_linear_issue_from_value(&issue).unwrap();

        assert_eq!(signal.identifier, "ENG-123");
        assert_eq!(signal.status, "UAT");
        assert!(signal.is_uat);
        assert_eq!(signal.assignee.as_deref(), Some("Ary Rabelo"));
        assert_eq!(signal.labels, vec!["backend"]);
        assert_eq!(
            signal.pull_request_urls,
            vec!["https://github.com/entrc/entrc-backend/pull/42"],
        );
        assert_eq!(signal.unacked_comments, 1);
        assert_eq!(
            signal.action_timestamp,
            parse_github_timestamp_millis("2026-05-01T12:00:00Z").unwrap(),
        );
    }

    #[test]
    fn dedupe_project_worktrees_by_canonical_path_and_repo_branch() {
        fn candidate(path: &str, branch: Option<&str>, reason: &str) -> ProjectWorktreeCandidate {
            ProjectWorktreeCandidate {
                root_path: "/repos/app".to_string(),
                worktree_path: path.to_string(),
                canonical_path: path.to_string(),
                branch: branch.map(String::from),
                head: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                repo_remote: Some("git@github.com:entrc/entrc-backend.git".to_string()),
                is_root: path == "/repos/app",
                inclusion_reason: reason.to_string(),
                last_activity_at: None,
                last_activity_source: None,
                activity_scan_limited: false,
                recent_activity_rank: None,
                pull_request: None,
                pr_promoted: false,
            }
        }

        let deduped = dedupe_project_worktree_candidates(vec![
            candidate("/repos/app", Some("main"), "git-worktree-list"),
            candidate("/repos/app", Some("main"), "worktrees-directory-scan"),
            candidate("/worktrees/feature-a", Some("feature/a"), "git-worktree-list"),
            ProjectWorktreeCandidate {
                repo_remote: Some("https://github.com/entrc/entrc-backend.git".to_string()),
                ..candidate("/worktrees/feature-a-copy", Some("feature/a"), "worktrees-directory-scan")
            },
            candidate("/worktrees/feature-b", Some("feature/b"), "worktrees-directory-scan"),
        ]);

        let paths: Vec<&str> = deduped.iter().map(|item| item.canonical_path.as_str()).collect();
        assert_eq!(paths, vec!["/repos/app", "/worktrees/feature-a", "/worktrees/feature-b"]);
    }

    #[test]
    fn worktree_activity_scan_ignores_generated_directories() {
        for ignored in [
            ".git",
            "node_modules",
            "dist",
            "build",
            "target",
            ".next",
            ".nuxt",
            ".output",
            ".cache",
            ".turbo",
            "coverage",
            "vendor",
        ] {
            assert!(is_ignored_worktree_activity_dir(ignored), "{ignored} should be ignored");
        }

        assert!(!is_ignored_worktree_activity_dir("src"));
        assert!(!is_ignored_worktree_activity_dir("docs"));
    }

    #[test]
    fn worktree_activity_scan_ignores_git_file_entries() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!("borabr-worktree-activity-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".git"), "gitdir: /tmp/repo/.git/worktrees/example").unwrap();

        let (activity, limited) = newest_relevant_file_activity(&dir);

        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(activity, None);
        assert!(!limited);
    }

    #[test]
    fn recent_activity_ranking_takes_top_five_and_skips_root_and_promoted() {
        fn candidate(path: &str, is_root: bool, last_activity_at: Option<u64>) -> ProjectWorktreeCandidate {
            ProjectWorktreeCandidate {
                root_path: "/repos/app".to_string(),
                worktree_path: path.to_string(),
                canonical_path: path.to_string(),
                branch: None,
                head: None,
                repo_remote: Some("git@github.com:entrc/entrc-backend.git".to_string()),
                is_root,
                inclusion_reason: "git-worktree-list".to_string(),
                last_activity_at,
                last_activity_source: last_activity_at.map(|_| "file-mtime".to_string()),
                activity_scan_limited: false,
                recent_activity_rank: None,
                pull_request: None,
                pr_promoted: false,
            }
        }

        let mut candidates = vec![
            candidate("/repos/app", true, Some(9_000)),
            candidate("/worktrees/promoted", false, Some(8_000)),
            candidate("/worktrees/b", false, Some(7_000)),
            candidate("/worktrees/c", false, Some(6_000)),
            candidate("/worktrees/d", false, Some(5_000)),
            candidate("/worktrees/e", false, Some(4_000)),
            candidate("/worktrees/f", false, Some(3_000)),
            candidate("/worktrees/g", false, Some(2_000)),
        ];
        let promoted = HashSet::from(["/worktrees/promoted".to_string()]);

        assign_recent_activity_ranks(&mut candidates, 5, &promoted);

        let ranks: HashMap<&str, Option<usize>> = candidates
            .iter()
            .map(|candidate| (candidate.canonical_path.as_str(), candidate.recent_activity_rank))
            .collect();

        assert_eq!(ranks["/repos/app"], None);
        assert_eq!(ranks["/worktrees/promoted"], None);
        assert_eq!(ranks["/worktrees/b"], Some(1));
        assert_eq!(ranks["/worktrees/c"], Some(2));
        assert_eq!(ranks["/worktrees/d"], Some(3));
        assert_eq!(ranks["/worktrees/e"], Some(4));
        assert_eq!(ranks["/worktrees/f"], Some(5));
        assert_eq!(ranks["/worktrees/g"], None);
    }

    #[test]
    fn parse_empty_flat_array() {
        let result = parse_issues_tolerant("[]", "test_empty_flat");
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn parse_empty_envelope() {
        let json = r#"{"issues":[],"total":0,"offset":0,"limit":50,"has_more":false}"#;
        let result = parse_issues_tolerant(json, "test_empty_envelope");
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn parse_object_without_issues_key_fails() {
        let json = r#"{"error":"something went wrong"}"#;
        let result = parse_issues_tolerant(json, "test_bad_object");
        assert!(result.is_err());
    }

    #[test]
    fn parse_invalid_json_fails() {
        let result = parse_issues_tolerant("not json at all", "test_invalid");
        assert!(result.is_err());
    }

    #[test]
    fn parse_real_br_envelope() {
        // Matches the shape from br 0.1.30+ (`br list --json --limit 1`):
        // br omits many optional fields (owner, assignee, labels, etc.) and includes extra
        // fields (source_repo, compaction_level). serde_json defaults missing Option<T> to None
        // and ignores unknown fields, so this parses correctly.
        let json = r#"{"issues":[{"id":"proj-abc","title":"Example bug report","description":"A test description","status":"open","priority":2,"issue_type":"bug","created_at":"2025-06-15T09:30:00.000000000Z","updated_at":"2025-06-15T10:45:00.000000000Z","source_repo":".","compaction_level":0,"dependency_count":0,"dependent_count":0}],"total":1,"limit":1,"offset":0,"has_more":true}"#;
        let result = parse_issues_tolerant(json, "test_real_br");
        assert!(result.is_ok());
        let issues = result.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, "proj-abc");
        assert_eq!(issues[0].issue_type, "bug");
        assert_eq!(issues[0].priority, 2);
    }

    #[test]
    fn parse_envelope_skips_malformed_entries() {
        let good = minimal_issue_json("abc-123", "Good");
        let bad = r#"{"id":"bad-456","title":"Bad"}"#; // missing required fields
        let json = format!(r#"{{"issues":[{},{}],"total":2,"offset":0,"limit":50,"has_more":false}}"#, good, bad);
        let result = parse_issues_tolerant(&json, "test_tolerant_envelope");
        assert!(result.is_ok());
        let issues = result.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, "abc-123");
    }

    #[test]
    fn auto_mode_executor_command_references_prompt_file_and_task() {
        let command = build_auto_mode_agent_command(
            "borabr-unf",
            "borabr-unf.7",
            "auto-mode trigger: poll br ready when enabled, dispatch to cmux+claude",
            "workspace:42",
        );

        assert!(command.starts_with("claude "));
        assert!(command.contains(".claude/auto-mode-prompt.md"));
        assert!(command.contains("borabr-unf.7"));
        assert!(command.contains("br show borabr-unf.7"));
        assert!(command.contains("br close --actor auto-mode borabr-unf.7"));
        assert!(command.contains("Do NOT loop"));
    }

    #[test]
    fn auto_mode_epic_orchestrator_references_prompt_file() {
        let command = build_auto_mode_agent_command(
            "borabr-unf",
            "borabr-unf",
            "CMUX Task Orchestration",
            "workspace:41",
        );

        assert!(command.starts_with("claude "));
        assert!(command.contains(".claude/auto-mode-prompt.md"));
        assert!(command.contains("borabr-unf"));
        assert!(command.contains("--actor auto-mode"));
        assert!(command.contains("--json"));
        assert!(command.contains("claim"));
        assert!(command.contains("close"));
    }

    #[test]
    fn find_latest_claude_session_id_returns_none_for_nonexistent_path() {
        let result = find_latest_claude_session_id("/nonexistent/worktree/path");
        assert!(result.is_none());
    }

    #[test]
    fn build_auto_mode_resume_command_returns_none_for_nonexistent_path() {
        let result = build_auto_mode_resume_command("/nonexistent/worktree/path");
        assert!(result.is_none());
    }

    #[test]
    fn auto_mode_dispatch_scope_uses_shared_epic_branch_for_child_work() {
        let task_scope = auto_mode_dispatch_scope("borabr-unf", "borabr-unf.2");
        assert_eq!(task_scope.branch, "epic/borabr-unf");
        assert_eq!(task_scope.worktree_name, "epic-borabr-unf");
        assert_eq!(task_scope.workspace_name, "task:borabr-unf.2");
        assert!(!task_scope.is_epic);

        let epic_scope = auto_mode_dispatch_scope("borabr-unf", "borabr-unf");
        assert_eq!(epic_scope.branch, "epic/borabr-unf");
        assert_eq!(epic_scope.worktree_name, "epic-borabr-unf");
        assert_eq!(epic_scope.workspace_name, "epic:borabr-unf");
        assert!(epic_scope.is_epic);
    }

    #[test]
    fn cmux_send_prompt_targets_workspace_refs_as_workspaces() {
        assert_eq!(
            cmux_send_prompt_command("workspace:42", "hello"),
            vec![
                "send".to_string(),
                "--workspace".to_string(),
                "workspace:42".to_string(),
                "hello\\n".to_string(),
            ],
        );

        assert_eq!(
            cmux_send_prompt_command("surface:42", "hello"),
            vec![
                "send".to_string(),
                "--surface".to_string(),
                "surface:42".to_string(),
                "hello\\n".to_string(),
            ],
        );
    }

    #[test]
    fn auto_mode_in_progress_guard_ignores_epics() {
        let epic = BdRawIssue {
            id: "borabr-unf".to_string(),
            title: "Epic".to_string(),
            description: None,
            status: "in_progress".to_string(),
            priority: 0,
            issue_type: "epic".to_string(),
            owner: None,
            assignee: None,
            labels: None,
            created_at: "2026-05-01T12:00:00Z".to_string(),
            created_by: None,
            updated_at: "2026-05-01T12:00:00Z".to_string(),
            closed_at: None,
            close_reason: None,
            blocked_by: None,
            blocks: None,
            comments: None,
            external_ref: None,
            estimate: None,
            design: None,
            acceptance_criteria: None,
            notes: None,
            parent: None,
            dependents: None,
            dependencies: None,
            dependency_count: None,
            dependent_count: None,
            metadata: None,
            spec_id: None,
            comment_count: None,
        };
        let task = BdRawIssue {
            id: "borabr-unf.1".to_string(),
            issue_type: "task".to_string(),
            ..epic.clone()
        };

        assert!(!has_in_progress_non_epic_issue(&[epic]));
        assert!(has_in_progress_non_epic_issue(&[task]));
    }

    #[test]
    fn reviewer_command_includes_task_context_and_quality_gates() {
        let command = build_auto_mode_reviewer_command(
            "borabr-unf",
            "borabr-unf.4",
            "review agent gate",
            "task-borabr-unf.4",
            "abc1234",
            "workspace:50",
        );

        assert!(command.starts_with("claude "));
        assert!(command.contains("independent reviewer"));
        assert!(command.contains("Beads task borabr-unf.4"));
        assert!(command.contains("epic borabr-unf"));
        assert!(command.contains("fresh context"));
        assert!(command.contains("no bias"));
        assert!(command.contains("Branch: `task-borabr-unf.4`"));
        assert!(command.contains("executor commit: abc1234"));
        assert!(command.contains("br show borabr-unf.4"));
        assert!(command.contains("git diff master...task-borabr-unf.4"));
        assert!(command.contains("pnpm test"));
        assert!(command.contains("vue-tsc --noEmit"));
        assert!(command.contains("br comments add --actor auto-mode borabr-unf.4"));
        assert!(command.contains("REVIEW_VERDICT"));
        assert!(command.contains("APPROVED"));
        assert!(command.contains("CHANGES_REQUESTED"));
        assert!(command.contains("Do not modify code"));
    }

    #[test]
    fn reviewer_command_uses_correct_assignee_format() {
        let command = build_auto_mode_reviewer_command(
            "epic-1",
            "epic-1.3",
            "some task",
            "task-epic-1.3",
            "def5678",
            "workspace:99",
        );

        assert!(command.contains("cmux:workspace:99"));
    }
}
