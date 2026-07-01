//! Serde domain types for the first-run readiness aggregator (issue #4252).
//!
//! These types are transport-facing (returned over JSON-RPC and rendered by
//! the onboarding "Readiness" step) and are deliberately decoupled from the
//! underlying probe sources (`health`, `memory`, `accessibility`, Ollama,
//! inference). [`ReadinessInputs`] is the pure input to
//! [`crate::openhuman::readiness::ops::evaluate`] so the rollup/plain-language
//! logic can be unit-tested without live services.

use serde::{Deserialize, Serialize};

/// Outcome of a single readiness check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// Verified working.
    Ok,
    /// Non-blocking problem — setup can still complete (e.g. optional Ollama
    /// not installed).
    Warn,
    /// Blocking problem for a required capability.
    Fail,
    /// Not applicable on this platform / not evaluated. Never implies the
    /// capability is working (acceptance criterion 6).
    Skipped,
}

impl CheckStatus {
    /// Rank for rolling several statuses up into an overall status. Higher =
    /// worse. `Skipped` is treated as neutral (does not worsen the rollup).
    fn severity(self) -> u8 {
        match self {
            CheckStatus::Ok => 0,
            CheckStatus::Skipped => 0,
            CheckStatus::Warn => 1,
            CheckStatus::Fail => 2,
        }
    }
}

/// A single plain-language readiness check surfaced in the setup flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessCheck {
    /// Stable machine id (`core_health`, `model_connection`, …).
    pub id: String,
    /// UI grouping bucket (`core`, `storage`, `permissions`, `model`, `local`).
    pub category: String,
    pub status: CheckStatus,
    /// Short human-readable title.
    pub title: String,
    /// One-line detail of what was observed.
    pub detail: String,
    /// Specific next-step recovery action when not `Ok`. Empty when `Ok`
    /// (acceptance criterion 5).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub remediation: String,
    /// Host OS this check evaluated against (`std::env::consts::OS`).
    pub platform: String,
    /// Whether a non-`Ok` result blocks a healthy overall rollup. Optional
    /// capabilities (Ollama) are `false`.
    pub required: bool,
}

/// Aggregated readiness report returned by `readiness.check_all`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessReport {
    pub checks: Vec<ReadinessCheck>,
    /// Worst rollup across all checks.
    pub overall: CheckStatus,
    /// Whether at least one model connection validated. The onboarding
    /// "Finish" action gates on this (acceptance criterion 3); the frontend
    /// may still offer a clearly-labeled non-silent override.
    pub model_connection_ok: bool,
    /// Host OS the core evaluated on.
    pub host_os: String,
}

/// Tri-state permission signal, mirrored from
/// [`crate::openhuman::accessibility::PermissionState`] but kept local so
/// [`evaluate`](crate::openhuman::readiness::ops::evaluate) stays pure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermState {
    Granted,
    Denied,
    Unknown,
    Unsupported,
}

/// Memory-storage / file-access probe inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageProbe {
    pub exists: bool,
    pub readable: bool,
    pub writable: bool,
    pub pipeline_healthy: bool,
    /// Whether the agent's action directory (`~/OpenHuman/projects`) is
    /// writable — the file-access signal (acceptance criterion 4).
    pub action_dir_writable: bool,
}

/// Desktop-automation permission probe inputs. On macOS these come from real
/// TCC checks; on other platforms UI Automation needs no OS grant so
/// `supported` is `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionProbe {
    /// Whether the host gates desktop automation behind an OS permission
    /// (macOS true; Windows/Linux false — no permission to verify).
    pub os_gated: bool,
    pub accessibility: PermState,
    pub screen_recording: PermState,
    pub input_monitoring: PermState,
}

/// Local Ollama probe inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OllamaProbe {
    pub reachable: bool,
    /// Installed model names discovered from `/api/tags` (acceptance
    /// criterion 2). Empty when unreachable or none installed.
    pub models: Vec<String>,
}

/// Model-connection probe inputs (the setup gate, acceptance criterion 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelConnProbe {
    /// Whether a configured model provider was validated as reachable + usable.
    pub ok: bool,
    /// Provider id that was validated (or attempted). Empty if none configured.
    pub provider_id: String,
    /// Failure detail when `ok` is false.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

/// Pure input bundle for [`evaluate`](crate::openhuman::readiness::ops::evaluate).
/// Populated by `gather_inputs` from live probes; hand-constructed in tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessInputs {
    pub host_os: String,
    /// Whether all critical core components report healthy.
    pub core_healthy: bool,
    /// Plain-language summary of core component state.
    pub core_detail: String,
    pub storage: StorageProbe,
    pub permissions: PermissionProbe,
    pub ollama: OllamaProbe,
    pub model_connection: ModelConnProbe,
}
