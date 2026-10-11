//! RPC-facing operations for the `task_sources` domain.
//!
//! Each function returns an [`Outcome`] so the controller layer can
//! surface logs alongside the value. Errors are `String` to match the
//! `ControllerFuture` boundary. Business logic stays here; `schemas.rs`
//! only parses params and delegates.

use serde_json::{json, Value};

use crate::config::Config;
use crate::core::Outcome;
use crate::integrations::composio::contract::{NormalizedTask, TaskContainer};

use super::types::{
    FetchReason, FilterSpec, ProviderSlug, SourceTarget, TaskSource, TaskSourcePatch,
};
use super::{filter, pipeline, store};

/// List all configured task sources.
pub async fn list(config: &Config) -> Result<Outcome<Vec<TaskSource>>, String> {
    let sources = store::list_sources(config).map_err(|e| e.to_string())?;
    tracing::debug!(count = sources.len(), "[task_sources:ops] list");
    Ok(Outcome::new(sources, vec![]))
}

/// Fetch a single source by id.
pub async fn get(config: &Config, id: &str) -> Result<Outcome<TaskSource>, String> {
    let source = store::get_source(config, id).map_err(|e| e.to_string())?;
    Ok(Outcome::new(source, vec![]))
}

/// Create a new source. Missing schedule / target / cap fields fall back
/// to the `[task_sources]` config defaults.
pub async fn add(
    config: &Config,
    provider: ProviderSlug,
    connection_id: Option<String>,
    name: Option<String>,
    filter: FilterSpec,
    interval_secs: Option<u64>,
    target: Option<SourceTarget>,
    max_tasks_per_fetch: Option<u32>,
) -> Result<Outcome<TaskSource>, String> {
    let defaults = &config.task_sources;
    let interval_secs = interval_secs.unwrap_or(defaults.default_interval_secs);
    let max = max_tasks_per_fetch.unwrap_or(defaults.max_tasks_per_fetch);
    let target = target.unwrap_or(if defaults.auto_proactive {
        SourceTarget::AgentTodoProactive
    } else {
        SourceTarget::TodoOnly
    });

    let source = store::add_source(
        config,
        provider,
        connection_id.filter(|s| !s.trim().is_empty()),
        name.filter(|s| !s.trim().is_empty()),
        filter,
        interval_secs,
        target,
        max,
    )
    .map_err(|e| e.to_string())?;

    tracing::info!(
        source_id = %source.id,
        provider = %source.provider.as_str(),
        "[task_sources:ops] add created source"
    );
    Ok(Outcome::new(source, vec![]))
}

/// Apply a partial update to a source.
pub async fn update(
    config: &Config,
    id: &str,
    patch: TaskSourcePatch,
) -> Result<Outcome<TaskSource>, String> {
    let source = store::update_source(config, id, patch).map_err(|e| e.to_string())?;
    tracing::debug!(source_id = %id, "[task_sources:ops] update applied");
    Ok(Outcome::new(source, vec![]))
}

/// Remove a source by id.
pub async fn remove(config: &Config, id: &str) -> Result<Outcome<Value>, String> {
    let ingested = store::list_ingested_refs(config, id).map_err(|e| e.to_string())?;
    let mut pruned = 0usize;
    for item in ingested {
        if store::remove_ingested(config, id, &item.external_id).map_err(|e| e.to_string())? {
            pruned += 1;
        }
    }
    store::remove_source(config, id).map_err(|e| e.to_string())?;
    tracing::debug!(source_id = %id, pruned, "[task_sources:ops] removed");
    Ok(Outcome::new(
        json!({ "id": id, "removed": true, "pruned": pruned }),
        vec![],
    ))
}

/// Manually fetch one source now (`FetchReason::Manual`).
pub async fn fetch(config: &Config, id: &str) -> Result<Outcome<super::FetchOutcome>, String> {
    let source = store::get_source(config, id).map_err(|e| e.to_string())?;
    let outcome = pipeline::run_source_once(config, &source, FetchReason::Manual).await;
    Ok(Outcome::new(outcome, vec![]))
}

