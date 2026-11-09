//! Syncing sources into memory.
//!
//! The loop (claims, schedule, reading, chunked storing) is
//! `tinymemory_integrations::sources::sync`. This file is the host side: the
//! [`SourceRegistry`] over `[[memory.sources]]` and `sources/state.rs`, the
//! placement of each item under its brain node, and the belief-build jobs for
//! the nodes a sync touched.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use chrono::{DateTime, Utc};
use tinymemory_integrations::sources::sync::{
    self as module, RegisteredSource, SourceRegistry, SyncError, SyncState, SyncStatus, SyncUpdate,
    Syncer,
};
use tinymemory_integrations::sources::SourceKind;
use tinymemory_integrations::sources::SourceStatus;

use crate::config::schema::{MemorySourceConfig, MemorySourceKind};
use crate::config::Config;
use crate::memory::engine;
use tinymemory_api::{Error, Result};

use super::state;

/// The module's source for `source`.
fn registered(source: &MemorySourceConfig) -> RegisteredSource {
    let kind = match source.kind {
        MemorySourceKind::Folder => SourceKind::Folder,
        MemorySourceKind::File => SourceKind::File,
        MemorySourceKind::Link => SourceKind::WebPage,
        MemorySourceKind::Github => SourceKind::GithubRepo,
        MemorySourceKind::Rss => SourceKind::RssFeed,
    };
    RegisteredSource {
        id: source.id.clone(),
        kind,
        target: source.target.clone(),
        label: source.label.clone(),
        schedule_mins: source.schedule_mins,
    }
}

fn host_error(error: SyncError) -> Error {
    match error {
        SyncError::Invalid(message) => Error::InvalidRequest(message),
        SyncError::Read(message) => Error::Engine(format!("reading the source failed: {message}")),
        SyncError::Engine(error) => error,
    }
}

/// Checks `source` is readable as configured, without reading it.
pub(super) fn validate(source: &MemorySourceConfig) -> Result<()> {
    module::entry_for(&registered(source))
        .map(|_| ())
        .map_err(host_error)
}

/// The registry the module's [`Syncer`] runs over: the workspace's configured
/// sources, and the state file beside them. The config slot is refreshed on
/// every call, because the syncer outlives any one config snapshot.
struct HostRegistry {
    workspace: PathBuf,
    config: Arc<Mutex<Config>>,
}

impl SourceRegistry for HostRegistry {
    fn sources(&self) -> Vec<RegisteredSource> {
        let config = self.config.lock().unwrap_or_else(PoisonError::into_inner);
        config.memory.sources.iter().map(registered).collect()
    }

    fn state(&self, id: &str) -> Option<SyncState> {
        state::load(&self.workspace).get(id).map(|state| SyncState {
            status: match state.status {
                SourceStatus::Idle => SyncStatus::Idle,
                SourceStatus::Syncing => SyncStatus::Syncing,
                SourceStatus::Error => SyncStatus::Error,
            },
            last_sync_at: state.last_sync_at,
        })
    }

    fn record(&self, id: &str, update: SyncUpdate) {
        state::update(&self.workspace, id, |state| match update {
            SyncUpdate::Started => {
                state.status = SourceStatus::Syncing;
                state.error = None;
            }
            SyncUpdate::Finished { items, at } => {
                state.status = SourceStatus::Idle;
                state.last_sync_at = Some(at);
                state.items = items;
                state.error = None;
            }
            SyncUpdate::Failed { error, at } => {
                state.status = SourceStatus::Error;
                state.last_sync_at = Some(at);
                state.error = Some(error);
            }
        });
    }
}

type Slot = (Syncer, Arc<Mutex<Config>>);

/// One syncer per workspace, so a second request for a running source starts
/// nothing.
static SYNCERS: LazyLock<Mutex<HashMap<PathBuf, Slot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn syncer_for(config: &Config) -> Syncer {
    let mut all = SYNCERS.lock().unwrap_or_else(PoisonError::into_inner);
    let (syncer, slot) = all.entry(config.workspace_dir.clone()).or_insert_with(|| {
        let slot = Arc::new(Mutex::new(config.clone()));
        let registry = HostRegistry {
            workspace: config.workspace_dir.clone(),
            config: Arc::clone(&slot),
        };
        (Syncer::new(Arc::new(registry)), slot)
    });
    *slot.lock().unwrap_or_else(PoisonError::into_inner) = config.clone();
    syncer.clone()
}

/// Reads `source` and stores what it yields. Returns the number stored.
pub async fn sync_one(config: &Config, source: &MemorySourceConfig) -> Result<u64> {
    engine::resolve(config).await.engine()?;
    run(config, source).await.map_err(host_error)
}

async fn run(config: &Config, source: &MemorySourceConfig) -> std::result::Result<u64, SyncError> {
    let bound = engine::resolve(config)
        .await
        .engine()
        .map_err(|error| SyncError::Invalid(error.to_string()))?;
    let collected = module::read_source(
        &registered(source),
        &config.action_dir,
        &crate::modules::memory_converter::converter(config),
    )
    .await?;
    let layout = super::layout_of_source(config, source);
    place_and_store(
        config,
        &bound,
        collected.items,
        (source.kind, &source.id),
        &layout,
    )
    .await
}

/// Uses the module to file `items` into `layout`'s brain, and queues its
/// belief-build jobs. Returns how many were stored.
pub(crate) async fn place_and_store(
    config: &Config,
    bound: &engine::BoundEngine,
    items: Vec<tinymemory_api::StoreItem>,
    (kind, source_id): (MemorySourceKind, &str),
    layout: &tinymemory_tools::MemoryLayout,
) -> std::result::Result<u64, SyncError> {
    let brain_source = crate::memory::brain::brain_source(kind);
    let placed = module::store_placed(bound.engine.as_ref(), source_id, items, |item| {
        crate::memory::brain::brain_node(config, layout, &brain_source, item)
    })
    .await?;
    crate::memory::lifecycle::jobs::enqueue(config, layout.root(), placed.jobs).await;
    Ok(placed.stored.stored)
}

/// `memory_sources_sync`: starts a background sync of source `id`, or of
/// every source, and returns the ids whose sync started.
pub async fn start_sync(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    id: Option<&str>,
) -> Result<Vec<String>> {
    engine::resolve(config).await.engine()?;
    let claims = syncer_for(config).start(id).map_err(host_error)?;
    let mut started = Vec::new();
    for claim in claims {
        started.push(claim.source().id.clone());
        let config = config.clone();
        ctx.spawn(async move {
            let result = match config
                .memory
                .sources
                .iter()
                .find(|source| source.id == claim.source().id)
            {
                Some(source) => run(&config, source).await,
                None => Err(SyncError::Invalid("the source was removed".to_string())),
            };
            match &result {
                Ok(items) => {
                    tracing::info!(id = %claim.source().id, items, "[memory:sources] sync finished")
                }
                Err(error) => tracing::warn!(
                    id = %claim.source().id,
                    error = %error,
                    "[memory:sources] sync failed"
                ),
            }
            syncer_for(&config).finish(claim, &result);
        });
    }
    Ok(started)
}

/// The scheduled tick: starts every source whose `schedule_mins` has elapsed.
/// Returns the ids started. Memory off starts nothing.
pub async fn sync_due(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    now: DateTime<Utc>,
) -> Vec<String> {
    if !engine::is_on(config).await {
        return Vec::new();
    }
    let mut started = Vec::new();
    for id in syncer_for(config).due(now) {
        match start_sync(ctx, config, Some(&id)).await {
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
