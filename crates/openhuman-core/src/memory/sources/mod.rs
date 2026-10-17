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

use tinymemory_api::{ForgetTarget, MetaFilter};

use crate::config::schema::{MemorySourceConfig, MemorySourceKind};
use crate::config::Config;

use super::engine;
use super::error::{MemoryError, MemoryResult};
use super::types::{SourceView, SourcesAddParams};

pub use sync::{start_sync, sync_due, sync_one};

/// Fewest minutes between scheduled syncs of one source (the module's).
pub use tinymemory_integrations::sources::sync::MIN_SCHEDULE_MINS;

/// The view of `source` with its sync state.
#[must_use]
pub fn view(source: &MemorySourceConfig, state: Option<&state::SourceState>) -> SourceView {
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

/// Normalises a target for `kind`: GitHub accepts `owner/repo` or a URL;
/// network kinds must be http(s) URLs.
pub fn normalize_target(kind: MemorySourceKind, target: &str) -> MemoryResult<String> {
    let target = target.trim();
    if target.is_empty() {
        return Err(MemoryError::invalid("target must not be empty"));
    }
    match kind {
        MemorySourceKind::Folder | MemorySourceKind::File => Ok(target.to_string()),
        MemorySourceKind::Github => {
            if target.starts_with("http://") || target.starts_with("https://") {
                return http_url(target);
            }
            let mut parts = target.split('/');
            match (parts.next(), parts.next(), parts.next()) {
                (Some(owner), Some(repo), None) if !owner.is_empty() && !repo.is_empty() => {
                    Ok(format!("https://github.com/{owner}/{repo}"))
                }
                _ => Err(MemoryError::invalid(
                    "a GitHub target is `owner/repo` or a repository URL",
                )),
            }
        }
        MemorySourceKind::Link | MemorySourceKind::Rss => http_url(target),
    }
}

fn http_url(target: &str) -> MemoryResult<String> {
    let url = url::Url::parse(target).map_err(|_| MemoryError::invalid("target is not a URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(MemoryError::invalid("target must be an http(s) URL"));
    }
    Ok(url.to_string())
}

/// Builds a new source from `memory_sources_add` params and appends it to
/// `config`; the caller persists `config`.
pub fn apply_add(
    config: &mut Config,
    params: &SourcesAddParams,
) -> MemoryResult<MemorySourceConfig> {
    let kind = MemorySourceKind::parse(&params.kind).ok_or_else(|| {
        MemoryError::invalid(format!(
            "unknown source kind `{}` (folder, file, link, github, rss)",
            params.kind.trim()
        ))
    })?;
    let target = normalize_target(kind, &params.target)?;
    if let Some(mins) = params.schedule_mins {
        if mins < MIN_SCHEDULE_MINS {
            return Err(MemoryError::invalid(format!(
                "schedule_mins must be at least {MIN_SCHEDULE_MINS}"
            )));
        }
    }
    if config
        .memory
        .sources
        .iter()
        .any(|source| source.kind == kind && source.target == target)
    {
        return Err(MemoryError::invalid("that source is already added"));
    }
    let label = params
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map_or_else(|| target.clone(), str::to_string);
    let namespace = match params.namespace.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) => Some(
            raw.parse::<tinymemory_api::Namespace>()
                .map_err(|error| MemoryError::invalid(error.to_string()))?
                .to_string(),
        ),
    };
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
pub async fn forget_items(config: &Config, id: &str) -> MemoryResult<usize> {
    let pending = || crate::memory::deletion::PendingDeletion::Source {
        source_id: id.to_string(),
    };
    let bound = match engine::resolve(config).engine() {
        Ok(bound) => bound,
        Err(MemoryError::Off(_)) => {
            crate::memory::deletion::enqueue(&config.workspace_dir, pending());
            return Ok(0);
        }
        Err(error) => return Err(error),
    };
    let filter = MetaFilter {
        source_id: Some(id.to_string()),
        ..MetaFilter::default()
    };
    let report = match bound.engine.forget(ForgetTarget::Filter(filter)).await {
        Ok(report) => report,
        Err(error) => {
            crate::memory::deletion::enqueue(&config.workspace_dir, pending());
            return Err(error.into());
        }
    };
    tracing::debug!(id = %id, forgotten = report.forgotten, "[memory:sources] items forgotten");
    Ok(report.forgotten)
}

/// The layout source `source` files into: its own `namespace` (a layout
/// root such as `team:acme`) when set, else the configured root.
#[must_use]
pub fn layout_of_source(
    config: &Config,
    source: &MemorySourceConfig,
) -> tinymemory_tools::MemoryLayout {
    let default = || {
        crate::memory::scope::MemoryIdentity::root()
            .resolve(config)
            .layout
    };
    let Some(raw) = source
        .namespace
        .as_deref()
        .filter(|raw| !raw.trim().is_empty())
    else {
        return default();
    };
    raw.parse::<tinymemory_api::Namespace>()
        .map_err(|error| error.to_string())
        .and_then(|root| tinymemory_tools::MemoryLayout::new(root).map_err(|error| error.to_string()))
        .unwrap_or_else(|error| {
            tracing::warn!(id = %source.id, %error, "[memory:sources] invalid root; using the configured one");
            default()
        })
}

/// The layout the configured source `source_id` files into; the configured
/// root for an unknown id.
#[must_use]
pub fn layout_of(config: &Config, source_id: &str) -> tinymemory_tools::MemoryLayout {
    config
        .memory
        .sources
        .iter()
        .find(|source| source.id == source_id)
        .map_or_else(
            || {
                crate::memory::scope::MemoryIdentity::root()
                    .resolve(config)
                    .layout
            },
            |source| layout_of_source(config, source),
        )
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
