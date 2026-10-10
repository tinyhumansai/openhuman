use crate::config::PrivacyMode;
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
/// The agent harness's repeated-failure middleware
/// ([`crate::agent::tinyagents::middleware::RepeatedToolFailureMiddleware`])
/// detects this and halts on the **first verbatim repeat** rather than
/// reiterating a provably-futile call. Kept short and bracketed so it survives the
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
/// except for credential stores and workspace-internal application state (see
/// `SecurityPolicy::is_always_forbidden` and
/// `SecurityPolicy::is_workspace_internal_path`).
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

/// Coarse permission bucket the harness approval gate keys on. Defined with the
/// classifier in `tinybox_core::shell::classify`; re-exported here so host
/// callers keep their path.
pub use tinybox_core::shell::classify::CommandClass;

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
    // Per-profile homes contain prompt-controlling SOUL.md files and private
    // skills. They are core-managed state, never part of the agent action
    // surface, even when a trusted root otherwise reaches workspace_dir.
    "personalities",
    "cron",
    "devices",
    "mcp_clients",
    "vault",
    "task_sources",
    // The whatsapp_data store was removed along with the scanner that wrote it,
    // but an upgraded profile can still hold `whatsapp_data/whatsapp_data.db`
    // (chat and message history) from an older version. Keep the directory on
    // the internal denylist so agents with workspace access cannot read it.
    "whatsapp_data",
    // The retired background-reasoning engine kept its SQLite state under
    // `subconscious/`. The engine is gone, but an upgraded profile can still
    // hold that database (memory-derived reflections). Keep it denied.
    "subconscious",
    // The redirect_links domain was removed (#5051), but an upgraded profile can
    // still hold a legacy `redirect_links/links.db` (stored URL history) written
    // by an older version. Keep the directory on the internal denylist so agents
    // with workspace access cannot read or overwrite that leftover state.
    "redirect_links",
    // The codegraph domain was removed, but upgraded workspaces can retain its
    // internal index. Keep legacy state inaccessible to agent file tools.
    "codegraph",
    ".openhuman",
    // A removed relay domain may leave encrypted identity/session state in an
    // upgraded workspace. Keep that legacy directory private.
    "tinyplace",
];

/// Per-profile internal-state directory families under the workspace.
pub(super) const WORKSPACE_INTERNAL_PREFIXES: &[&str] =
    &["memory-", "memory_tree-", "session_raw-"];

/// The artifact store under `workspace_dir`. Its per-artifact directories are
/// internal state (see `is_workspace_internal_path`); only
/// [`ARTIFACT_TOOL_RESULTS_DIR`] inside it stays agent-readable.
pub(super) const ARTIFACTS_DIR: &str = "artifacts";
/// The account config file, stored beside `workspace_dir` (see
/// `is_workspace_internal_path`).
pub(super) const ACCOUNT_CONFIG_FILE: &str = "config.toml";
/// Where oversized tool outputs are persisted for the agent to read back.
pub(super) const ARTIFACT_TOOL_RESULTS_DIR: &str = "tool-results";

/// Where oversized tool outputs are persisted for the agent to read back:
/// `<workspace_dir>/artifacts/tool-results`.
///
/// Inside the core's own state, not the agent's working directory. The action
/// directory is often a project the agent is editing, and a tool output saved
/// there becomes a stray file in that project (picked up by `git add -A`, shown
/// in the diff). This is the one subdirectory of the internal `artifacts/` store
/// agent file tools may read (`is_workspace_internal_path`), and `from_config`
/// grants it as a read-only root so the absolute pointer the store hands out
/// stays readable when `workspace_only` refuses other absolute paths.
pub fn tool_result_artifacts_dir(workspace_dir: &std::path::Path) -> PathBuf {
    workspace_dir
        .join(ARTIFACTS_DIR)
        .join(ARTIFACT_TOOL_RESULTS_DIR)
}

