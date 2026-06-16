use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::OnceCell;

/// Stable, machine-recognizable marker prefixing a **permanent** policy
/// rejection: the identical `(tool, args)` call can never succeed in the
/// current tier (read-only blocking a write, a forbidden/credential path, a
/// disallowed high-risk or hidden-execution command, an off-allowlist command).
/// The agent harness ([`crate::openhuman::agent::harness::tool_loop`]) detects
/// this and halts on the **first verbatim repeat** rather than reiterating a
/// provably-futile call. Kept short and bracketed so it survives the
/// `Error: …` wrapping the tool layer adds and is easy to grep in logs.
pub const POLICY_BLOCKED_MARKER: &str = "[policy-blocked]";

/// Stable marker prefixing a **this-turn denial** — the user answered "no" to
/// an approval prompt, or the prompt timed out / its channel dropped. Unlike a
/// block this isn't permanent across turns, but re-issuing the *same* call this
/// turn just re-prompts the user, so the harness records it in the circuit
/// breaker and stops the agent from re-asking the identical call.
pub const POLICY_DENIED_MARKER: &str = "[policy-denied]";

/// How much autonomy the agent has
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AutonomyLevel {
    /// Read-only: can observe but not act
    ReadOnly,
    /// Supervised: acts but requires approval for risky operations
    #[default]
    Supervised,
    /// Full: autonomous execution within policy bounds
    Full,
}

/// Access level granted to a trusted root outside the workspace.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TrustedAccess {
    /// Read + list only.
    #[default]
    Read,
    /// Read and write/edit.
    ReadWrite,
}

/// A directory outside the workspace the agent is explicitly granted access to.
/// Takes precedence over `workspace_only` and `forbidden_paths` for its subtree,
/// except for credential stores (see `SecurityPolicy::is_always_forbidden`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TrustedRoot {
    /// Absolute path (a leading `~` is expanded to the user's home).
    pub path: String,
    /// Whether the agent may write within this root.
    #[serde(default)]
    pub access: TrustedAccess,
}

/// Risk score for shell command execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRiskLevel {
    Low,
    Medium,
    High,
}

/// Coarse permission bucket the harness approval gate keys on.
///
/// Classification is **fail-closed**: a command that is not provably read-only
/// (and not a recognized network/destructive command) is treated as at least
/// [`CommandClass::Write`]. Across multiple shell segments the **highest** class
/// wins (so `ls | curl …` is `Network`). Variants are ordered low→high so
/// [`Ord`] / [`Iterator::max`] compose them directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandClass {
    /// Provably read-only / observational (curated safe-read allowlist).
    Read,
    /// State-changing but not inherently catastrophic — the fail-closed default
    /// for anything not recognized as read/network/destructive.
    Write,
    /// Reaches the network (curl/wget/ssh/scp/…). Always prompts, every tier.
    Network,
    /// Installs an OS / language package (system package manager, or a *global*
    /// npm/pnpm/yarn/cargo/pip install). Always-ask in every acting tier,
    /// including Full — mirrors the dedicated `install_tool` gate so shell
    /// installs can't slip past it. Project-local installs are ordinary `Write`.
    Install,
    /// Catastrophic / irreversible / privilege-escalating / system-control.
    /// Always prompts, even in Full.
    Destructive,
}

/// What the harness should do with an acting tool call of a given
/// [`CommandClass`] under the session's [`AutonomyLevel`]. Computed by
/// [`SecurityPolicy::gate_decision`]; the harness translates `Prompt` into an
/// `ApprovalGate` round-trip *before* the tool's `execute()` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    /// Run without prompting.
    Allow,
    /// Require explicit human approval before running.
    Prompt,
    /// Refuse outright — no in-tier prompt can authorize it (e.g. any act in
    /// read-only mode).
    Block,
}

/// Classifies whether a tool operation is read-only or side-effecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOperation {
    Read,
    Act,
}

/// Sliding-window action tracker for rate limiting.
#[derive(Debug)]
pub struct ActionTracker {
    /// Timestamps of recent actions (kept within the last hour).
    actions: Mutex<Vec<Instant>>,
}

