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
            sqlite_open_timeout_secs: None,
        }
    }
}

/// Which inference backend the memory_tree's LLM calls (extractor +
/// summariser) should use.
///
/// - `Cloud` (default): route through `providers::router` against the
///   OpenHuman backend with the `summarization-v1` model. No local Ollama
///   required.
/// - `Local`: keep using the legacy Ollama-direct path (the
///   `llm_extractor_endpoint` / `llm_summariser_endpoint` config). Useful
///   for offline development and CI smoke tests.
///
/// Embedder selection is unchanged — `OllamaEmbedder` (bge-m3) stays
/// local-only and isn't governed by this enum.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LlmBackend {
    /// Route through the OpenHuman backend (default).
    Cloud,
    /// Use the local Ollama path configured via `llm_extractor_*` /
    /// `llm_summariser_*`.
    Local,
}

impl LlmBackend {
    /// Stable wire string for env vars / RPCs / logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cloud => "cloud",
            Self::Local => "local",
        }
    }

    /// Inverse of [`Self::as_str`]; case-insensitive parse.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cloud" => Ok(Self::Cloud),
            "local" => Ok(Self::Local),
            other => Err(format!("unknown llm (expected cloud|local): {other}")),
        }
    }
}

impl Default for LlmBackend {
    fn default() -> Self {
        Self::Cloud
    }
}

fn default_llm_backend() -> LlmBackend {
    LlmBackend::default()
}

/// Default model identifier to use when `llm_backend = "cloud"`. Routed
/// through the OpenHuman backend; keep in sync with the backend's
/// summariser model registry.
pub const DEFAULT_CLOUD_LLM_MODEL: &str = "summarization-v1";

