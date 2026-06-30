//! TokenJuice content-router configuration (`[tokenjuice]`).
//!
//! Controls the content-aware tool-output compaction engine: which compressors
//! are enabled, the Compress-Cache-Retrieve (CCR) store limits, and the opt-in
//! Python/ML plain-text compressor. Installed into the runtime at startup via
//! [`crate::openhuman::tokenjuice::configure`] + the CCR cache `configure`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TokenjuiceConfig {
    /// Master switch for the content router. When `false`, tool output passes
    /// through uncompacted.
    #[serde(default = "default_true")]
    pub router_enabled: bool,
    /// Whether lossy compressions offload the original to the CCR store and emit
    /// a `⟦tj:<hash>⟧` retrieval footer. Disabling makes compaction one-way.
    #[serde(default = "default_true")]
    pub ccr_enabled: bool,
    /// Persist CCR originals to disk under `<workspace>/.tokenjuice/ccr` so
    /// retrieval survives memory eviction (written by the core only).
    #[serde(default)]
    pub ccr_disk_enabled: bool,
    /// Max number of originals retained in the in-memory CCR store.
    #[serde(default = "default_max_cache_entries")]
    pub max_cache_entries: usize,
    /// Max total bytes retained in the in-memory CCR store.
    #[serde(default = "default_max_cache_bytes")]
    pub max_cache_bytes: usize,
    /// Optional TTL (seconds) for CCR entries; `None` ⇒ no expiry.
    #[serde(default)]
    pub ccr_ttl_secs: Option<u64>,
    /// Minimum output size (bytes) before compaction is attempted.
    #[serde(default = "default_min_bytes")]
    pub min_bytes_to_compress: usize,
    /// CCR only fires (original offloaded + lossy compaction) when the tool
    /// result is estimated at ≥ this many tokens. Smaller results pass through.
    #[serde(default = "default_ccr_min_tokens")]
    pub ccr_min_tokens: usize,
    /// Enable the search-results (grep) relevance compressor.
    #[serde(default = "default_true")]
    pub search_enabled: bool,
    /// Enable the AST/heuristic code compressor.
    #[serde(default = "default_true")]
    pub code_enabled: bool,
    /// Enable the HTML→text extractor.
    #[serde(default = "default_true")]
    pub html_enabled: bool,

    // --- ML plain-text compressor (Kompress) — opt-in, default OFF ---------
    /// Enable the Python/ML plain-text compressor ("Kompress"). Runs as a
    /// `kompress` backend of the runtime_python_server (requires
    /// `runtime_python.enabled`); degrades gracefully when unavailable.
    #[serde(default)]
    pub ml_compression_enabled: bool,
    /// HuggingFace model id for the ML compressor.
    #[serde(default = "default_ml_model_id")]
    pub ml_model_id: String,
    /// Target compression ratio (0–1) hint for the ML compressor.
    #[serde(default = "default_ml_target_ratio")]
    pub ml_target_ratio: f64,
    /// Idle seconds before the ML sidecar process is reaped to release memory.
    #[serde(default = "default_ml_idle_timeout_secs")]
    pub ml_sidecar_idle_timeout_secs: u64,
    /// Maximum input characters the ML compressor will accept (larger inputs
    /// fall back to a native compressor).
    #[serde(default = "default_ml_max_input_chars")]
    pub ml_max_input_chars: usize,
    /// Inference device: `cpu` or `auto`.
    #[serde(default = "default_ml_device")]
    pub ml_device: String,
}

fn default_true() -> bool {
    true
}
fn default_max_cache_entries() -> usize {
    256
}
fn default_max_cache_bytes() -> usize {
    64 * 1024 * 1024
}
fn default_min_bytes() -> usize {
    2048
}
fn default_ccr_min_tokens() -> usize {
    500
}
fn default_ml_model_id() -> String {
    "answerdotai/ModernBERT-base".to_string()
}
fn default_ml_target_ratio() -> f64 {
    0.5
}
fn default_ml_idle_timeout_secs() -> u64 {
    900
}
fn default_ml_max_input_chars() -> usize {
    200_000
}
fn default_ml_device() -> String {
    "cpu".to_string()
}

impl Default for TokenjuiceConfig {
    fn default() -> Self {
        Self {
            router_enabled: true,
            ccr_enabled: true,
            ccr_disk_enabled: false,
            max_cache_entries: default_max_cache_entries(),
            max_cache_bytes: default_max_cache_bytes(),
            ccr_ttl_secs: None,
            min_bytes_to_compress: default_min_bytes(),
            ccr_min_tokens: default_ccr_min_tokens(),
            search_enabled: true,
            code_enabled: true,
            html_enabled: true,
            ml_compression_enabled: false,
            ml_model_id: default_ml_model_id(),
            ml_target_ratio: default_ml_target_ratio(),
            ml_sidecar_idle_timeout_secs: default_ml_idle_timeout_secs(),
            ml_max_input_chars: default_ml_max_input_chars(),
            ml_device: default_ml_device(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = TokenjuiceConfig::default();
        assert!(c.router_enabled);
        assert!(c.ccr_enabled);
        assert!(c.search_enabled);
        assert!(!c.ml_compression_enabled);
        assert_eq!(c.ml_device, "cpu");
    }

    #[test]
    fn parses_from_toml() {
        let c: TokenjuiceConfig = toml::from_str(
            r#"
            router_enabled = false
            ml_compression_enabled = true
            max_cache_entries = 12
            ccr_ttl_secs = 300
            "#,
        )
        .unwrap();
        assert!(!c.router_enabled);
        assert!(c.ml_compression_enabled);
        assert_eq!(c.max_cache_entries, 12);
        assert_eq!(c.ccr_ttl_secs, Some(300));
        // Untouched fields keep defaults.
        assert!(c.code_enabled);
    }
}
