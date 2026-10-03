//! Document sources: the registry persisted in `[[memory.sources]]`, and sync.
//!
//! A source names something to read — a folder, a file, a web page, a GitHub
//! repository, an RSS feed or a connected Composio toolkit — and how often.
//! Sync reads it through `tinymemory-sources` (or, for Composio, the connector
//! module) and stores each item as a `Document` whose `meta.source` is
//! `{kind, id: <source id>}`, so removing a source can forget exactly its
//! items. Sync runs on demand ([`start_sync`]) and from the
//! `memory_sources_sync` cron job ([`sync_due`]).

pub mod composio;
pub mod state;
mod sync;

use chrono::{DateTime, Utc};
use tinymemory::{ForgetTarget, MetaFilter};

use crate::config::schema::{MemorySourceConfig, MemorySourceKind};
use crate::config::Config;

use super::engine;
use super::error::{MemoryError, MemoryResult};
use super::types::{SourceStatus, SourceView, SourcesAddParams};

pub use sync::{start_sync, sync_due, sync_one};

/// Fewest minutes between scheduled syncs of one source.
pub const MIN_SCHEDULE_MINS: u32 = 15;

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
/// network kinds must be http(s) URLs; a Composio target is a toolkit slug.
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
        MemorySourceKind::Composio => {
            if target
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                Ok(target.to_ascii_lowercase())
            } else {
                Err(MemoryError::invalid("a Composio target is a toolkit slug"))
            }
        }
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
            "unknown source kind `{}` (folder, file, link, github, rss, composio)",
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
    let source = MemorySourceConfig {
        id: format!("src-{}", uuid::Uuid::new_v4().simple()),
        kind,
        target,
        label,
        schedule_mins: params.schedule_mins,
    };
    sync::reader_entry(&source)?;
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

/// Forgets every item source `id` stored. Memory off is not an error here:
/// there is nothing reachable to forget.
pub async fn forget_items(config: &Config, id: &str) -> MemoryResult<usize> {
    let bound = match engine::resolve(config).engine() {
        Ok(bound) => bound,
        Err(MemoryError::Off(_)) => return Ok(0),
        Err(error) => return Err(error),
    };
    let filter = MetaFilter {
        source_id: Some(id.to_string()),
        ..MetaFilter::default()
    };
    let report = bound.engine.forget(ForgetTarget::Filter(filter)).await?;
    tracing::debug!(id = %id, forgotten = report.forgotten, "[memory:sources] items forgotten");
    Ok(report.forgotten)
}

/// Whether `source` is due for a scheduled sync at `now`.
#[must_use]
pub fn is_due(
    source: &MemorySourceConfig,
    last: Option<&state::SourceState>,
    now: DateTime<Utc>,
) -> bool {
    let Some(mins) = source.schedule_mins else {
        return false;
    };
    match last {
        Some(state) if state.status == SourceStatus::Syncing => false,
        Some(state) => state.last_sync_at.is_none_or(|at| {
            now.signed_duration_since(at)
                >= chrono::Duration::minutes(i64::from(mins.max(MIN_SCHEDULE_MINS)))
        }),
        None => true,
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