impl Default for ActionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionTracker {
    pub fn new() -> Self {
        Self {
            actions: Mutex::new(Vec::new()),
        }
    }

    /// Record an action and return the current count within the window.
    pub fn record(&self) -> usize {
        let mut actions = self.actions.lock();
        let cutoff = Instant::now()
            .checked_sub(std::time::Duration::from_secs(3600))
            .unwrap_or_else(Instant::now);
        actions.retain(|t| *t > cutoff);
        actions.push(Instant::now());
        actions.len()
    }

    /// Count of actions in the current window without recording.
    pub fn count(&self) -> usize {
        let mut actions = self.actions.lock();
        let cutoff = Instant::now()
            .checked_sub(std::time::Duration::from_secs(3600))
            .unwrap_or_else(Instant::now);
        actions.retain(|t| *t > cutoff);
        actions.len()
    }
}

impl Clone for ActionTracker {
    fn clone(&self) -> Self {
        let actions = self.actions.lock();
        Self {
            actions: Mutex::new(actions.clone()),
        }
    }
}

/// Subdirectories under `workspace_dir` that hold internal application state
/// (memory DBs, sessions, tokens, etc.) and must not be writable by agent tools.
pub(super) const WORKSPACE_INTERNAL_DIRS: &[&str] = &[
    "memory",
    "memory_tree",
    "state",
    "approval",
    "sessions",
    "session_raw",
    "cron",
    "devices",
    "mcp_clients",
    "subconscious",
    "vault",
    "task_sources",
    "whatsapp_data",
    "redirect_links",
    "codegraph",
    ".openhuman",
];

/// Files directly under `workspace_dir` that hold secrets or persona config
/// and must not be writable by agent tools.
pub(super) const WORKSPACE_INTERNAL_FILES: &[&str] = &[
    "core.token",
    "dev-keychain.json",
    ".env",
    "SOUL.md",
    "IDENTITY.md",
    "HEARTBEAT.md",
    "PROFILE.md",
];

