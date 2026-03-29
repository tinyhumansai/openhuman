use std::path::Path;
use std::sync::Arc;

use crate::openhuman::config::{EmbeddingRouteConfig, MemoryConfig, StorageProviderConfig};
use crate::openhuman::memory::embeddings::{self, EmbeddingProvider};
use crate::openhuman::memory::store::unified::UnifiedMemory;
use crate::openhuman::memory::traits::Memory;

pub fn effective_memory_backend_name(
    _memory_backend: &str,
    _storage_provider: Option<&StorageProviderConfig>,
) -> String {
    "namespace".to_string()
}

pub fn create_memory(
    config: &MemoryConfig,
    workspace_dir: &Path,
    api_key: Option<&str>,
) -> anyhow::Result<Box<dyn Memory>> {
    create_memory_with_storage_and_routes(config, &[], None, workspace_dir, api_key)
}

pub fn create_memory_with_storage(
    config: &MemoryConfig,
    storage_provider: Option<&StorageProviderConfig>,
    workspace_dir: &Path,
    api_key: Option<&str>,
) -> anyhow::Result<Box<dyn Memory>> {
    create_memory_with_storage_and_routes(config, &[], storage_provider, workspace_dir, api_key)
}

pub fn create_memory_with_storage_and_routes(
    config: &MemoryConfig,
    _embedding_routes: &[EmbeddingRouteConfig],
    _storage_provider: Option<&StorageProviderConfig>,
    workspace_dir: &Path,
    api_key: Option<&str>,
) -> anyhow::Result<Box<dyn Memory>> {
    let embedder: Arc<dyn EmbeddingProvider> = Arc::from(embeddings::create_embedding_provider(
        &config.embedding_provider,
        api_key,
        &config.embedding_model,
        config.embedding_dimensions,
    ));
    let mem = UnifiedMemory::new(workspace_dir, embedder, config.sqlite_open_timeout_secs)?;
    Ok(Box::new(mem))
}

pub fn create_memory_for_migration(
    _backend: &str,
    _workspace_dir: &Path,
) -> anyhow::Result<Box<dyn Memory>> {
    anyhow::bail!("memory migration is disabled for the unified namespace memory core")
}
