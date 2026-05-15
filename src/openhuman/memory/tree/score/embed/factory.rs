//! Build an [`Embedder`] from [`Config`] settings.
//!
//! Resolution order:
//! 1. **Explicit override** — `memory_tree.embedding_endpoint` +
//!    `memory_tree.embedding_model` both Some → [`OllamaEmbedder`] with
//!    those exact values. For power users / E2E test rigs that want to
//!    point at a non-default Ollama endpoint.
//! 2. **Local-AI usage flag** — `config.local_ai.use_local_for_embeddings()`
//!    (i.e. `runtime_enabled && usage.embeddings`) → [`OllamaEmbedder`]
//!    against [`ollama_base_url`] with the user's chosen
//!    `config.local_ai.embedding_model_id`. This is the path driven by
//!    the "Memory embeddings" checkbox in Local AI Settings.
//! 3. **Default** — [`CloudEmbedder`] (OpenHuman backend / Voyage,
//!    1024 dims). Auth failures surface at the first `embed()` call so
//!    ingest's existing retry-with-backoff logic handles them.
//!
//! NOTE on dimensions: the memory tree on-disk format is hard-coded at
//! [`EMBEDDING_DIM`](super::EMBEDDING_DIM) (1024). If the user picks a
//! local embedding model whose output is a different dimensionality,
//! the trait's post-call validator rejects each embed with a clear
//! `expected N dims, got M` error. Switching the local model picker in
//! Local AI Settings is the fix.
//!
//! The historical `InertEmbedder` (zero vectors) path is retained for
//! tests only — it is no longer the production lax-mode fallback.
//!
//! Env var overrides applied in [`crate::openhuman::config::load`]:
//! - `OPENHUMAN_MEMORY_EMBED_ENDPOINT`
//! - `OPENHUMAN_MEMORY_EMBED_MODEL`
//! - `OPENHUMAN_MEMORY_EMBED_TIMEOUT_MS`

use anyhow::Result;

use super::{CloudEmbedder, Embedder, InertEmbedder, OllamaEmbedder};
use crate::openhuman::config::Config;
use crate::openhuman::local_ai::ollama_base_url;

/// Cheap heuristic for "is a backend session reachable?" — the cloud
/// embedder needs one and bails on first embed call without it. We use
/// the *presence* of `auth-profiles.json` next to the config file as a
/// proxy: production after login has it, test harnesses and fresh
/// pre-login installs don't. The CloudEmbedder still re-validates the
/// JWT at every embed call, so a stale file just surfaces at embed
/// time (not factory build), preserving the prior failure behavior.
fn cloud_session_available(config: &Config) -> bool {
    config
        .config_path
        .parent()
        .map(|dir| dir.join("auth-profiles.json").exists())
        .unwrap_or(false)
}

