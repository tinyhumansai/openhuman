//! The single `[security]` table: audit, approval and the overlay that carries
//! the sandbox, autonomy and privacy policy.
//!
//! Policy settings historically lived in three top-level tables (`[autonomy]`,
//! `[sandbox]`, `[privacy]`). `[security.autonomy]`, `[security.sandbox]` and
//! `[security.privacy]` are the canonical spelling; the pre-decode rewrite in
//! [`crate::config::migrations::security_policy`] overlays them onto the legacy
//! tables key by key (the `[security.*]` value wins) before the typed decode, so
//! consumers keep reading `Config::{autonomy, sandbox, privacy}`. The policy
//! stays off unless `autonomy.enabled = true`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `[security]`: everything that decides what an agent may do.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(default)]
pub struct SecurityConfig {
    /// `[security.audit]`: append-only audit log of policy decisions.
    pub audit: AuditConfig,
    /// `[security.approval]`: interactive approval timing.
    pub approval: ApprovalSettings,
    /// `[security.auto_approve]`: automatic approval beyond the per-tool list.
    pub auto_approve: AutoApproveSettings,
}

/// `[security.auto_approve]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AutoApproveSettings {
    /// `[security.auto_approve.judge]`: the Jev judge. Off by default; any
    /// judge failure, low confidence or budget exhaustion falls to a human.
    pub judge: JudgeSettings,
}

/// `[security.auto_approve.judge]`. Scores are in thousandths (`900` = 0.9).
/// The credential is never stored here: it is resolved with
/// `resolve_backend_credential` when the module is configured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct JudgeSettings {
    /// Master switch. Default `false`.
    pub enabled: bool,
    /// Origins that opt in (`operator`, `copilot`, `sub_agent`, `cron`,
    /// `saas`). Empty means no origin is judged, even when `enabled`.
    pub origins: Vec<String>,
    /// Minimum intent and reversibility score.
    pub threshold_millis: u16,
    /// Maximum exfiltration score.
    pub exfiltrate_millis: u16,
    /// Total request deadline in milliseconds.
    pub timeout_ms: u32,
    /// Explicit evaluation endpoint; empty uses the provider default.
    pub endpoint: String,
    /// Requests allowed per budget window.
    pub max_requests: u32,
    /// Budget window in milliseconds.
    pub budget_window_ms: u32,
}

impl Default for JudgeSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            origins: Vec::new(),
            threshold_millis: 900,
            exfiltrate_millis: 100,
            timeout_ms: 5_000,
            endpoint: String::new(),
            max_requests: 30,
            budget_window_ms: 60_000,
        }
    }
}

/// `[security.approval]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ApprovalSettings {
    /// Seconds an interactive approval stays parked before it expires as
    /// denied. `None` keeps the built-in ten minutes.
    pub ttl_secs: Option<u64>,
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
#[path = "security_tests.rs"]
mod tests;
