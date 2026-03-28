use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AccessibilityAutomationConfig {
    #[serde(default = "default_capture_policy")]
    pub capture_policy: String,
    #[serde(default = "default_baseline_fps")]
    pub baseline_fps: f32,
    #[serde(default = "default_session_ttl_secs")]
    pub session_ttl_secs: u64,
    #[serde(default = "default_panic_stop_hotkey")]
    pub panic_stop_hotkey: String,
    #[serde(default = "default_autocomplete_enabled")]
    pub autocomplete_enabled: bool,
    #[serde(default)]
    pub denylist: Vec<String>,
}

fn default_capture_policy() -> String {
    "hybrid".to_string()
}

fn default_baseline_fps() -> f32 {
    1.0
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

impl Default for AccessibilityAutomationConfig {
    fn default() -> Self {
        Self {
            capture_policy: default_capture_policy(),
            baseline_fps: default_baseline_fps(),
            session_ttl_secs: default_session_ttl_secs(),
            panic_stop_hotkey: default_panic_stop_hotkey(),
            autocomplete_enabled: default_autocomplete_enabled(),
            denylist: vec![
                "1password".to_string(),
                "keychain".to_string(),
                "wallet".to_string(),
                "seed phrase".to_string(),
            ],
        }
    }
}
