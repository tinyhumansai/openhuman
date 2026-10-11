//! TokenJuice content-router configuration (`[tokenjuice]`).
//!
//! Controls the TinyJuice content-aware tool-output compaction engine: which
//! compressors are enabled and the Compress-Cache-Retrieve (CCR) store limits.
//! Plain text uses TinyJuice’s deterministic TextCrusher. The host applies these settings
//! before module calls via [`crate::inference::tokenjuice::install_from_config`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TokenjuiceConfig {
    /// Master switch for the content router. When `false`, tool output passes
    /// through uncompacted. On by default; it only acts while
    /// `ContextConfig::compaction_enabled` is also on.
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
    /// Store a large result behind a handle and show the model a stats line, a
    /// short head and the handle, instead of one compressed blob. The model then
    /// queries the stored original with `juice_find`, `juice_extract` and
    /// `juice_summarize`, which are registered only while this is in effect
    /// (see `crate::inference::tokenjuice::repl_handle_active`). Needs
    /// `router_enabled`, `ccr_enabled` and `context.compaction_enabled`. In this
    /// mode the LLM summary stage is skipped for results big enough to get a
    /// handle. Turn off to keep the one-blob compression and the
    /// `juice_retrieve` round trip.
    #[serde(default = "default_true")]
    pub repl_handle_enabled: bool,
    /// Also write each stored original to
    /// `<workspace>/.tokenjuice/repl/<handle>.txt` (mode 0600) so the agent can
    /// script over it. Off by default: it puts raw tool output on disk, and
    /// nothing prunes the directory.
    #[serde(default)]
    pub repl_save_enabled: bool,
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

impl Default for TokenjuiceConfig {
    fn default() -> Self {
        Self {
            router_enabled: true,
            ccr_enabled: true,
            repl_handle_enabled: true,
            repl_save_enabled: false,
            ccr_disk_enabled: false,
            max_cache_entries: default_max_cache_entries(),
            max_cache_bytes: default_max_cache_bytes(),
            ccr_ttl_secs: None,
            min_bytes_to_compress: default_min_bytes(),
            ccr_min_tokens: default_ccr_min_tokens(),
            search_enabled: true,
            code_enabled: true,
            html_enabled: true,
        }
    }
}

#[cfg(test)]
#[path = "tokenjuice_tests.rs"]
mod tests;
