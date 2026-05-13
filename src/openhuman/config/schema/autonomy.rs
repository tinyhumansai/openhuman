//! Autonomy and security policy configuration.

use super::defaults;
use crate::openhuman::security::AutonomyLevel;
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
    #[serde(default = "default_auto_approve")]
    pub auto_approve: Vec<String>,
    #[serde(default = "default_always_ask")]
    pub always_ask: Vec<String>,
}

fn default_true() -> bool {
    defaults::default_true()
}

fn default_max_actions_per_hour() -> u32 {
    20
}

fn default_max_cost_per_day_cents() -> u32 {
    500
}

fn default_allowed_commands() -> Vec<String> {
    vec![
        "git".into(),
        "npm".into(),
        "cargo".into(),
        "ls".into(),
        "cat".into(),
        "grep".into(),
        "find".into(),
        "echo".into(),
        "pwd".into(),
        "wc".into(),
        "head".into(),
        "tail".into(),
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
        "file_read".into(),
        "memory_search".into(),
        "memory_list".into(),
        "get_time".into(),
        "list_dir".into(),
    ]
}

fn default_always_ask() -> Vec<String> {
    vec![]
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
            always_ask: default_always_ask(),
        }
    }
}
