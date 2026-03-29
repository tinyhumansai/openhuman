use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutocompleteConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,
    #[serde(default = "default_style_preset")]
    pub style_preset: String,
    #[serde(default)]
    pub style_instructions: Option<String>,
    #[serde(default)]
    pub style_examples: Vec<String>,
    #[serde(default)]
    pub disabled_apps: Vec<String>,
    #[serde(default = "default_accept_with_tab")]
    pub accept_with_tab: bool,
}

fn default_enabled() -> bool {
    true
}

fn default_debounce_ms() -> u64 {
    120
}

fn default_max_chars() -> usize {
    384
}

fn default_style_preset() -> String {
    "balanced".to_string()
}

fn default_accept_with_tab() -> bool {
    true
}

impl Default for AutocompleteConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            debounce_ms: default_debounce_ms(),
            max_chars: default_max_chars(),
            style_preset: default_style_preset(),
            style_instructions: None,
            style_examples: Vec::new(),
            disabled_apps: vec![],
            accept_with_tab: default_accept_with_tab(),
        }
    }
}
