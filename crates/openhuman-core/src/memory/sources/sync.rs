//! Syncing sources into memory.
//!
//! Sources are read through `tinymemory-integrations`' readers
//! (`collect_items`), which turn each file, page, commit or feed entry into a
//! `Document` with its metadata filled. Every item is scrubbed and stored on
//! the bound engine; per-item failures are logged and skipped, not fatal.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Utc};
use tinymemory_integrations::sources::readers::reader_for_request;
use tinymemory_integrations::sources::{collect_items, MemorySourceEntry, SourceKind};

use crate::config::schema::{MemorySourceConfig, MemorySourceKind};
use crate::config::Config;
use crate::memory::engine::{self, BoundEngine};
use crate::memory::error::{MemoryError, MemoryResult};
use crate::memory::ops::{store_many_on, store_on};
use crate::memory::types::SourceStatus;

use super::state;

/// Sources syncing right now, per workspace, so a second request for the same
/// source does not start a parallel run.
static RUNNING: LazyLock<Mutex<HashSet<(PathBuf, String)>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// The `tinymemory-integrations` entry that reads `source`.
pub(super) fn reader_entry(source: &MemorySourceConfig) -> MemoryResult<MemorySourceEntry> {
    let kind = match source.kind {
        MemorySourceKind::Folder => SourceKind::Folder,
        MemorySourceKind::File => SourceKind::File,
        MemorySourceKind::Link => SourceKind::WebPage,
        MemorySourceKind::Github => SourceKind::GithubRepo,
        MemorySourceKind::Rss => SourceKind::RssFeed,
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
    Ok(entry)
}

/// Fills the per-kind read caps the user did not set: a GitHub repo reads at
/// most 10 PRs, 10 issues and 50 commits, a feed at most 20 items. How much a
/// sync pulls is host policy, so it lives here rather than in the readers.
fn apply_kind_defaults(entry: &mut MemorySourceEntry) {
    match entry.kind {
        SourceKind::GithubRepo => {
            entry.max_prs.get_or_insert(10);
            entry.max_issues.get_or_insert(10);
            entry.max_commits.get_or_insert(50);
        }
        SourceKind::RssFeed => {
            entry.max_items.get_or_insert(20);
        }
        _ => {}
    }
}

/// Reads `source` and stores what it yields. Returns the number stored.
pub async fn sync_one(config: &Config, source: &MemorySourceConfig) -> MemoryResult<u64> {
    let bound = engine::resolve(config).engine()?;
    let entry = reader_entry(source)?;
    let reader = reader_for_request(&entry.kind);
    let collected = collect_items(
        reader.as_ref(),
        &entry,
        &config.action_dir,
        &crate::memory::convert::converter(config),
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
    store_all(
        config,
        &bound,
        collected.items,
        (source.kind, &source.id),
        &super::layout_of_source(config, source),
    )
    .await
}

/// How many items go to the engine in one request. The contract caps a batch
/// at [`tinymemory_api::MAX_STORE_MANY`]; sitting at the cap is what makes the
/// request count `ceil(N / 100)` instead of `N`.
const STORE_CHUNK: usize = tinymemory_api::MAX_STORE_MANY;

/// Files `items` into `layout`'s brain, each under the brain source it
/// belongs to (`memory::brain::brain_node`), and queues one belief build
/// per node it touched. Returns how many were stored.
pub(crate) async fn store_all(
    config: &Config,
    bound: &BoundEngine,
    items: Vec<tinymemory_api::StoreItem>,
    (kind, source_id): (crate::config::schema::MemorySourceKind, &str),
    layout: &tinymemory_tools::MemoryLayout,
) -> MemoryResult<u64> {
    let mut stored = 0u64;
    let mut failed = 0u64;
    let mut last_error = None;
    let mut touched = std::collections::BTreeSet::new();
    let brain_source = crate::memory::brain::brain_source(kind);

    // Placing an item under its brain node is pure, so do every one up front:
    // a batch has to carry both the node (what `touched` collects, and what
    // the belief-build jobs below are keyed on) and the filed item.
    let mut placed = Vec::with_capacity(items.len());
    for item in items {
        let node = crate::memory::brain::brain_node(config, layout, &brain_source, &item)?;
        let item = crate::memory::brain::file_into(node.clone(), item);
        placed.push((node, item));
    }

    while !placed.is_empty() {
        let take = placed.len().min(STORE_CHUNK);
        let chunk: Vec<_> = placed.drain(..take).collect();
        // Cloned because a rejected batch hands nothing back, and the
        // item-by-item retry below needs the items. One clone per chunk
        // against one request per chunk instead of one per item.
        let batch: Vec<tinymemory_api::StoreItem> =
            chunk.iter().map(|(_, item)| item.clone()).collect();
        match store_many_on(bound, batch).await {
            Ok(receipts) => {
                stored += receipts.len() as u64;
                for (node, _) in chunk {
                    touched.insert(node);
                }
            }
            // Out of credits or unreachable refuses every item, so stop
            // rather than fail each one in turn.
            Err(error) if error.is_account_wide() => return Err(error),
            Err(error) => {
                // One invalid item rejects the whole batch. Skipping its
                // siblings would store fewer items than the per-item path
                // did, so retry this chunk one at a time: the accounting that
                // follows is then exactly what it was before batching.
                tracing::debug!(
                    id = %source_id,
                    code = error.code(),
                    count = chunk.len(),
                    "[memory:sources] batch store failed — retrying item by item"
                );
                for (node, item) in chunk {
                    match store_on(bound, item).await {
                        Ok(_) => {
                            stored += 1;
                            touched.insert(node);
                        }
                        Err(error) if error.is_account_wide() => return Err(error),
                        Err(error) => {
                            tracing::debug!(id = %source_id, code = error.code(), "[memory:sources] item store failed");
                            failed += 1;
                            last_error = Some(error);
                        }
                    }
                }
            }
        }
    }
    // An engine that rebuilds beliefs on its own needs no build job, as
    // `Brain::ingest` hands back none for it.
    let automatic =
        bound.engine.descriptor().consolidation == tinymemory_api::Consolidation::Automatic;
    let jobs = touched
        .into_iter()
        .filter(|_| !automatic)
        .map(|node| tinymemory_tools::BackgroundJob::BuildBeliefs {
            request: tinymemory_api::ConsolidateRequest::new(tinymemory_api::Reach::exact(node))
                .kinds([tinymemory_api::ItemKind::Document]),
        })
        .collect();
    crate::memory::lifecycle::jobs::enqueue(config, layout.root(), jobs).await;
    match last_error {
        Some(error) if stored == 0 && failed > 0 => Err(error),
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
        crate::core::runtime::spawn_scoped(async move {
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