/// Security policy enforced on all tool executions
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    pub autonomy: AutonomyLevel,
    pub workspace_dir: PathBuf,
    /// Agent action sandbox root — tools resolve relative paths and default
    /// their cwd here instead of `workspace_dir`. Kept separate so internal
    /// state (memory DBs, sessions, tokens) under `workspace_dir` is not
    /// reachable from agent tool calls.
    pub action_dir: PathBuf,
    pub workspace_only: bool,
    pub allowed_commands: Vec<String>,
    pub forbidden_paths: Vec<String>,
    pub max_actions_per_hour: u32,
    pub max_cost_per_day_cents: u32,
    pub require_approval_for_medium_risk: bool,
    pub block_high_risk_commands: bool,
    /// Directories outside the workspace the agent may access (read or read-write).
    pub trusted_roots: Vec<TrustedRoot>,
    /// Whether the agent may install OS packages via the `install_tool` tool.
    pub allow_tool_install: bool,
    /// Tool names the user has pre-approved ("Always allow"). The `ApprovalGate`
    /// skips the interactive prompt for any tool in this set. Sourced from
    /// `autonomy.auto_approve`; populated/cleared via `config.update_autonomy_settings`
    /// (or an "Always allow" decision) and observed live via `live_policy`.
    pub auto_approve: Vec<String>,
    pub tracker: ActionTracker,
    /// Lazily-cached canonical form of [`workspace_dir`].
    ///
    /// `validate_path` / `validate_parent_path` use the canonical workspace
    /// root to check resolved paths against `forbidden_paths`. Without a cache
    /// each call invokes `tokio::fs::canonicalize(&workspace_dir)` — one
    /// `stat(2)` + symlink walk on the same path on every file op. A single
    /// agent turn doing tens of read/edit/shell-path validations hits this
    /// repeatedly with identical input.
    ///
    /// `workspace_dir` is effectively immutable for a given `SecurityPolicy`
    /// (a config update builds a *new* policy via `from_config` and swaps the
    /// `Arc` in [`live_policy`]), so caching the resolved value is safe and
    /// stays correct across config updates.
    ///
    /// `Arc<OnceCell<_>>` so the struct stays `Clone` (clone the `Arc`) and
    /// init happens lazily on the first async call site without blocking
    /// constructors. Fallback (raw `workspace_dir` if canonicalize fails)
    /// matches the previous inline behavior exactly.
    ///
    /// Visibility is `pub` to match every other field on the struct: external
    /// crates (Cargo examples, downstream consumers) construct
    /// `SecurityPolicy` with the `..SecurityPolicy::default()` functional-update
    /// spread, and Rust requires every field of the target struct to be
    /// visible to the caller in that syntax — even fields supplied by the
    /// default. `pub(crate)` was an over-tight first cut that broke
    /// `examples/mouse_smoke.rs` with E0451.
    pub canonical_workspace: Arc<OnceCell<PathBuf>>,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: PathBuf::from("."),
            action_dir: PathBuf::from("."),
            workspace_only: true,
            // When adding a new entry to this allowlist, re-audit
            // `DANGEROUS_ENV_PREFIXES` (see below). Every newly-allowed binary
            // may introduce its own env-driven subprocess hooks (pager, editor,
            // loader override, SSH/diff helper, preprocessor) — those names
            // must be added to the prefix denylist so that the
            // `KEY=cmd <allowed-binary>` shape cannot bypass allowlisting via
            // `skip_env_assignments` in `is_command_allowed`. Cross-ref #2636.
            allowed_commands: vec![
                // Version control
                "git".into(),
                // Package managers / build systems
                "npm".into(),
                "pnpm".into(),
                "yarn".into(),
                "cargo".into(),
                "make".into(),
                "cmake".into(),
                // Directory / file inspection (read-only, low-risk)
                "ls".into(),
                "cat".into(),
                "grep".into(),
                "find".into(),
                "echo".into(),
                "pwd".into(),
                "wc".into(),
                "head".into(),
                "tail".into(),
                "date".into(),
                "sort".into(),
                "uniq".into(),
                "diff".into(),
                "which".into(),
                "uname".into(),
                "basename".into(),
                "dirname".into(),
                "tr".into(),
                "cut".into(),
                "realpath".into(),
                "readlink".into(),
                "stat".into(),
                "file".into(),
                // Filesystem mutations (medium-risk — require approval in Supervised mode)
                "mkdir".into(),
                "touch".into(),
                "cp".into(),
                "mv".into(),
                "ln".into(),
                // Windows read-only equivalents for the same basic
                // inspection workflows as ls/cat/grep/which.
                "dir".into(),
                "type".into(),
                "where".into(),
                "findstr".into(),
                "more".into(),
            ],
            forbidden_paths: vec![
                // System directories (blocked even when workspace_only=false)
                "/etc".into(),
                "/root".into(),
                "/home".into(),
                "/usr".into(),
                "/bin".into(),
                "/sbin".into(),
                "/lib".into(),
                "/opt".into(),
                "/boot".into(),
                "/dev".into(),
                "/proc".into(),
                "/sys".into(),
                "/var".into(),
                "/tmp".into(),
                // Sensitive dotfiles
                "~/.ssh".into(),
                "~/.gnupg".into(),
                "~/.aws".into(),
                "~/.config".into(),
            ],
            // Effectively unlimited — matches AutonomyConfig::default_max_actions_per_hour().
            // The rate-limiter check is `count <= max`, so u32::MAX is functionally
            // infinite without requiring an Option sentinel on the field type.
            max_actions_per_hour: u32::MAX,
            max_cost_per_day_cents: 500,
            require_approval_for_medium_risk: true,
            block_high_risk_commands: true,
            trusted_roots: Vec::new(),
            allow_tool_install: false,
            auto_approve: Vec::new(),
            tracker: ActionTracker::new(),
            canonical_workspace: Arc::new(OnceCell::new()),
        }
    }
}
