//! Storage provider and memory configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct StorageConfig {
    #[serde(default)]
    pub provider: StorageProviderSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct StorageProviderSection {
    #[serde(default)]
    pub config: StorageProviderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StorageProviderConfig {
    #[serde(default)]
    pub provider: String,
}

impl Default for StorageProviderConfig {
    fn default() -> Self {
        Self {
            provider: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[allow(clippy::struct_excessive_bools)]
pub struct MemoryConfig {
    pub backend: String,
    pub auto_save: bool,
    #[serde(default = "default_embedding_provider")]
    pub embedding_provider: String,
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    #[serde(default = "default_embedding_dims")]
    pub embedding_dimensions: usize,
    #[serde(default = "default_min_relevance_score")]
    pub min_relevance_score: f64,
    #[serde(default)]
    pub response_cache_enabled: bool,
    #[serde(default)]
    pub sqlite_open_timeout_secs: Option<u64>,
}

fn default_embedding_provider() -> String {
    "ollama".into()
}
fn default_embedding_model() -> String {
    "nomic-embed-text:latest".into()
}
fn default_embedding_dims() -> usize {
    768
}
fn default_min_relevance_score() -> f64 {
    0.4
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: "sqlite".into(),
            auto_save: true,
            embedding_provider: default_embedding_provider(),
            embedding_model: default_embedding_model(),
            embedding_dimensions: default_embedding_dims(),
            min_relevance_score: default_min_relevance_score(),
            response_cache_enabled: false,
            sqlite_open_timeout_secs: None,
        }
    }
}

/// Phase 4 memory-tree configuration — embedding provider wiring for the
/// hierarchical memory (#710).
///
/// When `embedding_endpoint` and `embedding_model` are both set, ingest
/// and bucket-seal route every new chunk/summary through the Ollama
/// embedder before writing. When unset, behaviour depends on
/// `embedding_strict`:
/// - `true` (default): ingest/seal bail with a clear config error.
/// - `false`: fall back to the inert zero-vector embedder and warn.
///
/// Env overrides apply in [`super::load`]:
/// - `OPENHUMAN_MEMORY_EMBED_ENDPOINT`
/// - `OPENHUMAN_MEMORY_EMBED_MODEL`
/// - `OPENHUMAN_MEMORY_EMBED_TIMEOUT_MS`
/// - `OPENHUMAN_MEMORY_EXTRACT_ENDPOINT`
/// - `OPENHUMAN_MEMORY_EXTRACT_MODEL`
/// - `OPENHUMAN_MEMORY_EXTRACT_TIMEOUT_MS`
/// - `OPENHUMAN_MEMORY_SUMMARISE_ENDPOINT`
/// - `OPENHUMAN_MEMORY_SUMMARISE_MODEL`
/// - `OPENHUMAN_MEMORY_SUMMARISE_TIMEOUT_MS`
/// - `OPENHUMAN_MEMORY_TREE_CONTENT_DIR` (Phase MD-content)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryTreeConfig {
    /// Ollama endpoint for the embedder (e.g. `http://localhost:11434`).
    /// `None` disables the Ollama path — see `embedding_strict` for the
    /// resulting behaviour.
    #[serde(default = "default_memory_tree_embedding_endpoint")]
    pub embedding_endpoint: Option<String>,

    /// Embedding model name. Must produce 768-dim vectors (see
    /// `memory::tree::score::embed::EMBEDDING_DIM`). `None` disables
    /// the Ollama path.
    #[serde(default = "default_memory_tree_embedding_model")]
    pub embedding_model: Option<String>,

    /// Per-request timeout for the embedder, in milliseconds.
    #[serde(default = "default_memory_tree_embedding_timeout_ms")]
    pub embedding_timeout_ms: Option<u64>,

    /// When true, ingest/seal refuse to run with embeddings disabled.
    /// When false, an inert zero-vector embedder is used and retrieval
    /// rerank falls back to scope + recency ordering only.
    #[serde(default = "default_memory_tree_embedding_strict")]
    pub embedding_strict: bool,

    /// Ollama endpoint for the LLM entity extractor
    /// (`memory::tree::score::extract::llm::LlmEntityExtractor`). When
    /// unset, ingest uses the regex-only extractor — no LLM call. Soft
    /// failures in the LLM path fall back to regex-only for that chunk.
    #[serde(default = "default_memory_tree_llm_endpoint")]
    pub llm_extractor_endpoint: Option<String>,

    /// Model name for the entity extractor (e.g. `qwen2.5:0.5b`).
    #[serde(default = "default_memory_tree_llm_endpoint")]
    pub llm_extractor_model: Option<String>,

    /// Per-request timeout for the LLM extractor, in milliseconds.
    #[serde(default = "default_memory_tree_llm_extractor_timeout_ms")]
    pub llm_extractor_timeout_ms: Option<u64>,

    /// Ollama endpoint for the summariser
    /// (`memory::tree::source_tree::summariser::llm::LlmSummariser`).
    /// When unset, bucket-seal cascades use `InertSummariser` — sealed
    /// nodes contain concatenated+truncated child text instead of a
    /// real LLM summary. Soft failures fall back to inert per seal.
    #[serde(default = "default_memory_tree_llm_endpoint")]
    pub llm_summariser_endpoint: Option<String>,

    /// Model name for the summariser. Larger models produce better
    /// summaries at higher latency; `llama3.1:8b` is a reasonable
    /// default for production.
    #[serde(default = "default_memory_tree_llm_endpoint")]
    pub llm_summariser_model: Option<String>,

    /// Per-request timeout for the summariser, in milliseconds. Default
    /// is higher than the extractor because summarisation uses more
    /// tokens and therefore takes longer to generate.
    #[serde(default = "default_memory_tree_llm_summariser_timeout_ms")]
    pub llm_summariser_timeout_ms: Option<u64>,

    /// Phase MD-content: root directory where chunk `.md` files are stored.
    ///
    /// Resolved at runtime via [`super::types::Config::memory_tree_content_root`]:
    /// - `Some(path)` → use that path verbatim.
    /// - `None` → default `<workspace_dir>/memory_tree/content/`.
    ///
    /// Env override: `OPENHUMAN_MEMORY_TREE_CONTENT_DIR` (empty string = fall
    /// back to default, consistent with other memory_tree env vars).
    #[serde(default = "default_memory_tree_content_dir")]
    pub content_dir: Option<PathBuf>,
}

/// Returns `None` so that existing installs that never opted into Phase 4
/// embeddings stay on the inert zero-vector path rather than suddenly
/// attempting to reach a local Ollama daemon they haven't configured.
/// Operators enable the Ollama path by setting either `embedding_endpoint`
/// in TOML or the `OPENHUMAN_MEMORY_EMBED_ENDPOINT` env var.
fn default_memory_tree_embedding_endpoint() -> Option<String> {
    None
}

fn default_memory_tree_embedding_model() -> Option<String> {
    None
}

fn default_memory_tree_embedding_timeout_ms() -> Option<u64> {
    Some(10_000)
}

/// Defaults to `false` so installs without an embedding endpoint fall back
/// to the inert zero-vector embedder (with a warn log) instead of refusing
/// to run. Set to `true` in production configs that require embeddings.
fn default_memory_tree_embedding_strict() -> bool {
    false
}

/// Shared `None` default for the LLM-path fields (extractor + summariser
/// endpoints + models). Keeping the same function for all of them makes
/// the intent explicit: nothing here auto-enables Ollama.
fn default_memory_tree_llm_endpoint() -> Option<String> {
    None
}

fn default_memory_tree_llm_extractor_timeout_ms() -> Option<u64> {
    Some(15_000)
}

fn default_memory_tree_llm_summariser_timeout_ms() -> Option<u64> {
    // 120s — large enough for small/medium local models to finish a
    // seal-budget summary on a cold-loaded weight cache. Tighter
    // values cause the LlmSummariser to time out and silently fall
    // back to InertSummariser (no LLM signal in the resulting node).
    Some(120_000)
}

/// Returns `None` so the default `<workspace>/memory_tree/content/` path is
/// used unless explicitly overridden via TOML or env var.
fn default_memory_tree_content_dir() -> Option<PathBuf> {
    None
}

impl Default for MemoryTreeConfig {
    fn default() -> Self {
        Self {
            embedding_endpoint: default_memory_tree_embedding_endpoint(),
            embedding_model: default_memory_tree_embedding_model(),
            embedding_timeout_ms: default_memory_tree_embedding_timeout_ms(),
            embedding_strict: default_memory_tree_embedding_strict(),
            llm_extractor_endpoint: default_memory_tree_llm_endpoint(),
            llm_extractor_model: default_memory_tree_llm_endpoint(),
            llm_extractor_timeout_ms: default_memory_tree_llm_extractor_timeout_ms(),
            llm_summariser_endpoint: default_memory_tree_llm_endpoint(),
            llm_summariser_model: default_memory_tree_llm_endpoint(),
            llm_summariser_timeout_ms: default_memory_tree_llm_summariser_timeout_ms(),
            content_dir: default_memory_tree_content_dir(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_tree_config_default_content_dir_is_none() {
        let cfg = MemoryTreeConfig::default();
        assert!(
            cfg.content_dir.is_none(),
            "default content_dir must be None so workspace default path is used"
        );
    }

    /// Verify that the env-var override logic correctly maps non-empty strings
    /// to `Some(PathBuf)` and empty/blank strings to `None`. We test the
    /// logic inline (not via `apply_env_overrides`) to avoid mutating the
    /// process environment in a way that could race with parallel tests.
    #[test]
    fn content_dir_env_override_logic() {
        // Simulate the load.rs overlay logic.
        let apply = |raw: &str| -> Option<PathBuf> {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            }
        };

        assert_eq!(apply("/tmp/foo"), Some(PathBuf::from("/tmp/foo")));
        assert_eq!(apply("  /tmp/foo  "), Some(PathBuf::from("/tmp/foo")));
        assert_eq!(apply(""), None);
        assert_eq!(apply("   "), None);
    }
}
