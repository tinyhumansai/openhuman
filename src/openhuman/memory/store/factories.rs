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

use crate::openhuman::config::{EmbeddingRouteConfig, MemoryConfig, StorageProviderConfig};
use crate::openhuman::memory::embeddings::{self, EmbeddingProvider};
use crate::openhuman::memory::store::unified::UnifiedMemory;
use crate::openhuman::memory::traits::Memory;

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
///
/// # Arguments
///
/// * `config` - The memory configuration (provider, model, etc.).
/// * `workspace_dir` - The directory where memory data should be stored.
/// * `api_key` - Optional API key for external embedding providers.
pub fn create_memory(
    config: &MemoryConfig,
    workspace_dir: &Path,
    api_key: Option<&str>,
) -> anyhow::Result<Box<dyn Memory>> {
    create_memory_with_storage_and_routes(config, &[], None, workspace_dir, api_key)
}

/// Create a memory instance with an optional storage provider configuration.
pub fn create_memory_with_storage(
    config: &MemoryConfig,
    storage_provider: Option<&StorageProviderConfig>,
    workspace_dir: &Path,
    api_key: Option<&str>,
) -> anyhow::Result<Box<dyn Memory>> {
    create_memory_with_storage_and_routes(config, &[], storage_provider, workspace_dir, api_key)
}

/// The most comprehensive factory function for creating a memory instance.
///
/// This function initializes the embedding provider and then creates a
/// `UnifiedMemory` instance.
///
/// # Arguments
///
/// * `config` - Core memory configuration.
/// * `_embedding_routes` - Configuration for routing embeddings (currently unused).
/// * `_storage_provider` - Configuration for the storage backend (currently unused).
/// * `workspace_dir` - The directory for storage.
/// * `api_key` - API key for the embedding provider.
pub fn create_memory_with_storage_and_routes(
    config: &MemoryConfig,
    _embedding_routes: &[EmbeddingRouteConfig],
    _storage_provider: Option<&StorageProviderConfig>,
    workspace_dir: &Path,
    api_key: Option<&str>,
) -> anyhow::Result<Box<dyn Memory>> {
    // 1. Create the embedding provider based on config (Local vs Remote).
    let embedder: Arc<dyn EmbeddingProvider> = Arc::from(embeddings::create_embedding_provider(
        &config.embedding_provider,
        api_key,
        &config.embedding_model,
        config.embedding_dimensions,
    )?);

    // 2. Instantiate UnifiedMemory which handles SQLite and vector storage.
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
