//! Document sources: the registry persisted in `[[memory.sources]]`, and sync.
//!
//! A source names something to read — a folder, a file, a web page, a GitHub
//! repository or an RSS feed — and how often. Sync reads it through
//! `tinymemory-integrations`' readers and stores each item as a `Document` whose `meta.source` is
//! `{kind, id: <source id>}`, so removing a source can forget exactly its
//! items. Sync runs on demand ([`start_sync`]) and from the
//! `memory_sources_sync` cron job ([`sync_due`]).

pub mod state;
mod sync;

use tinymemory_integrations::sources::SourceState;
use tinymemory_integrations::sources::{Error as SourceError, SourceSpec};
use tinymemory_tools::ops::MemoryOps;

use crate::config::schema::{MemorySourceConfig, MemorySourceKind};
use crate::config::Config;

use super::engine;
use super::types::{SourceView, SourcesAddParams};
use tinymemory_api::{Error, Result};

pub use sync::{start_sync, sync_due, sync_one};

fn source_spec_error(error: SourceError) -> Error {
    match error {
        SourceError::Invalid(message) => Error::InvalidRequest(message),
        error => Error::InvalidRequest(error.to_string()),
    }
}

/// The view of `source` with its sync state.
#[must_use]
pub fn view(source: &MemorySourceConfig, state: Option<&SourceState>) -> SourceView {
    let state = state.cloned().unwrap_or_default();
    SourceView {
        id: source.id.clone(),
        kind: source.kind,
        target: source.target.clone(),
        label: source.label.clone(),
        schedule_mins: source.schedule_mins,
        last_sync_at: state.last_sync_at,
        status: state.status,
        error: state.error,
        items: state.items,
        namespace: source.namespace.clone().unwrap_or_default(),
    }
}

/// `memory_sources_list`.
#[must_use]
pub fn list(config: &Config) -> Vec<SourceView> {
    let states = state::load(&config.workspace_dir);
    config
        .memory
        .sources
        .iter()
        .map(|source| view(source, states.get(&source.id)))
        .collect()
}

/// Normalises a target for `kind` using TinyMemory's source rules.
pub fn normalize_target(kind: MemorySourceKind, target: &str) -> Result<String> {
    SourceSpec::normalize_target(kind, target).map_err(source_spec_error)
}

/// Builds a new source from `memory_sources_add` params and appends it to
/// `config`; the caller persists `config`.
pub fn apply_add(config: &mut Config, params: &SourcesAddParams) -> Result<MemorySourceConfig> {
    let kind = MemorySourceKind::parse(&params.kind).ok_or_else(|| {
        Error::InvalidRequest(format!(
            "unknown source kind `{}` (folder, file, link, github, rss)",
            params.kind.trim()
        ))
    })?;
    let spec = SourceSpec {
        kind,
        target: params.target.clone(),
        schedule_mins: params.schedule_mins,
        namespace: params.namespace.clone(),
    };
    let target = spec
        .validate_new(&config.memory.sources)
        .map_err(source_spec_error)?;
    let label = params
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map_or_else(|| target.clone(), str::to_string);
    let namespace = spec
        .namespace()
        .map_err(source_spec_error)?
        .map(|namespace| namespace.to_string());
    let source = MemorySourceConfig {
        id: format!("src-{}", uuid::Uuid::new_v4().simple()),
        kind,
        target,
        label,
        schedule_mins: params.schedule_mins,
        namespace,
    };
    sync::validate(&source)?;
    config.memory.sources.push(source.clone());
    tracing::info!(id = %source.id, kind = kind.as_str(), "[memory:sources] source added");
    Ok(source)
}

/// Removes source `id` from `config`; the caller persists `config`. Returns
/// the removed source.
pub fn apply_remove(config: &mut Config, id: &str) -> Option<MemorySourceConfig> {
    let index = config.memory.sources.iter().position(|s| s.id == id)?;
    let removed = config.memory.sources.remove(index);
    tracing::info!(id = %removed.id, "[memory:sources] source removed");
    Some(removed)
}

/// Forgets, for good, every item source `id` stored (by `memory_ids`, with
/// an explicit `redact_events` cascade). Memory off is not an error here:
/// the deletion is queued ([`crate::memory::deletion`]) and runs on the next
/// sign-in. A failure is queued the same way, and returned.
pub async fn forget_items(config: &Config, id: &str) -> Result<usize> {
    let bound = match engine::resolve(config).await.engine() {
        Ok(bound) => Some(bound),
        Err(Error::Off(_)) => None,
        Err(error) => return Err(error),
    };
    // Source IDs span ordinary scopes; the engine's default root reach
    // excludes service sandboxes without restricting cleanup to one profile.
    let ops = MemoryOps::unconfined();
    let engine = bound.as_ref().map(|bound| bound.engine.as_ref());
    let forgotten = ops
        .forget_source(engine, id, |deletion| {
            crate::memory::deletion::enqueue(&config.workspace_dir, deletion);
        })
        .await?;
    tracing::debug!(id = %id, forgotten, "[memory:sources] items forgotten");
    Ok(forgotten)
}

/// The layout source `source` files into: its own `namespace` (a layout
/// root such as `team:acme`) when set, else the configured root.
#[must_use]
pub fn layout_of_source(
    config: &Config,
    source: &MemorySourceConfig,
) -> tinymemory_tools::MemoryLayout {
    tinymemory_integrations::sources::layout_of_source(&configured_layout(config), source)
}

/// The layout the configured source `source_id` files into; the configured
/// root for an unknown id.
#[must_use]
pub fn layout_of(config: &Config, source_id: &str) -> tinymemory_tools::MemoryLayout {
    tinymemory_integrations::sources::layout_of(
        &configured_layout(config),
        &config.memory.sources,
        source_id,
    )
}

fn configured_layout(config: &Config) -> tinymemory_tools::MemoryLayout {
    tinymemory_tools::layout::identity::Identity::root()
        .resolve(&crate::memory::scope::identity_pins(config))
        .layout
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
