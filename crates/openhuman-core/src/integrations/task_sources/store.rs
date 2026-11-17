//! Persistence for the `task_sources` domain.
//!
//! Two collections of the storage backend (`store_documents.rs`):
//!   * `task_sources` — the configured sources (provider + filter +
//!     schedule + routing target).
//!   * `ingested_tasks` — per-(source, external task) dedup ledger with
//!     an edit-aware `content_hash`, plus the normalized task payload so
//!     the UI can list recently ingested items.
//!
//! The `task_sources/sources.db` older installs wrote is read once,
//! read-only, by the storage importer (`store_import.rs`).

use anyhow::Result;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::Config;
use crate::integrations::composio::contract::NormalizedTask;

use super::types::{
    FetchReason, FilterSpec, ProviderSlug, SourceTarget, TaskSource, TaskSourcePatch,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestedTaskRef {
    pub external_id: String,
}

/// Compute an edit-aware content hash for a task. Two fetches of the
/// same `external_id` whose title/body/status/updated_at/url differ produce
/// different hashes, so an *edited* upstream item re-ingests.
///
/// `url` is part of the canonical form because it is load-bearing downstream
/// (it lands in the card's `source_metadata`/notes and drives external
/// write-back); a provider that edits the URL without advancing `updated_at`
/// would otherwise leave a stale link on the board.
///
/// Uses SHA-256 over a canonical field-delimited representation so the
/// digest is stable across Rust/toolchain versions (the dedup key is
/// persisted on disk; a non-deterministic hasher would force spurious
/// re-ingests after a toolchain bump).
pub fn content_hash(task: &NormalizedTask) -> String {
    let canonical = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        task.title,
        task.body.as_deref().unwrap_or(""),
        task.status.as_deref().unwrap_or(""),
        task.updated_at.as_deref().unwrap_or(""),
        task.url.as_deref().unwrap_or(""),
    );
    let digest = Sha256::digest(canonical.as_bytes());
    format!("{digest:x}")
}

/// Insert a new task source.
#[allow(clippy::too_many_arguments)]
pub fn add_source(
    config: &Config,
    provider: ProviderSlug,
    connection_id: Option<String>,
    name: Option<String>,
    filter: FilterSpec,
    interval_secs: u64,
    target: SourceTarget,
    max_tasks_per_fetch: u32,
) -> Result<TaskSource> {
    if filter.provider() != provider {
        anyhow::bail!(
            "filter provider '{}' does not match source provider '{}'",
            filter.provider().as_str(),
            provider.as_str()
        );
    }
    // Normalize blank optional fields to NULL so a whitespace-only
    // connection_id can't masquerade as a real selector (mirrors
    // `update_source`).
    let connection_id = connection_id.filter(|s| !s.trim().is_empty());
    let name = name.filter(|s| !s.trim().is_empty());
    let id = Uuid::new_v4().to_string();
    let now = Utc::now();
    super::store_documents::current(config)?.add_source(&TaskSource {
        id,
        provider,
        connection_id,
        name,
        enabled: true,
        filter,
        interval_secs,
        target,
        max_tasks_per_fetch,
        created_at: now,
        last_fetch_at: None,
        last_status: None,
    })
}

pub fn get_source(config: &Config, id: &str) -> Result<TaskSource> {
    let docs = super::store_documents::current(config)?;
    docs.get_source(id)
}

pub fn list_sources(config: &Config) -> Result<Vec<TaskSource>> {
    let docs = super::store_documents::current(config)?;
    docs.list_sources()
}

/// Applies `patch` to `source`, as both stores do.
///
/// # Errors
///
/// When the patch's filter is for a different provider than the source.
pub(super) fn apply_patch(source: &mut TaskSource, patch: TaskSourcePatch) -> Result<()> {
    if let Some(name) = patch.name {
        source.name = Some(name).filter(|s| !s.trim().is_empty());
    }
    if let Some(enabled) = patch.enabled {
        source.enabled = enabled;
    }
    if let Some(filter) = patch.filter {
        if filter.provider() != source.provider {
            anyhow::bail!(
                "patch filter provider '{}' does not match source provider '{}'",
                filter.provider().as_str(),
                source.provider.as_str()
            );
        }
        source.filter = filter;
    }
    if let Some(interval_secs) = patch.interval_secs {
        source.interval_secs = interval_secs;
    }
    if let Some(target) = patch.target {
        source.target = target;
    }
    if let Some(max) = patch.max_tasks_per_fetch {
        source.max_tasks_per_fetch = max;
    }
    if let Some(connection_id) = patch.connection_id {
        source.connection_id = Some(connection_id).filter(|s| !s.trim().is_empty());
    }
    Ok(())
}