/// Files directly under `workspace_dir` that hold secrets or persona config
/// and must not be writable by agent tools.
pub(super) const WORKSPACE_INTERNAL_FILES: &[&str] = &[
    "core.token",
    "dev-keychain.json",
    ".env",
    "SOUL.md",
    "IDENTITY.md",
    // No longer seeded or read (#5701), but an upgraded workspace can still
    // hold one; keep it out of the agent-writable surface.
    "HEARTBEAT.md",
    "PROFILE.md",
];

/// Security policy enforced on all tool executions
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    /// Master switch for the autonomy policy. **Off by default** — these agents
    /// run inside containers, platform jails and Docker sandboxes that already
    /// provide the isolation this in-process policy was approximating, and an
    /// agent whose shell refuses ordinary shell syntax is not a usable agent.
    ///
    /// When `false`, command classification, the approval gate, the command
    /// allowlist, the hourly action budget, `workspace_only`, `forbidden_paths`
    /// and the workspace-internal boundary are all skipped.
    ///
    /// [`Self::is_always_forbidden`] still applies either way: credential
    /// stores (`~/.ssh`, `~/.gnupg`, `~/.aws`, …) and system roots stay
    /// unreachable. That is a deliberate floor, not an oversight — it has never
    /// blocked legitimate agent work, and removing it is a one-line change if
    /// a host genuinely wants no path policy at all.
    ///
    /// Set `[autonomy] enabled = true` to restore the full policy.
    ///
    /// Note the asymmetry with [`Default`]: a policy built from config defaults
    /// to disabled, a policy built with no config at all defaults to enabled.
    /// See the comment on the `Default` impl below.
    pub enabled: bool,
    pub autonomy: AutonomyLevel,
    /// Data-egress posture (Privacy Mode) — DISTINCT from `autonomy`, which
    /// governs act-power. `LocalOnly` blocks external model calls at the
    /// inference chokepoint (see
    /// [`create_chat_provider_from_string`](crate::inference::provider::factory)).
    /// Sourced from `config.privacy.mode` at policy-build time and hot-swapped
    /// via [`live_policy::reload_privacy`](crate::security::live_policy::reload_privacy).
    ///
    /// HOOK (later slices): `Sensitive` mode enforcement (approval / redaction /
    /// destination disclosure — S2/S4/S7) and `LocalOnly` enforcement for
    /// integrations / network tools (S5/S6) branch on this field. Those arms are
    /// intentionally NOT implemented in S1 (#4435).
    pub privacy_mode: PrivacyMode,
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
    /// When true, the approval gate auto-approves ALL tool calls without
    /// prompting — a blanket bypass, not just the `auto_approve` allowlist
    /// above. `AgentTurnOrigin::Unknown` origins are still denied by the gate
    /// regardless of this flag. A remote-origin triage dispatch is *not* in
    /// that protected set: with this flag on it is allowed without parking and
    /// without a `pending_approvals` audit row (openhuman#5634, accepted).
    /// Sourced from `autonomy.auto_approve_all`;
    /// observed live via `live_policy`. Does not affect
    /// `is_always_forbidden`, `is_workspace_internal_path`, or
    /// `ToolPolicyMiddleware`, which are independent code paths.
    pub auto_approve_all: bool,
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
            // Deliberately the opposite of `AutonomyConfig::default()`, which is
            // `false`. That one is the *shipped* default, chosen once, from
            // config, at the single `from_config` chokepoint. This one is the
            // fallback for a `SecurityPolicy` built without any config at all —
            // bare `SecurityPolicy::default()` constructions in tests, examples
            // and a handful of internal call sites — and there fail-closed is
            // right: a policy nobody configured should not be a policy nobody
            // enforces.
            enabled: true,
            autonomy: AutonomyLevel::Supervised,
            privacy_mode: PrivacyMode::Standard,
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
            auto_approve_all: false,
            tracker: ActionTracker::new(),
            canonical_workspace: Arc::new(OnceCell::new()),
        }
    }
}
