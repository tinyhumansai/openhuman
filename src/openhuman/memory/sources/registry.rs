//! Config discovery and write-locking around the source registry's CRUD.
//!
//! The registry itself is [`tinymemory_sources::registry::SourceRegistry`],
//! which reads and rewrites the `[[memory_sources]]` table in the host's own
//! config file. This layer adds the two things that are the host's: **which**
//! config file, and a lock that serialises writes to it.
//!
//! # Why this is here and not reached through the engine (#5560)
//!
//! It was `tinymemory_core::sources::registry`, re-exported into this module by
//! a glob — which meant every source read in the host was a compile-time link
//! to the memory engine. That layer named nothing engine-shaped: no store, no
//! SQLite, no TinyCortex. It was `SourceRegistry::new(config.config_path())`
//! plus a `tokio::sync::Mutex`, over a file **this host writes**. So it came
//! home, and the crate that owns the registry is now a direct dependency.
//!
//! The port is function for function, with the same locking and the same error
//! stringification, so nothing about the on-disk format or the RPC surface
//! moves.
//!
//! # The `_in` variants take an explicit config, and that is load-bearing
//!
//! [`list_sources`] and friends resolve their config through
//! `config::rpc::load_config_with_timeout`, i.e. from the process environment.
//! That is right for an RPC handler, which serves the active user, and wrong
//! for anything bound to one workspace: reading the global path there would let
//! a caller bound to workspace B answer with workspace A's sources — the
//! cross-workspace leak the workspace-keyed memory binding exists to prevent.
//!
//! The `_in` variants are synchronous because only the config lookup was ever
//! async; the registry read itself is not.

use std::sync::OnceLock;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;

use tinymemory_sources::types::{MemorySourceEntry, SourceKind};
pub use tinymemory_sources::{
    apply_kind_defaults, memory_sync_defaults_for_toolkit, ComposioUpsertTarget, MemorySourcePatch,
};

static MEMORY_SOURCES_WRITE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

/// Serialise writes to the registry file.
///
/// The registry rewrites the whole `[[memory_sources]]` table, so two
/// concurrent writers would each read, mutate and write back — and the second
/// would drop the first's change with no error anywhere.
pub(crate) async fn memory_sources_write_guard() -> tokio::sync::MutexGuard<'static, ()> {
    MEMORY_SOURCES_WRITE_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

async fn registry() -> Result<tinymemory_sources::registry::SourceRegistry, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    Ok(registry_in(&config))
}

fn registry_in(config: &Config) -> tinymemory_sources::registry::SourceRegistry {
    tinymemory_sources::registry::SourceRegistry::new(config.config_path.clone())
}

/// Every registered source, in file order.
///
/// # Errors
///
/// The registry's error, stringified, when the file cannot be read or parsed.
pub async fn list_sources() -> Result<Vec<MemorySourceEntry>, String> {
    registry().await?.list().map_err(|error| error.to_string())
}

/// Enabled sources of one kind.
///
/// # Errors
///
/// As [`list_sources`].
pub async fn list_enabled_by_kind(kind: SourceKind) -> Result<Vec<MemorySourceEntry>, String> {
    registry()
        .await?
        .list_enabled_by_kind(kind)
        .map_err(|error| error.to_string())
}

/// One source by id, or `None` when it is not registered.
///
/// # Errors
///
/// As [`list_sources`].
pub async fn get_source(id: &str) -> Result<Option<MemorySourceEntry>, String> {
    registry().await?.get(id).map_err(|error| error.to_string())
}

/// [`get_source`] against an **explicit** config — see the module docs for why
/// a workspace-bound caller must not read the process-global path.
///
/// # Errors
///
/// As [`list_sources`].
pub fn get_source_in(config: &Config, id: &str) -> Result<Option<MemorySourceEntry>, String> {
    registry_in(config)
        .get(id)
        .map_err(|error| error.to_string())
}

