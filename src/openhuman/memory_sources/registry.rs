//! Product config discovery and locking around tinycortex source registry CRUD.

use std::sync::OnceLock;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceKind};

pub use tinycortex::memory::sources::{
    memory_sync_defaults_for_toolkit, ComposioUpsertTarget, MemorySourcePatch,
};

static MEMORY_SOURCES_WRITE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

pub(crate) async fn memory_sources_write_guard() -> tokio::sync::MutexGuard<'static, ()> {
    MEMORY_SOURCES_WRITE_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

async fn registry() -> Result<tinycortex::memory::sources::SourceRegistry, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    Ok(tinycortex::memory::sources::SourceRegistry::new(
        config.config_path,
    ))
}

pub async fn list_sources() -> Result<Vec<MemorySourceEntry>, String> {
    registry().await?.list().map_err(|error| error.to_string())
}

pub async fn list_enabled_by_kind(kind: SourceKind) -> Result<Vec<MemorySourceEntry>, String> {
    registry()
        .await?
        .list_enabled_by_kind(kind)
        .map_err(|error| error.to_string())
}

pub async fn get_source(id: &str) -> Result<Option<MemorySourceEntry>, String> {
    registry().await?.get(id).map_err(|error| error.to_string())
}

pub async fn add_source(entry: MemorySourceEntry) -> Result<MemorySourceEntry, String> {
    let _guard = memory_sources_write_guard().await;
    log::debug!("[memory_sources] crate add kind={}", entry.kind.as_str());
    registry()
        .await?
        .add(entry)
        .map_err(|error| error.to_string())
}

pub async fn update_source(
    id: &str,
    patch: MemorySourcePatch,
) -> Result<MemorySourceEntry, String> {
    let _guard = memory_sources_write_guard().await;
    log::debug!("[memory_sources] crate update id_len={}", id.len());
    registry()
        .await?
        .update(id, patch)
        .map_err(|error| error.to_string())
}

pub async fn remove_source(id: &str) -> Result<bool, String> {
    let _guard = memory_sources_write_guard().await;
    registry()
        .await?
        .remove(id)
        .map_err(|error| error.to_string())
}

pub async fn remove_composio_source_by_connection_id(connection_id: &str) -> Result<usize, String> {
    let _guard = memory_sources_write_guard().await;
    registry()
        .await?
        .remove_composio_source_by_connection_id(connection_id)
        .map_err(|error| error.to_string())
}

pub async fn upsert_composio_source(
    toolkit: &str,
    connection_id: &str,
    label: &str,
) -> Result<MemorySourceEntry, String> {
    let _guard = memory_sources_write_guard().await;
    registry()
        .await?
        .upsert_composio_source(toolkit, connection_id, label)
        .map_err(|error| error.to_string())
}

pub async fn upsert_composio_sources_batch(
    targets: &[ComposioUpsertTarget],
) -> Result<u32, String> {
    let _guard = memory_sources_write_guard().await;
    registry()
        .await?
        .upsert_composio_sources_batch(targets)
        .map_err(|error| error.to_string())
}

pub async fn apply_all_in() -> Result<Vec<MemorySourceEntry>, String> {
    let _guard = memory_sources_write_guard().await;
    registry()
        .await?
        .apply_all_in()
        .map_err(|error| error.to_string())
}