fn default_cloud_llm_model() -> Option<String> {
    Some(DEFAULT_CLOUD_LLM_MODEL.to_string())
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
/// - `OPENHUMAN_MEMORY_TREE_LLM_BACKEND` (cloud|local)
/// - `OPENHUMAN_MEMORY_TREE_CLOUD_LLM_MODEL`
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
    /// (`memory::tree::score::extract::llm::LlmEntityExtractor`).
    /// Defaults to `Some("http://localhost:11434")` — the standard
    /// Ollama listener — see [`default_memory_tree_llm_endpoint`].
    /// Soft failures in the LLM path fall back to regex-only for
    /// that chunk.
    #[serde(default = "default_memory_tree_llm_endpoint")]
    pub llm_extractor_endpoint: Option<String>,

    /// Model name for the entity extractor. Defaults to `gemma3:4b`
    /// (see [`default_memory_tree_llm_model`] for the rationale);
    /// override to a smaller model on resource-constrained hosts.
    #[serde(default = "default_memory_tree_llm_model")]
    pub llm_extractor_model: Option<String>,

    /// Per-request timeout for the LLM extractor, in milliseconds.
    #[serde(default = "default_memory_tree_llm_extractor_timeout_ms")]
    pub llm_extractor_timeout_ms: Option<u64>,

    /// Ollama endpoint for the summariser
    /// (`memory::tree::tree_source::summariser::llm::LlmSummariser`).
    /// Defaults to `Some("http://localhost:11434")` — see
    /// [`default_memory_tree_llm_endpoint`]. Soft failures fall back
    /// to `InertSummariser` per seal.
    #[serde(default = "default_memory_tree_llm_endpoint")]
    pub llm_summariser_endpoint: Option<String>,

    /// Model name for the summariser. Defaults to `gemma3:4b` —
    /// larger Gemma tiers (`gemma3:12b-it-qat`, `gemma3:27b`) produce
    /// more coherent abstractive summaries at higher latency. See
    /// [`default_memory_tree_llm_model`].
    #[serde(default = "default_memory_tree_llm_model")]
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

    /// Backend selector for the memory_tree's LLM calls (extractor +
    /// summariser). Defaults to [`LlmBackend::Cloud`] so a fresh install
    /// works without requiring a local Ollama daemon. Set to
    /// [`LlmBackend::Local`] (or `OPENHUMAN_MEMORY_TREE_LLM_BACKEND=local`) to
    /// keep the legacy Ollama-direct path.
    ///
    /// The embedder is unaffected by this setting — `OllamaEmbedder` (bge-m3)
    /// stays local-only.
    #[serde(default = "default_llm_backend")]
    pub llm_backend: LlmBackend,

    /// Model identifier used when `llm_backend = "cloud"`. Routed through the
    /// OpenHuman backend's chat-completions surface.
    ///
    /// Defaults to [`DEFAULT_CLOUD_LLM_MODEL`] (`summarization-v1`).
    /// Env override: `OPENHUMAN_MEMORY_TREE_CLOUD_LLM_MODEL`.
    #[serde(default = "default_cloud_llm_model")]
    pub cloud_llm_model: Option<String>,
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
/// the intent explicit.
///
/// Default points at the standard Ollama localhost listener. A user
/// who sets `llm_backend = "local"` plus a `_model` is clearly opting
/// into Ollama, and forcing them to also specify the endpoint just to
/// hit `localhost:11434` was a stealth foot-gun: the
/// `OllamaChatProvider` returned an error on an empty endpoint, which
/// the summariser silently swallowed into its `InertSummariser`
/// fallback — producing concat-and-truncate "summaries" that looked
/// correct but didn't run any LLM at all. With a default endpoint in
/// place, the only signal needed to enable a local LLM seal is a
/// non-empty `_model`. Override via TOML or
/// `OPENHUMAN_MEMORY_TREE_LLM_*_ENDPOINT` to point at a different
/// Ollama host.
fn default_memory_tree_llm_endpoint() -> Option<String> {
    Some("http://localhost:11434".to_string())
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

/// Default Ollama model for the memory-tree LLMs (extractor + summariser).
///
/// `gemma3:4b` is in the Gemma 3 family (Gemma 4 isn't released yet)
/// and sits between the 1B compact tier and the 12B/27B large tiers.
/// At ~3 GB on disk and ~8 GB RAM at inference it stays inside the
/// envelope of a typical laptop and produces coherent abstractive
/// summaries on real Gmail inboxes — smaller models (≤1.5B) regress
/// to "the email says X, the email says Y" enumeration that's barely
/// better than the InertSummariser concat fallback.
///
/// Override via `memory_tree.llm_summariser_model` /
/// `llm_extractor_model` in TOML (or `OPENHUMAN_MEMORY_TREE_LLM_*_MODEL`
/// env vars) to scale up (`gemma3:12b-it-qat`, `llama3.1:8b`) or down
/// (`gemma3:1b-it-qat`) for the host's headroom. The frontend
/// `ModelCatalog` lists the curated picks the UI offers as
/// downloadable presets.
fn default_memory_tree_llm_model() -> Option<String> {
    Some("gemma3:4b".to_string())
}

impl Default for MemoryTreeConfig {
    fn default() -> Self {
        Self {
            embedding_endpoint: default_memory_tree_embedding_endpoint(),
            embedding_model: default_memory_tree_embedding_model(),
            embedding_timeout_ms: default_memory_tree_embedding_timeout_ms(),
            embedding_strict: default_memory_tree_embedding_strict(),
            llm_extractor_endpoint: default_memory_tree_llm_endpoint(),
            llm_extractor_model: default_memory_tree_llm_model(),
            llm_extractor_timeout_ms: default_memory_tree_llm_extractor_timeout_ms(),
            llm_summariser_endpoint: default_memory_tree_llm_endpoint(),
            llm_summariser_model: default_memory_tree_llm_model(),
            llm_summariser_timeout_ms: default_memory_tree_llm_summariser_timeout_ms(),
            content_dir: default_memory_tree_content_dir(),
            llm_backend: default_llm_backend(),
            cloud_llm_model: default_cloud_llm_model(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_default_is_cloud() {
        assert_eq!(LlmBackend::default(), LlmBackend::Cloud);
        assert_eq!(MemoryTreeConfig::default().llm_backend, LlmBackend::Cloud);
    }

    #[test]
    fn llm_round_trip() {
        for v in [LlmBackend::Cloud, LlmBackend::Local] {
            assert_eq!(LlmBackend::parse(v.as_str()).unwrap(), v);
        }
    }

    #[test]
    fn llm_parse_is_case_insensitive() {
        assert_eq!(LlmBackend::parse("CLOUD").unwrap(), LlmBackend::Cloud);
        assert_eq!(LlmBackend::parse(" Local ").unwrap(), LlmBackend::Local);
    }

    #[test]
    fn llm_parse_rejects_unknown() {
        assert!(LlmBackend::parse("hybrid").is_err());
        assert!(LlmBackend::parse("").is_err());
    }

    #[test]
    fn cloud_llm_model_default_is_summarizer_v1() {
        let cfg = MemoryTreeConfig::default();
        assert_eq!(
            cfg.cloud_llm_model.as_deref(),
            Some(DEFAULT_CLOUD_LLM_MODEL)
        );
        assert_eq!(DEFAULT_CLOUD_LLM_MODEL, "summarization-v1");
    }

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
