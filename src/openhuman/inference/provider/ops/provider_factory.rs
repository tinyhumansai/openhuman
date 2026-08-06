use std::path::PathBuf;

/// Fixed id for the single inference backend (OpenHuman API).
pub const INFERENCE_BACKEND_ID: &str = "openhuman";

#[derive(Debug, Clone)]
pub struct ProviderRuntimeOptions {
    pub auth_profile_override: Option<String>,
    pub openhuman_dir: Option<PathBuf>,
    pub secrets_encrypt: bool,
    pub reasoning_enabled: Option<bool>,
}

impl Default for ProviderRuntimeOptions {
    fn default() -> Self {
        Self {
            auth_profile_override: None,
            openhuman_dir: None,
            secrets_encrypt: true,
            reasoning_enabled: None,
        }
    }
}

/// Information about a supported provider for display purposes.
pub struct ProviderInfo {
    pub name: &'static str,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub local: bool,
}

/// Return known providers for display (single backend path).
pub fn list_providers() -> Vec<ProviderInfo> {
    vec![ProviderInfo {
        name: INFERENCE_BACKEND_ID,
        display_name: "OpenHuman (backend)",
        aliases: &["backend", "openhuman-backend"],
        local: false,
    }]
}

// Legacy provider alias stubs (integrations / config); remote providers were removed.
pub fn is_glm_alias(_name: &str) -> bool {
    false
}
pub fn is_zai_alias(_name: &str) -> bool {
    false
}
pub fn is_minimax_alias(_name: &str) -> bool {
    false
}
pub fn is_moonshot_alias(_name: &str) -> bool {
    false
}
pub fn is_qianfan_alias(_name: &str) -> bool {
    false
}
pub fn is_qwen_alias(_name: &str) -> bool {
    false
}
pub fn is_qwen_oauth_alias(_name: &str) -> bool {
    false
}
pub fn canonical_china_provider_name(_name: &str) -> Option<&'static str> {
    let _ = _name;
    None
}
