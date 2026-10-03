//! Syncing sources into memory.
//!
//! Non-Composio sources are read through `tinymemory-sources`' readers
//! (`collect_items`), which turn each file, page, commit or feed entry into a
//! `Document` with its metadata filled. Every item is scrubbed and stored on
//! the bound engine; per-item failures are logged and skipped, not fatal.
//! Composio sources go through [`super::composio`].

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Utc};
use tinymemory::documents::NativeConverter;
use tinymemory::sources::readers::reader_for_request;
use tinymemory::sources::{apply_kind_defaults, collect_items, MemorySourceEntry, SourceKind};

use crate::config::schema::{MemorySourceConfig, MemorySourceKind};
use crate::config::Config;
use crate::memory::engine::{self, BoundEngine};
use crate::memory::error::{MemoryError, MemoryResult};
use crate::memory::ops::store_on;
use crate::memory::types::SourceStatus;

use super::state;

/// Sources syncing right now, per workspace, so a second request for the same
/// source does not start a parallel run.
static RUNNING: LazyLock<Mutex<HashSet<(PathBuf, String)>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// The `tinymemory-sources` entry that reads `source`; `None` for Composio,
/// which syncs through the connector module instead.
pub(super) fn reader_entry(source: &MemorySourceConfig) -> MemoryResult<Option<MemorySourceEntry>> {
    let kind = match source.kind {
        MemorySourceKind::Folder => SourceKind::Folder,
        MemorySourceKind::File => SourceKind::File,
        MemorySourceKind::Link => SourceKind::WebPage,
        MemorySourceKind::Github => SourceKind::GithubRepo,
        MemorySourceKind::Rss => SourceKind::RssFeed,
        MemorySourceKind::Composio => return Ok(None),
    };
    let mut entry = MemorySourceEntry::new(source.id.clone(), kind, source.label.clone());
    match source.kind {
        MemorySourceKind::Folder | MemorySourceKind::File => {
            entry.path = Some(source.target.clone())
        }
        _ => entry.url = Some(source.target.clone()),
    }
    apply_kind_defaults(&mut entry);
    entry
        .validate()
        .map_err(|error| MemoryError::invalid(error.to_string()))?;
    Ok(Some(entry))
}

/// Reads `source` and stores what it yields. Returns the number stored.
pub async fn sync_one(config: &Config, source: &MemorySourceConfig) -> MemoryResult<u64> {
    let bound = engine::resolve(config).engine()?;
    let Some(entry) = reader_entry(source)? else {
        return super::composio::sync_toolkit(config, &bound, source).await;
    };
    let reader = reader_for_request(&entry.kind);
    let collected = collect_items(
        reader.as_ref(),
        &entry,
        &config.action_dir,
        &NativeConverter,
    )
    .await
    .map_err(|error| MemoryError::Engine(format!("reading the source failed: {error}")))?;
    if !collected.skipped.is_empty() {
        tracing::debug!(
            id = %source.id,
            skipped = collected.skipped.len(),
            "[memory:sources] some items could not be read"
        );
    }
    store_all(&bound, collected.items, &source.id).await
}

pub(super) async fn store_all(
    bound: &BoundEngine,
    items: Vec<tinymemory::StoreItem>,
    source_id: &str,
) -> MemoryResult<u64> {
    let mut stored = 0u64;
    let mut last_error = None;
    for item in items {
        match store_on(bound, item).await {
            Ok(_) => stored += 1,
            Err(error @ (MemoryError::Unauthorized(_) | MemoryError::Off(_))) => return Err(error),
            Err(error) => {
                tracing::debug!(id = %source_id, code = error.code(), "[memory:sources] item store failed");
                last_error = Some(error);
            }
        }
    }
    match (stored, last_error) {
        (0, Some(error)) => Err(error),
        _ => Ok(stored),
    }
}

/// `memory_sources_sync`: starts a background sync of source `id`, or of
/// every source, and returns the ids whose sync started.
pub fn start_sync(config: &Config, id: Option<&str>) -> MemoryResult<Vec<String>> {
    engine::resolve(config).engine()?;
    let targets: Vec<MemorySourceConfig> = match id {
        Some(id) => vec![config
            .memory
            .sources
            .iter()
            .find(|source| source.id == id)
            .cloned()
            .ok_or_else(|| MemoryError::invalid(format!("no source `{id}`")))?],
        None => config.memory.sources.clone(),
    };
    let mut started = Vec::new();
    for source in targets {
        let key = (config.workspace_dir.clone(), source.id.clone());
        let claimed = RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key.clone());
        if !claimed {
            tracing::debug!(id = %source.id, "[memory:sources] already syncing");
            continue;
        }
        state::update(&config.workspace_dir, &source.id, |state| {
            state.status = SourceStatus::Syncing;
            state.error = None;
        });
        started.push(source.id.clone());
        let config = config.clone();
        tokio::spawn(async move {
            run_and_record(&config, &source).await;
            RUNNING
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&key);
        });
    }
    Ok(started)
}

async fn run_and_record(config: &Config, source: &MemorySourceConfig) {
    let result = sync_one(config, source).await;
    let now = Utc::now();
    state::update(&config.workspace_dir, &source.id, |state| match &result {
        Ok(items) => {
            state.status = SourceStatus::Idle;
            state.last_sync_at = Some(now);
            state.items = *items;
            state.error = None;
        }
        Err(error) => {
            state.status = SourceStatus::Error;
            state.last_sync_at = Some(now);
            state.error = Some(error.to_string());
        }
    });
    match result {
        Ok(items) => tracing::info!(id = %source.id, items, "[memory:sources] sync finished"),
        Err(error) => tracing::warn!(
            id = %source.id,
            code = error.code(),
            "[memory:sources] sync failed"
        ),
    }
}

/// The scheduled tick: starts every source whose `schedule_mins` has elapsed.
/// Returns the ids started. Memory off starts nothing.
pub fn sync_due(config: &Config, now: DateTime<Utc>) -> Vec<String> {
    if !engine::is_on(config) {
        return Vec::new();
    }
    let states = state::load(&config.workspace_dir);
    let due: Vec<String> = config
        .memory
        .sources
        .iter()
        .filter(|source| super::is_due(source, states.get(&source.id), now))
        .map(|source| source.id.clone())
        .collect();
    let mut started = Vec::new();
    for id in due {
        match start_sync(config, Some(&id)) {
            Ok(ids) => started.extend(ids),
            Err(error) => {
                tracing::debug!(id = %id, code = error.code(), "[memory:sources] due sync not started")
            }
        }
    }
    started
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;
