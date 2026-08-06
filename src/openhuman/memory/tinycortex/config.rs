//! `Config` → [`tinycortex::memory::MemoryConfig`] mapping (W1).
//!
//! The crate's [`MemoryConfig`] is the single input every engine primitive
//! takes (`workspace`, embedding dims/model/strict, tree budgets, retrieval
//! weight profile, sync budget). This adapter derives it from OpenHuman's
//! host [`Config`] plus the resolved memory workspace root, so the rest of the
//! seam constructs engine calls from real product configuration.
//!
//! Field provenance:
//! - `workspace` ← the memory workspace root (same root `MemoryClient` opens).
//! - `embedding.dim` ← `config.memory.embedding_dimensions`.
//! - `embedding.model` ← `config.memory.embedding_model`.
//! - `embedding.strict` ← `config.memory_tree.embedding_strict` (when false the
//!   engine tolerates an inert embedder and falls back to scope+recency rerank).
//! - `tree` / `retrieval` / `sync_budget` ← crate defaults, which already match
//!   the host engine's constants (`INPUT_TOKEN_BUDGET = 50_000`,
//!   `OUTPUT_TOKEN_BUDGET = 5_000`, `SUMMARY_FANOUT = 10`,
//!   `DEFAULT_FLUSH_AGE_SECS = 604_800`). The `tree_policy.rs` flavour overlays
//!   and per-source `WeightProfile` selection are layered on at call sites in
//!   later workstreams; this base mapping is the W1 foundation.

use std::path::PathBuf;

use tinycortex::memory::config::EmbeddingConfig;
use tinycortex::memory::MemoryConfig;

use crate::openhuman::config::Config;

/// Build a [`MemoryConfig`] from the host [`Config`] and the resolved memory
/// workspace root.
///
/// `workspace` is the directory under which the engine stores `chunks.db`, the
/// content vault, and the tree DBs — it must be the same root the host
/// `MemoryClient` opens so an existing user workspace is read in place (parity
/// is gated by the W3 golden-workspace harness).
pub fn memory_config_from(config: &Config, workspace: PathBuf) -> MemoryConfig {
    let mut mc = MemoryConfig::new(workspace);
    mc.content_root = Some(config.memory_tree_content_root());
    mc.embedding = EmbeddingConfig {
        dim: config.memory.embedding_dimensions,
        model: config.memory.embedding_model.clone(),
        strict: config.memory_tree.embedding_strict,
    };
    mc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_workspace_and_embedding_from_host_config() {
        let mut config = Config::default();
        config.memory.embedding_dimensions = 1024;
        config.memory.embedding_model = "embedding-v1".to_string();
        config.memory_tree.embedding_strict = true;

        let workspace = PathBuf::from("/tmp/openhuman/ws");
        let mc = memory_config_from(&config, workspace.clone());

        assert_eq!(mc.workspace, workspace);
        assert_eq!(mc.embedding.dim, 1024);
        assert_eq!(mc.embedding.model, "embedding-v1");
        assert!(mc.embedding.strict);
    }

    #[test]
    fn tree_defaults_match_engine_constants() {
        // The base mapping leaves tree budgets at the crate defaults, which are
        // the host engine's own constants — asserted here so a crate-side change
        // to those defaults surfaces as a failing parity test rather than a
        // silent behaviour drift.
        let mc = memory_config_from(&Config::default(), PathBuf::from("/tmp/ws"));
        assert_eq!(mc.tree.input_token_budget, 50_000);
        assert_eq!(mc.tree.output_token_budget, 5_000);
        assert_eq!(mc.tree.summary_fanout, 10);
        assert_eq!(mc.tree.flush_age_secs, 604_800);
    }
}
