//! Local AI runtime configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LocalAiConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_model_id")]
    pub model_id: String,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub checksum_sha256: Option<String>,
    #[serde(default = "default_artifact_name")]
    pub artifact_name: String,
    #[serde(default = "default_autosummary_debounce_ms")]
    pub autosummary_debounce_ms: u64,
    #[serde(default = "default_context_compaction_threshold_tokens")]
    pub context_compaction_threshold_tokens: usize,
    #[serde(default = "default_max_suggestions")]
    pub max_suggestions: usize,
}

fn default_enabled() -> bool {
    true
}

fn default_provider() -> String {
    "mistralrs".to_string()
}

fn default_model_id() -> String {
    "qwen3.5-1b".to_string()
}

fn default_artifact_name() -> String {
    "qwen3.5-1b.gguf".to_string()
}

fn default_autosummary_debounce_ms() -> u64 {
    2500
}

fn default_context_compaction_threshold_tokens() -> usize {
    100_000
}

fn default_max_suggestions() -> usize {
    5
}

impl Default for LocalAiConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            provider: default_provider(),
            model_id: default_model_id(),
            download_url: None,
            checksum_sha256: None,
            artifact_name: default_artifact_name(),
            autosummary_debounce_ms: default_autosummary_debounce_ms(),
            context_compaction_threshold_tokens: default_context_compaction_threshold_tokens(),
            max_suggestions: default_max_suggestions(),
        }
    }
}