/// Manually sync all enabled task sources now.
pub async fn sync(config: &Config) -> Result<Outcome<Vec<super::FetchOutcome>>, String> {
    let sources = store::list_sources(config).map_err(|e| e.to_string())?;
    let mut outcomes = Vec::new();
    for source in sources.into_iter().filter(|source| source.enabled) {
        outcomes.push(pipeline::run_source_once(config, &source, FetchReason::Manual).await);
    }
    tracing::info!(
        source_count = outcomes.len(),
        fetched = outcomes
            .iter()
            .map(|outcome| outcome.fetched)
            .sum::<usize>(),
        routed = outcomes.iter().map(|outcome| outcome.routed).sum::<usize>(),
        pruned = outcomes.iter().map(|outcome| outcome.pruned).sum::<usize>(),
        "[task_sources:ops] sync completed"
    );
    Ok(Outcome::new(outcomes, vec![]))
}

/// Recently ingested tasks for a source (newest first).
pub async fn list_tasks(
    config: &Config,
    id: &str,
    limit: Option<usize>,
) -> Result<Outcome<Vec<NormalizedTask>>, String> {
    let limit = limit.unwrap_or(50);
    let tasks = store::list_ingested(config, id, limit).map_err(|e| e.to_string())?;
    Ok(Outcome::new(tasks, vec![]))
}

/// Dry-run a filter: fetch matching tasks WITHOUT routing or recording
/// anything. Lets the UI validate a filter before saving a source.
pub async fn preview_filter(
    config: &Config,
    provider: ProviderSlug,
    filter_spec: FilterSpec,
    connection_id: Option<String>,
    max: Option<u32>,
) -> Result<Outcome<Vec<NormalizedTask>>, String> {
    if filter_spec.provider() != provider {
        return Err(format!(
            "filter provider '{}' does not match requested provider '{}'",
            filter_spec.provider().as_str(),
            provider.as_str()
        ));
    }
    let _ = connection_id;
    let max = max.unwrap_or(config.task_sources.max_tasks_per_fetch);
    let _fetch_filter = filter::to_fetch_filter(&filter_spec, max);
    // `ComposioProvider::fetch_tasks` has no replacement — see
    // `pipeline::fetch_tasks_unavailable`'s doc comment for why.
    Err(format!(
        "task_sources preview for toolkit '{}' is unavailable: tinymemory v1.13.4 deleted \
         ComposioProvider::fetch_tasks with no replacement, and the tinyconnectors module \
         exposes no structured task-fetch surface to reimplement it against",
        provider.as_str()
    ))
}

/// List the selectable containers (today: Notion databases) a connected
/// provider exposes, so the UI can offer a picker instead of a raw-id text
/// field. Mirrors [`preview_filter`]'s context setup.
pub async fn list_databases(
    config: &Config,
    provider: ProviderSlug,
    connection_id: Option<String>,
) -> Result<Outcome<Vec<TaskContainer>>, String> {
    let _ = (config, connection_id);
    // `ComposioProvider::list_databases` has no replacement — see
    // `pipeline::fetch_tasks_unavailable`'s doc comment for why.
    Err(format!(
        "task_sources list_databases for toolkit '{}' is unavailable: tinymemory v1.13.4 \
         deleted ComposioProvider::list_databases with no replacement, and the tinyconnectors \
         module exposes no structured task-fetch surface to reimplement it against",
        provider.as_str()
    ))
}

/// Domain status: enabled flag + source counts.
pub async fn status(config: &Config) -> Result<Outcome<Value>, String> {
    let sources = store::list_sources(config).map_err(|e| e.to_string())?;
    let enabled_count = sources.iter().filter(|s| s.enabled).count();
    Ok(Outcome::new(
        json!({
            "enabled": config.task_sources.enabled,
            "defaultIntervalSecs": config.task_sources.default_interval_secs,
            "sourceCount": sources.len(),
            "enabledSourceCount": enabled_count,
        }),
        vec![],
    ))
}
