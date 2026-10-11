//! Serde types exchanged with the frontend: the persisted local app-state
//! blob, the patch applied to it, and the polled `app_state_snapshot` shape.

use crate::platform::service::ServiceStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tinyinference_local::status::LocalAiStatus;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoredOnboardingTasks {
    #[serde(default)]
    pub accessibility_permission_granted: bool,
    // `localModelConsentGiven` / `localModelDownloadStarted` were dropped with
    // local model downloads. Older app-state files still carry them; serde
    // ignores unknown fields here (no `deny_unknown_fields`), so those files
    // keep loading and the keys are dropped on the next save.
    #[serde(default)]
    pub enabled_tools: Vec<String>,
    #[serde(default)]
    pub connected_sources: Vec<String>,
    #[serde(default)]
    pub updated_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoredAppState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding_tasks: Option<StoredOnboardingTasks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyring_consent: Option<crate::security::keyring_consent::ConsentPreference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateSnapshot {
    pub auth: crate::security::credentials::responses::AuthStateResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_user: Option<Value>,
    pub onboarding_completed: bool,
    /// Deprecated — the welcome agent has been removed. Retained in the
    /// snapshot for backward compatibility with frontend code that still
    /// reads it. This value may be `false` in newer configs; routing no
    /// longer depends on this field.
    pub chat_onboarding_completed: bool,
    pub analytics_enabled: bool,
    pub local_state: StoredAppState,
    pub keyring_status: crate::security::keyring_consent::KeyringStatus,
    pub runtime: RuntimeSnapshot,
    /// Process + component health, folded into this snapshot so the frontend
    /// hydrates the daemon-health store from the same poll instead of running a
    /// second `health_snapshot` poller. Fields stay snake_case (the type has no
    /// camelCase rename) to match the frontend's existing health parser.
    pub health: crate::platform::health::HealthSnapshot,
    /// `true` when this session's config loader had to recover a corrupted
    /// `config.toml` (renamed to `.corrupted.<ts>` and reset to defaults / a
    /// backup). Latched at boot so it stays reported even after the file is
    /// healed; the frontend raises a one-shot "settings were reset" notice
    /// (#5167). Serialized as `configRecovered`.
    pub config_recovered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub local_ai: LocalAiStatus,
    pub service: ServiceStatus,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoredAppStatePatch {
    #[serde(default)]
    pub encryption_key: Option<Option<String>>,
    #[serde(default)]
    pub onboarding_tasks: Option<Option<StoredOnboardingTasks>>,
    #[serde(default)]
    pub keyring_consent: Option<Option<crate::security::keyring_consent::ConsentPreference>>,
}
