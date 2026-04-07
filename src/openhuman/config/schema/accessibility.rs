use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScreenIntelligenceConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_capture_policy")]
    pub capture_policy: String,
    #[serde(default = "default_policy_mode")]
    pub policy_mode: String,
    #[serde(default = "default_baseline_fps")]
    pub baseline_fps: f32,
    #[serde(default = "default_vision_enabled")]
    pub vision_enabled: bool,
    #[serde(default = "default_session_ttl_secs")]
    pub session_ttl_secs: u64,
    #[serde(default = "default_panic_stop_hotkey")]
    pub panic_stop_hotkey: String,
    #[serde(default = "default_autocomplete_enabled")]
    pub autocomplete_enabled: bool,
    /// When `true`, captured screenshots are saved to `{workspace_dir}/screenshots/`
    /// instead of being discarded after vision processing. Default: `false`.
    #[serde(default)]
    pub keep_screenshots: bool,
    #[serde(default)]
    pub allowlist: Vec<String>,
    #[serde(default)]
    pub denylist: Vec<String>,
}

fn default_enabled() -> bool {
    false
}

fn default_capture_policy() -> String {
    "hybrid".to_string()
}

fn default_policy_mode() -> String {
    "all_except_blacklist".to_string()
}

fn default_baseline_fps() -> f32 {
    0.2
}

fn default_vision_enabled() -> bool {
    true
}

fn default_session_ttl_secs() -> u64 {
    300
}

fn default_panic_stop_hotkey() -> String {
    "Cmd+Shift+.".to_string()
}

fn default_autocomplete_enabled() -> bool {
    true
}

impl Default for ScreenIntelligenceConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            capture_policy: default_capture_policy(),
            policy_mode: default_policy_mode(),
            baseline_fps: default_baseline_fps(),
            vision_enabled: default_vision_enabled(),
            session_ttl_secs: default_session_ttl_secs(),
            panic_stop_hotkey: default_panic_stop_hotkey(),
            autocomplete_enabled: default_autocomplete_enabled(),
            keep_screenshots: false,
            allowlist: vec![],
            denylist: vec![
                "1password".to_string(),
                "keychain".to_string(),
                "wallet".to_string(),
                "seed phrase".to_string(),
            ],
        }
    }
}
