//! # Memory Store Factories
//!
//! Factory functions for creating and initializing various memory store
//! implementations.
//!
//! This module provides a centralized way to instantiate memory stores based on
//! configuration, ensuring that the correct embedding providers and storage
//! backends are used. Currently, it primarily focuses on creating
//! `UnifiedMemory` instances.

use std::path::Path;
use std::sync::Arc;

use crate::openhuman::config::{
    EmbeddingRouteConfig, LocalAiConfig, MemoryConfig, StorageProviderConfig,
};
use crate::openhuman::embeddings::{
    self, EmbeddingProvider, DEFAULT_OLLAMA_DIMENSIONS, DEFAULT_OLLAMA_MODEL,
};
use crate::openhuman::memory::store::unified::UnifiedMemory;
use crate::openhuman::memory::traits::Memory;

/// Returns the effective `(provider, model, dimensions)` triple for the
/// embedding backend.
///
/// The user-facing default is `"cloud"` (OpenHuman backend, Voyage-backed) so
/// fresh installs work without a local Ollama daemon. When the user has
/// explicitly opted into local AI for embeddings —
/// [`LocalAiConfig::use_local_for_embeddings`] — we route through the local
/// Ollama embedder regardless of what `memory.embedding_provider` says, since
/// that toggle is a stronger statement of intent than the per-section default.
pub fn effective_embedding_settings(
    memory: &MemoryConfig,
    local_ai: Option<&LocalAiConfig>,
) -> (String, String, usize) {
    if local_ai
        .map(LocalAiConfig::use_local_for_embeddings)
        .unwrap_or(false)
    {
        // Trim once and reuse — the emptiness check and the final model
        // string must agree, otherwise a value like "  bge-m3  " would pass
        // through to Ollama with surrounding whitespace and 404.
        let model = local_ai
            .map(|c| c.embedding_model_id.trim())
            .filter(|m| !m.is_empty())
            .unwrap_or(DEFAULT_OLLAMA_MODEL)
            .to_string();
        return ("ollama".to_string(), model, DEFAULT_OLLAMA_DIMENSIONS);
    }
    (
        memory.embedding_provider.clone(),
        memory.embedding_model.clone(),
        memory.embedding_dimensions,
    )
}

/// Returns the effective name of the memory backend being used.
///
/// Currently, this always returns "namespace" as the unified memory system
/// is the standard.
pub fn effective_memory_backend_name(
    _memory_backend: &str,
    _storage_provider: Option<&StorageProviderConfig>,
) -> String {
    "namespace".to_string()
}

/// Create a standard memory instance based on the provided configuration.
pub fn create_memory(
    config: &MemoryConfig,
    workspace_dir: &Path,
) -> anyhow::Result<Box<dyn Memory>> {
    create_memory_with_storage_and_routes(config, &[], None, workspace_dir)
}

/// Create a memory instance with an optional storage provider configuration.
pub fn create_memory_with_storage(
    config: &MemoryConfig,
    storage_provider: Option<&StorageProviderConfig>,
    workspace_dir: &Path,
) -> anyhow::Result<Box<dyn Memory>> {
    create_memory_full(config, &[], storage_provider, None, workspace_dir)
}

/// Create a memory instance honoring both the `memory` and `local_ai` sections.
///
/// Used by top-level entry points (agent harness, channels runtime) that have
/// the full `Config` in scope and want the local-AI opt-in to flip the
/// embedder to Ollama.
pub fn create_memory_with_local_ai(
    memory: &MemoryConfig,
    local_ai: &LocalAiConfig,
    embedding_routes: &[EmbeddingRouteConfig],
    storage_provider: Option<&StorageProviderConfig>,
    workspace_dir: &Path,
) -> anyhow::Result<Box<dyn Memory>> {
    create_memory_full(
        memory,
        embedding_routes,
        storage_provider,
        Some(local_ai),
        workspace_dir,
    )
}

/// Back-compat wrapper preserved for existing call sites that don't have a
/// `LocalAiConfig` to pass. The local-AI opt-in is not honored on this path —
/// use [`create_memory_with_local_ai`] when both sections are available.
pub fn create_memory_with_storage_and_routes(
    config: &MemoryConfig,
    embedding_routes: &[EmbeddingRouteConfig],
    storage_provider: Option<&StorageProviderConfig>,
    workspace_dir: &Path,
) -> anyhow::Result<Box<dyn Memory>> {
    create_memory_full(
        config,
        embedding_routes,
        storage_provider,
        None,
        workspace_dir,
    )
}

/// The most comprehensive factory function for creating a memory instance.
///
/// This function initializes the embedding provider and then creates a
/// `UnifiedMemory` instance.
fn create_memory_full(
    config: &MemoryConfig,
    _embedding_routes: &[EmbeddingRouteConfig],
    _storage_provider: Option<&StorageProviderConfig>,
    local_ai: Option<&LocalAiConfig>,
    workspace_dir: &Path,
) -> anyhow::Result<Box<dyn Memory>> {
    // 1. Resolve the effective (provider, model, dims) — local-AI opt-in
    //    overrides the per-section default when both are present.
    let (provider, model, dims) = effective_embedding_settings(config, local_ai);
    log::debug!(
        "[memory::factory] effective embedding settings: provider={} model={} dims={} (local_ai_opt_in={})",
        provider,
        model,
        dims,
        local_ai
            .map(LocalAiConfig::use_local_for_embeddings)
            .unwrap_or(false),
    );

    // 2. Create the embedding provider.
    let embedder: Arc<dyn EmbeddingProvider> = Arc::from(
        embeddings::create_embedding_provider(&provider, &model, dims).inspect_err(|err| {
            log::warn!(
                "[memory::factory] create_embedding_provider failed provider={} model={} dims={}: {err}",
                provider,
                model,
                dims,
            );
        })?,
    );

    // 3. Instantiate UnifiedMemory which handles SQLite and vector storage.
    let mem = UnifiedMemory::new(workspace_dir, embedder, config.sqlite_open_timeout_secs)?;
    Ok(Box::new(mem))
}

/// Create a memory instance specifically for migration purposes.
///
/// NOTE: This is currently disabled for the unified namespace memory core.
pub fn create_memory_for_migration(
    _backend: &str,
    _workspace_dir: &Path,
) -> anyhow::Result<Box<dyn Memory>> {
    anyhow::bail!("memory migration is disabled for the unified namespace memory core")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_memory_backend_name_always_returns_namespace() {
        assert_eq!(effective_memory_backend_name("sqlite", None), "namespace");
        assert_eq!(effective_memory_backend_name("anything", None), "namespace");
        assert_eq!(effective_memory_backend_name("", None), "namespace");
    }

    #[test]
    fn create_memory_for_migration_always_errors() {
        let tmp = tempfile::tempdir().unwrap();
        // Box<dyn Memory> doesn't impl Debug, so we can't use .unwrap_err().
        // Use match instead.
        match create_memory_for_migration("any", tmp.path()) {
            Ok(_) => panic!("expected error"),
            Err(e) => assert!(
                e.to_string().contains("migration is disabled"),
                "unexpected error: {e}"
            ),
        }
    }
}
