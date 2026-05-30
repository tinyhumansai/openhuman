//! Autonomy and security policy configuration.

use super::defaults;
use crate::openhuman::security::{AutonomyLevel, TrustedRoot};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AutonomyConfig {
    // No field-level override needed — AutonomyLevel's #[default] is Supervised,
    // matching the struct Default.
    pub level: AutonomyLevel,
    #[serde(default = "default_true")]
    pub workspace_only: bool,
    #[serde(default = "default_allowed_commands")]
    pub allowed_commands: Vec<String>,
    #[serde(default = "default_forbidden_paths")]
    pub forbidden_paths: Vec<String>,
    #[serde(default = "default_max_actions_per_hour")]
    pub max_actions_per_hour: u32,
    #[serde(default = "default_max_cost_per_day_cents")]
    pub max_cost_per_day_cents: u32,
    #[serde(default = "default_true")]
    pub require_approval_for_medium_risk: bool,
    #[serde(default = "default_true")]
    pub block_high_risk_commands: bool,
    /// Tool names the user has pre-approved ("Always allow"). The `ApprovalGate`
    /// skips the interactive prompt for any tool listed here. Populated by the
    /// "Always allow" approval decision or hand-edited, and surfaced in
    /// Settings → Agent access. Read live by the gate via `SecurityPolicy`.
    #[serde(default = "default_auto_approve")]
    pub auto_approve: Vec<String>,
    /// Directories outside the workspace the agent may access. Each entry grants
    /// read (or read+write) to its subtree, taking precedence over `workspace_only`
    /// and `forbidden_paths` — except credential stores (~/.ssh, ~/.gnupg, ~/.aws),
    /// which stay blocked regardless.
    #[serde(default)]
    pub trusted_roots: Vec<TrustedRoot>,
    /// Whether the agent may install OS packages via the `install_tool` tool.
    /// Intended to be enabled only in Full access mode.
    #[serde(default)]
    pub allow_tool_install: bool,
    /// When enabled, an agent-authored task brief must be approved before an
    /// assigned agent treats it as executable work.
    #[serde(default = "default_true")]
    pub require_task_plan_approval: bool,
}

fn default_true() -> bool {
    defaults::default_true()
}

fn default_max_actions_per_hour() -> u32 {
    // Effectively unlimited. The rate-limiter check is `count <= max`, so any
    // ceiling above realistic per-hour traffic is functionally infinite;
    // u32::MAX lets the field stay a plain `u32` without a sentinel option.
    u32::MAX
}

fn default_max_cost_per_day_cents() -> u32 {
    500
}

fn default_allowed_commands() -> Vec<String> {
    vec![
        // Version control
        "git".into(),
        // Package managers / build systems. `make` can run arbitrary recipes,
        // but the shell policy still gates execution to this command allow-list
        // and Supervised mode approval remains responsible for risky invocations.
        "npm".into(),
        "pnpm".into(),
        "yarn".into(),
        "cargo".into(),
        "make".into(),
        "cmake".into(),
        // Directory / file inspection (read-only)
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
        // Windows read-only equivalents for ls/cat/grep/which
        "dir".into(),
        "type".into(),
        "where".into(),
        "findstr".into(),
        "more".into(),
    ]
}

fn default_forbidden_paths() -> Vec<String> {
    vec![
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
        "~/.ssh".into(),
        "~/.gnupg".into(),
        "~/.aws".into(),
        "~/.config".into(),
    ]
}

fn default_auto_approve() -> Vec<String> {
    vec![
        // Read-only tools — always safe to skip the approval prompt
        "file_read".into(),
        "memory_search".into(),
        "memory_list".into(),
        "get_time".into(),
        "list_dir".into(),
        // Workspace-scoped search tools — read-only, no side effects
        "glob".into(),
        "grep".into(),
    ]
}

impl Default for AutonomyConfig {
    fn default() -> Self {
        Self {
            level: AutonomyLevel::Supervised,
            workspace_only: default_true(),
            allowed_commands: default_allowed_commands(),
            forbidden_paths: default_forbidden_paths(),
            max_actions_per_hour: default_max_actions_per_hour(),
            max_cost_per_day_cents: default_max_cost_per_day_cents(),
            require_approval_for_medium_risk: default_true(),
            block_high_risk_commands: default_true(),
            auto_approve: default_auto_approve(),
            trusted_roots: Vec::new(),
            allow_tool_install: false,
            require_task_plan_approval: default_true(),
        }
    }
}
