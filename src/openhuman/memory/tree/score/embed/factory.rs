//! Build an [`Embedder`] from [`Config::memory_tree`] settings.
//!
//! Resolution order:
//! 1. `memory_tree.embedding_endpoint` + `memory_tree.embedding_model`
//!    both Some → [`OllamaEmbedder`]
//! 2. Otherwise → depends on `memory_tree.embedding_strict`:
//!    - `true`  → bail with a clear "configure Ollama for Phase 4" error
//!    - `false` → fall back to [`InertEmbedder`] (zero vectors) with a
//!      warn log so the operator notices embeddings are disabled
//!
//! Env var overrides applied in [`crate::openhuman::config::load`]:
//! - `OPENHUMAN_MEMORY_EMBED_ENDPOINT`
//! - `OPENHUMAN_MEMORY_EMBED_MODEL`
//! - `OPENHUMAN_MEMORY_EMBED_TIMEOUT_MS`

use anyhow::Result;

use super::{Embedder, InertEmbedder, OllamaEmbedder};
use crate::openhuman::config::Config;

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
            if tree_cfg.embedding_strict {
                anyhow::bail!(
                    "memory_tree embedding is required (embedding_strict=true) but \
                     embedding_endpoint/embedding_model are unset. Set \
                     `memory_tree.embedding_endpoint` + `.embedding_model` in \
                     config.toml or export OPENHUMAN_MEMORY_EMBED_ENDPOINT / \
                     OPENHUMAN_MEMORY_EMBED_MODEL — or set \
                     `memory_tree.embedding_strict = false` to fall back to zero \
                     vectors (embeddings will not contribute to retrieval rerank)."
                );
            }
            log::warn!(
                "[memory_tree::embed::factory] no embedding endpoint/model — \
                 falling back to InertEmbedder (zero vectors). Set \
                 memory_tree.embedding_endpoint to enable semantic retrieval."
            );
            Ok(Box::new(InertEmbedder::new()))
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
        (tmp, cfg)
    }

    #[test]
    fn ollama_chosen_when_endpoint_and_model_set() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = Some("http://localhost:11434".into());
        cfg.memory_tree.embedding_model = Some("nomic-embed-text".into());
        cfg.memory_tree.embedding_timeout_ms = Some(5000);
        let e = build_embedder_from_config(&cfg).expect("Ollama path should build");
        assert_eq!(e.name(), "ollama");
    }

    #[test]
    fn strict_mode_bails_on_missing_endpoint() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = true;
        // `Box<dyn Embedder>` isn't `Debug`, so go through `match` rather
        // than `unwrap_err` (which needs Debug on the Ok variant).
        match build_embedder_from_config(&cfg) {
            Ok(_) => panic!("expected strict-mode bail"),
            Err(e) => assert!(e.to_string().contains("embedding_strict"), "{e}"),
        }
    }

    #[test]
    fn lax_mode_falls_back_to_inert() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        let e = build_embedder_from_config(&cfg).expect("lax path should build");
        assert_eq!(e.name(), "inert");
    }

    #[test]
    fn empty_strings_count_as_unset() {
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.embedding_endpoint = Some("".into());
        cfg.memory_tree.embedding_model = Some("".into());
        cfg.memory_tree.embedding_strict = false;
        let e = build_embedder_from_config(&cfg).expect("lax path should build");
        assert_eq!(e.name(), "inert");
    }
}
