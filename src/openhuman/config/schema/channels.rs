//! Channel provider configuration re-exported from tinychannels, plus
//! OpenHuman-owned security/sandbox configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use tinychannels::config::{
    ChannelsConfig, DingTalkConfig, DiscordConfig, EmailConfig, IMessageConfig, IrcConfig,
    LarkConfig, LarkReceiveMode, LinqConfig, MatrixConfig, MattermostConfig, QQConfig,
    SignalConfig, SlackConfig, StreamMode, TelegramConfig, WebhookConfig, WhatsAppConfig,
    YuanbaoConfig,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct SecurityConfig {
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub resources: ResourceLimitsConfig,
    #[serde(default)]
    pub audit: AuditConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SandboxConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub backend: SandboxBackend,
    #[serde(default)]
    pub firejail_args: Vec<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: None,
            backend: SandboxBackend::Auto,
            firejail_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SandboxBackend {
    #[default]
    Auto,
    Landlock,
    Firejail,
    Bubblewrap,
    Docker,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct ResourceLimitsConfig {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AuditConfig {
    #[serde(default = "default_audit_enabled")]
    pub enabled: bool,
    #[serde(default = "default_audit_log_path")]
    pub log_path: String,
    #[serde(default = "default_audit_max_size_mb")]
    pub max_size_mb: u32,
}

fn default_audit_enabled() -> bool {
    true
}

fn default_audit_log_path() -> String {
    "audit.log".to_string()
}

fn default_audit_max_size_mb() -> u32 {
    100
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: default_audit_enabled(),
            log_path: default_audit_log_path(),
            max_size_mb: default_audit_max_size_mb(),
        }
    }
}

#[cfg(test)]
#[path = "channels_tests.rs"]
mod tests;
