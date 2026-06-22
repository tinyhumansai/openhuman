//! Runtime (native/docker), reliability, and scheduler configuration.

use super::defaults;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RuntimeConfig {
    #[serde(default = "default_runtime_kind")]
    pub kind: String,
    #[serde(default)]
    pub docker: DockerRuntimeConfig,
    #[serde(default)]
    pub reasoning_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DockerRuntimeConfig {
    #[serde(default = "default_docker_image")]
    pub image: String,
    #[serde(default = "default_docker_network")]
    pub network: String,
    #[serde(default = "default_docker_memory_limit_mb")]
    pub memory_limit_mb: Option<u64>,
    #[serde(default = "default_docker_cpu_limit")]
    pub cpu_limit: Option<f64>,
    #[serde(default = "default_true")]
    pub read_only_rootfs: bool,
    #[serde(default = "default_true")]
    pub mount_workspace: bool,
    #[serde(default)]
    pub allowed_workspace_roots: Vec<String>,
}

/// `[shell]` — behaviour of the shell-family tools (`shell`, `node_exec`,
/// `npm_exec`, monitor) when they spawn child processes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ShellConfig {
    /// On Windows, suppress the console window that briefly flashes for every
    /// child process the shell tool spawns by passing `CREATE_NO_WINDOW`
    /// (`0x08000000`) in the process creation flags. No-op on macOS/Linux.
    /// Defaults to `false` for backward compatibility.
    #[serde(default)]
    pub hide_window: bool,
}

fn default_true() -> bool {
    defaults::default_true()
}

fn default_runtime_kind() -> String {
    "native".into()
}

fn default_docker_image() -> String {
    "alpine:3.20".into()
}

fn default_docker_network() -> String {
    "none".into()
}

fn default_docker_memory_limit_mb() -> Option<u64> {
    Some(512)
}

fn default_docker_cpu_limit() -> Option<f64> {
    Some(1.0)
}

impl Default for DockerRuntimeConfig {
    fn default() -> Self {
        Self {
            image: default_docker_image(),
            network: default_docker_network(),
            memory_limit_mb: default_docker_memory_limit_mb(),
            cpu_limit: default_docker_cpu_limit(),
            read_only_rootfs: true,
            mount_workspace: true,
            allowed_workspace_roots: Vec::new(),
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            kind: default_runtime_kind(),
            docker: DockerRuntimeConfig::default(),
            reasoning_enabled: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReliabilityConfig {
    #[serde(default = "default_provider_retries")]
    pub provider_retries: u32,
    #[serde(default = "default_provider_backoff_ms")]
    pub provider_backoff_ms: u64,
    #[serde(default)]
    pub fallback_providers: Vec<String>,
    #[serde(default)]
    pub model_fallbacks: HashMap<String, Vec<String>>,
    #[serde(default = "default_channel_backoff_secs")]
    pub channel_initial_backoff_secs: u64,
    #[serde(default = "default_channel_backoff_max_secs")]
    pub channel_max_backoff_secs: u64,
    #[serde(default = "default_scheduler_poll_secs")]
    pub scheduler_poll_secs: u64,
    #[serde(default = "default_scheduler_retries")]
    pub scheduler_retries: u32,
}

fn default_provider_retries() -> u32 {
    2
}

fn default_provider_backoff_ms() -> u64 {
    500
}

fn default_channel_backoff_secs() -> u64 {
    2
}

fn default_channel_backoff_max_secs() -> u64 {
    60
}

fn default_scheduler_poll_secs() -> u64 {
    15
}

fn default_scheduler_retries() -> u32 {
    2
}

impl Default for ReliabilityConfig {
    fn default() -> Self {
        Self {
            provider_retries: default_provider_retries(),
            provider_backoff_ms: default_provider_backoff_ms(),
            fallback_providers: Vec::new(),
            model_fallbacks: HashMap::new(),
            channel_initial_backoff_secs: default_channel_backoff_secs(),
            channel_max_backoff_secs: default_channel_backoff_max_secs(),
            scheduler_poll_secs: default_scheduler_poll_secs(),
            scheduler_retries: default_scheduler_retries(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SchedulerConfig {
    #[serde(default = "default_scheduler_enabled")]
    pub enabled: bool,
    #[serde(default = "default_scheduler_max_tasks")]
    pub max_tasks: usize,
    #[serde(default = "default_scheduler_max_concurrent")]
    pub max_concurrent: usize,
}

fn default_scheduler_enabled() -> bool {
    true
}

fn default_scheduler_max_tasks() -> usize {
    64
}

fn default_scheduler_max_concurrent() -> usize {
    4
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: default_scheduler_enabled(),
            max_tasks: default_scheduler_max_tasks(),
            max_concurrent: default_scheduler_max_concurrent(),
        }
    }
}

#[cfg(test)]
mod shell_config_tests {
    use super::ShellConfig;

    #[test]
    fn shell_config_defaults_hide_window_off() {
        // Backward compatibility: absent `[shell]` section must not change
        // behaviour, so `hide_window` defaults to false.
        assert!(!ShellConfig::default().hide_window);
    }

    #[test]
    fn shell_config_parses_hide_window_from_toml() {
        let cfg: ShellConfig = toml::from_str("hide_window = true").unwrap();
        assert!(cfg.hide_window);
    }

    #[test]
    fn shell_config_empty_table_keeps_default() {
        // An empty `[shell]` table relies on `#[serde(default)]` for the field.
        let cfg: ShellConfig = toml::from_str("").unwrap();
        assert!(!cfg.hide_window);
    }
}