/// Construct the active embedder for this process, honouring
/// `config.memory_tree.*` and `embedding_strict`.
///
/// Returns a boxed trait object so ingest / seal can call one code path
/// regardless of which provider is active. The returned box is created
/// per call — cheap because `OllamaEmbedder` owns a cloned `reqwest::Client`
/// internally and `InertEmbedder` is a ZST.
pub fn build_embedder_from_config(config: &Config) -> Result<Box<dyn Embedder>> {
    let tree_cfg = &config.memory_tree;
    match (
        tree_cfg.embedding_endpoint.as_deref(),
        tree_cfg.embedding_model.as_deref(),
    ) {
        (Some(endpoint), Some(model))
            if !endpoint.trim().is_empty() && !model.trim().is_empty() =>
        {
            let timeout_ms = tree_cfg.embedding_timeout_ms.unwrap_or(0);
            log::debug!(
                "[memory_tree::embed::factory] using Ollama endpoint={} model={} timeout_ms={}",
                endpoint,
                model,
                timeout_ms
            );
            Ok(Box::new(OllamaEmbedder::new(
                endpoint.to_string(),
                model.to_string(),
                timeout_ms,
            )))
        }
        _ => {
            // Honor the Local AI Settings "Memory embeddings" checkbox.
            // `use_local_for_embeddings()` is `runtime_enabled && usage.embeddings`
            // so we never route to a disabled local runtime.
            if config.local_ai.use_local_for_embeddings() {
                let model = config.local_ai.embedding_model_id.clone();
                let endpoint = ollama_base_url();
                let timeout_ms = tree_cfg.embedding_timeout_ms.unwrap_or(0);
                log::debug!(
                    "[memory_tree::embed::factory] usage.embeddings=true — using local Ollama endpoint={} model={} timeout_ms={}",
                    endpoint, model, timeout_ms
                );
                Ok(Box::new(OllamaEmbedder::new(endpoint, model, timeout_ms)))
            } else if cloud_session_available(config) {
                // Default for logged-in users: cloud (OpenHuman backend /
                // Voyage `voyage-3.5`, 1024 dims). Matches the main
                // embeddings path so a fresh install needs zero local
                // Ollama setup. JWT failures (expired, invalid, etc.)
                // surface as embed-call errors so ingest's existing
                // retry-with-backoff logic handles them.
                log::debug!(
                    "[memory_tree::embed::factory] using cloud (Voyage) — \
                     flip 'Memory embeddings' in Local AI Settings to switch to local"
                );
                Ok(Box::new(CloudEmbedder::new(config)))
            } else {
                // Pre-login, test harness, or unauthenticated runtime
                // path — no auth-profiles.json on disk means the cloud
                // path has no chance of resolving a bearer. Drop to
                // InertEmbedder (zero vectors) so ingest/seal/retrieval
                // can run without panic; semantic rerank degrades to
                // recency only until the user logs in (or until they
                // flip "Memory embeddings" to local with Ollama running).
                log::warn!(
                    "[memory_tree::embed::factory] no backend session found — \
                     using InertEmbedder (zero vectors). Log in to OpenHuman, or \
                     enable 'Memory embeddings' in Local AI Settings, to fix."
                );
                Ok(Box::new(InertEmbedder::new()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Plant config_path in the tempdir so cloud_session_available()
        // checks a writable directory; tests that need to simulate a
        // logged-in user just `touch` auth-profiles.json next to it.
        cfg.config_path = tmp.path().join("config.toml");
        (tmp, cfg)
    }

    /// Drop a stub `auth-profiles.json` next to the test config so
    /// `cloud_session_available()` returns true. Contents don't matter
    /// — the factory only checks presence.
    fn touch_auth_profile(cfg: &Config) {
        let path = cfg
            .config_path
            .parent()
            .map(|p| p.join("auth-profiles.json"))
            .expect("config_path has a parent");
        std::fs::write(&path, "{}").expect("write stub auth-profiles.json");
    }

    #[test]
    fn ollama_chosen_when_endpoint_and_model_set() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = Some("http://localhost:11434".into());
        cfg.memory_tree.embedding_model = Some("bge-m3".into());
        cfg.memory_tree.embedding_timeout_ms = Some(5000);
        let e = build_embedder_from_config(&cfg).expect("Ollama path should build");
        assert_eq!(e.name(), "ollama");
    }

    #[test]
    fn unset_endpoint_with_session_routes_to_cloud() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        touch_auth_profile(&cfg);
        let e = build_embedder_from_config(&cfg).expect("cloud default should build");
        assert_eq!(e.name(), "cloud");
    }

    #[test]
    fn unset_endpoint_without_session_falls_back_to_inert() {
        // Test harness / pre-login: no auth-profiles.json on disk,
        // factory degrades to InertEmbedder so callers don't crash on
        // first embed call.
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        let e = build_embedder_from_config(&cfg).expect("inert fallback should build");
        assert_eq!(e.name(), "inert");
    }

    #[test]
    fn empty_strings_count_as_unset_with_session() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = Some("".into());
        cfg.memory_tree.embedding_model = Some("".into());
        cfg.memory_tree.embedding_strict = false;
        touch_auth_profile(&cfg);
        let e = build_embedder_from_config(&cfg).expect("cloud default should build");
        assert_eq!(e.name(), "cloud");
    }

    #[test]
    fn strict_mode_no_longer_bails_with_cloud_default() {
        // Strict mode used to bail when endpoint/model were unset because
        // the only fallback was InertEmbedder. Now the lax-and-strict
        // paths share the cloud fallback; strict bail is a no-op here
        // and auth failures surface at first embed() call instead.
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = true;
        touch_auth_profile(&cfg);
        let e = build_embedder_from_config(&cfg).expect("cloud default should build");
        assert_eq!(e.name(), "cloud");
    }

    #[test]
    fn local_ai_usage_embeddings_routes_to_ollama() {
        // When the Local AI Settings "Memory embeddings" checkbox is on
        // (runtime_enabled && usage.embeddings), memory tree routes to
        // Ollama using the user's chosen `embedding_model_id`. The
        // explicit endpoint/model override is left unset so we exercise
        // the use_local_for_embeddings() branch.
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.local_ai.runtime_enabled = true;
        cfg.local_ai.usage.embeddings = true;
        cfg.local_ai.embedding_model_id = "all-minilm:latest".to_string();
        let e = build_embedder_from_config(&cfg).expect("ollama path should build");
        assert_eq!(e.name(), "ollama");
    }

    #[test]
    fn local_ai_usage_off_with_session_falls_back_to_cloud() {
        // runtime_enabled=true but usage.embeddings=false → cloud (with session).
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.local_ai.runtime_enabled = true;
        cfg.local_ai.usage.embeddings = false;
        touch_auth_profile(&cfg);
        let e = build_embedder_from_config(&cfg).expect("cloud default should build");
        assert_eq!(e.name(), "cloud");
    }

    #[test]
    fn explicit_endpoint_override_wins_over_local_ai_flag() {
        // Power-user override beats the checkbox.
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = Some("http://staging-embed:11434".into());
        cfg.memory_tree.embedding_model = Some("bge-m3".into());
        cfg.local_ai.runtime_enabled = true;
        cfg.local_ai.usage.embeddings = true;
        let e = build_embedder_from_config(&cfg).expect("override path should build");
        assert_eq!(e.name(), "ollama");
    }
}