/// [`list_sources`] against an **explicit** config — same reasoning as
/// [`get_source_in`].
///
/// This is what lets a host-config view answer `memory_sources_json` from the
/// file the host writes rather than from a load-time snapshot (openhuman#5820).
///
/// # Errors
///
/// As [`list_sources`].
pub fn list_sources_in(config: &Config) -> Result<Vec<MemorySourceEntry>, String> {
    registry_in(config)
        .list()
        .map_err(|error| error.to_string())
}

/// Replace the registry file an **explicit** config names — the write half of
/// [`list_sources_in`], so a view that reads live can also write through
/// (openhuman#5820).
///
/// # Errors
///
/// The registry's error, stringified, when an entry fails validation or the
/// file cannot be written atomically.
pub fn replace_sources_in(config: &Config, entries: &[MemorySourceEntry]) -> Result<(), String> {
    registry_in(config)
        .replace_all(entries)
        .map_err(|error| error.to_string())
}

/// Register a new source.
///
/// # Errors
///
/// As [`replace_sources_in`].
pub async fn add_source(entry: MemorySourceEntry) -> Result<MemorySourceEntry, String> {
    let _guard = memory_sources_write_guard().await;
    log::debug!("[memory_sources] crate add kind={}", entry.kind.as_str());
    registry()
        .await?
        .add(entry)
        .map_err(|error| error.to_string())
}

/// Apply a patch to one registered source.
///
/// # Errors
///
/// As [`replace_sources_in`].
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

/// Remove one source; `false` when it was not registered.
///
/// # Errors
///
/// As [`replace_sources_in`].
pub async fn remove_source(id: &str) -> Result<bool, String> {
    let _guard = memory_sources_write_guard().await;
    registry()
        .await?
        .remove(id)
        .map_err(|error| error.to_string())
}

/// Remove every Composio source bound to a connection, returning how many went.
///
/// # Errors
///
/// As [`replace_sources_in`].
pub async fn remove_composio_source_by_connection_id(connection_id: &str) -> Result<usize, String> {
    let _guard = memory_sources_write_guard().await;
    registry()
        .await?
        .remove_composio_source_by_connection_id(connection_id)
        .map_err(|error| error.to_string())
}

/// Register or update the Composio source for one connection.
///
/// # Errors
///
/// As [`replace_sources_in`].
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

/// [`upsert_composio_source`] for many connections under one lock hold.
///
/// # Errors
///
/// As [`replace_sources_in`].
pub async fn upsert_composio_sources_batch(
    targets: &[ComposioUpsertTarget],
) -> Result<u32, String> {
    let _guard = memory_sources_write_guard().await;
    registry()
        .await?
        .upsert_composio_sources_batch(targets)
        .map_err(|error| error.to_string())
}

/// Re-apply creation-time defaults across every registered source.
///
/// # Errors
///
/// As [`replace_sources_in`].
pub async fn apply_all_in() -> Result<Vec<MemorySourceEntry>, String> {
    let _guard = memory_sources_write_guard().await;
    registry()
        .await?
        .apply_all_in()
        .map_err(|error| error.to_string())
}

/// Decode the source registry a host config carries.
///
/// The registry crosses the memory host seam as JSON, so this is where it
/// becomes typed again.
///
/// A malformed or absent registry yields an empty list rather than an error.
/// Every caller is a background loop deciding what to sync, and "nothing is
/// registered" is the fail-closed answer there; propagating would take the loop
/// down over one bad row.
#[must_use]
pub fn decode_memory_sources(config: &Config) -> Vec<MemorySourceEntry> {
    use tinymemory_api::host::MemoryHostConfig as _;
    match config.memory_sources_json() {
        Ok(value) => serde_json::from_value(value).unwrap_or_else(|e| {
            log::warn!("[memory_sources:registry] could not decode memory sources: {e:#}");
            Vec::new()
        }),
        Err(e) => {
            log::warn!("[memory_sources:registry] could not read memory sources: {e:#}");
            Vec::new()
        }
    }
}