/// Apply a partial patch to a task source.
///
/// **Implementation note:** this function opens three separate SQLite
/// connections (read-modify-write + read-back). At settings-panel scale the
/// overhead is acceptable, but there is a theoretical TOCTOU window between
/// the initial `get_source` and the subsequent `UPDATE`. A future refactor
/// could fold all three operations into a single `with_connection` call using
/// a SQL `UPDATE … RETURNING` pattern. The document store applies the patch
/// under compare-and-swap, so it has no such window.
pub fn update_source(config: &Config, id: &str, patch: TaskSourcePatch) -> Result<TaskSource> {
    let docs = super::store_documents::current(config)?;
    docs.update_source(id, patch)
}

pub fn remove_source(config: &Config, id: &str) -> Result<()> {
    let docs = super::store_documents::current(config)?;
    docs.remove_source(id)
}

/// Update a source's `last_fetch_at` / `last_status` after a fetch pass.
pub fn record_fetch(
    config: &Config,
    id: &str,
    finished_at: DateTime<Utc>,
    reason: FetchReason,
    status: &str,
) -> Result<()> {
    let docs = super::store_documents::current(config)?;
    docs.record_fetch(id, finished_at, reason, status)
}

/// True when this `(source, external_id)` was already ingested with the
/// *same* content hash. A differing hash (edited upstream) returns
/// `false` so the item re-ingests.
pub fn is_ingested(
    config: &Config,
    source_id: &str,
    external_id: &str,
    hash: &str,
) -> Result<bool> {
    let docs = super::store_documents::current(config)?;
    docs.is_ingested(source_id, external_id, hash)
}

/// Record a routed task in the dedup ledger (idempotent upsert).
///
/// The `card_id` column is left `NULL`: tasks are no longer mirrored onto a
/// todo board, the ledger row itself is the record. The column stays so
/// older databases open unchanged.
pub fn mark_ingested(config: &Config, source_id: &str, task: &NormalizedTask) -> Result<()> {
    let docs = super::store_documents::current(config)?;
    docs.mark_ingested(source_id, task)
}

/// Whether `(source_id, external_id)` has been ingested before under any
/// content hash. The pipeline uses it to tell an edited upstream task from a
/// brand-new one in its logs.
pub fn was_ingested(config: &Config, source_id: &str, external_id: &str) -> Result<bool> {
    let docs = super::store_documents::current(config)?;
    docs.was_ingested(source_id, external_id)
}

/// Return ingested task ids for one source. Used by reconciliation to prune
/// ledger rows that no longer match the upstream source/filter.
pub fn list_ingested_refs(config: &Config, source_id: &str) -> Result<Vec<IngestedTaskRef>> {
    let docs = super::store_documents::current(config)?;
    docs.list_ingested_refs(source_id)
}

/// Delete one ingested ledger row after its board card has been reconciled.
pub fn remove_ingested(config: &Config, source_id: &str, external_id: &str) -> Result<bool> {
    let docs = super::store_documents::current(config)?;
    docs.remove_ingested(source_id, external_id)
}

/// List the most recently ingested tasks for a source (newest first).
pub fn list_ingested(
    config: &Config,
    source_id: &str,
    limit: usize,
) -> Result<Vec<NormalizedTask>> {
    let docs = super::store_documents::current(config)?;
    docs.list_ingested(source_id, limit)
}

/// Delete every task source (+ cascade ingested rows). Used by the E2E
/// `test_reset` RPC.
pub fn clear_all(config: &Config) -> Result<usize> {
    let docs = super::store_documents::current(config)?;
    docs.clear_all()
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
