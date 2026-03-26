//! Autonomy and security policy configuration.

use crate::openhuman::security::AutonomyLevel;
use super::defaults;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutonomyConfig {
    pub level: AutonomyLevel,
    pub workspace_only: bool,
    pub allowed_commands: Vec<String>,
    pub forbidden_paths: Vec<String>,
    pub max_actions_per_hour: u32,
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
            workspace_only: true,
            allowed_commands: vec![
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
            ],
            forbidden_paths: vec![
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
            ],
            max_actions_per_hour: 20,
            max_cost_per_day_cents: 500,
            require_approval_for_medium_risk: true,
            block_high_risk_commands: true,
            auto_approve: default_auto_approve(),
            always_ask: default_always_ask(),
        }
    }
}
