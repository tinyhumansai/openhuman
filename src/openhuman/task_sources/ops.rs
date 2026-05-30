//! RPC-facing operations for the `task_sources` domain.
//!
//! Each function returns an [`RpcOutcome`] so the controller layer can
//! surface logs alongside the value. Errors are `String` to match the
//! `ControllerFuture` boundary. Business logic stays here; `schemas.rs`
//! only parses params and delegates.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::memory_sync::composio::providers::{
    get_provider, NormalizedTask, ProviderContext,
};
use crate::rpc::RpcOutcome;

use super::types::{
    FetchReason, FilterSpec, ProviderSlug, SourceTarget, TaskSource, TaskSourcePatch,
};
use super::{filter, pipeline, store};

/// List all configured task sources.
pub async fn list(config: &Config) -> Result<RpcOutcome<Vec<TaskSource>>, String> {
    let sources = store::list_sources(config).map_err(|e| e.to_string())?;
    tracing::debug!(count = sources.len(), "[task_sources:ops] list");
    Ok(RpcOutcome::new(sources, vec![]))
}

/// Fetch a single source by id.
pub async fn get(config: &Config, id: &str) -> Result<RpcOutcome<TaskSource>, String> {
    let source = store::get_source(config, id).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::new(source, vec![]))
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
    assigned_executor: Option<String>,
) -> Result<RpcOutcome<TaskSource>, String> {
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

    // Apply the optional static executor routing (G7) as a follow-up patch so
    // `add_source`'s signature (and its many callers) stays unchanged.
    let source = match assigned_executor.filter(|s| !s.trim().is_empty()) {
        Some(executor) => store::update_source(
            config,
            &source.id,
            TaskSourcePatch {
                assigned_executor: Some(executor),
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?,
        None => source,
    };

    tracing::info!(
        source_id = %source.id,
        provider = %source.provider.as_str(),
        assigned_executor = ?source.assigned_executor,
        "[task_sources:ops] add created source"
    );
    Ok(RpcOutcome::new(source, vec![]))
}

/// Apply a partial update to a source.
pub async fn update(
    config: &Config,
    id: &str,
    patch: TaskSourcePatch,
) -> Result<RpcOutcome<TaskSource>, String> {
    let source = store::update_source(config, id, patch).map_err(|e| e.to_string())?;
    tracing::debug!(source_id = %id, "[task_sources:ops] update applied");
    Ok(RpcOutcome::new(source, vec![]))
}

/// Remove a source by id.
pub async fn remove(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    store::remove_source(config, id).map_err(|e| e.to_string())?;
    tracing::debug!(source_id = %id, "[task_sources:ops] removed");
    Ok(RpcOutcome::new(
        json!({ "id": id, "removed": true }),
        vec![],
    ))
}

/// Manually fetch one source now (`FetchReason::Manual`).
pub async fn fetch(config: &Config, id: &str) -> Result<RpcOutcome<super::FetchOutcome>, String> {
    let source = store::get_source(config, id).map_err(|e| e.to_string())?;
    let outcome = pipeline::run_source_once(config, &source, FetchReason::Manual).await;
    Ok(RpcOutcome::new(outcome, vec![]))
}

/// Recently ingested tasks for a source (newest first).
pub async fn list_tasks(
    config: &Config,
    id: &str,
    limit: Option<usize>,
) -> Result<RpcOutcome<Vec<NormalizedTask>>, String> {
    let limit = limit.unwrap_or(50);
    let tasks = store::list_ingested(config, id, limit).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::new(tasks, vec![]))
}

/// Dry-run a filter: fetch matching tasks WITHOUT routing or recording
/// anything. Lets the UI validate a filter before saving a source.
pub async fn preview_filter(
    config: &Config,
    provider: ProviderSlug,
    filter_spec: FilterSpec,
    connection_id: Option<String>,
    max: Option<u32>,
) -> Result<RpcOutcome<Vec<NormalizedTask>>, String> {
    if filter_spec.provider() != provider {
        return Err(format!(
            "filter provider '{}' does not match requested provider '{}'",
            filter_spec.provider().as_str(),
            provider.as_str()
        ));
    }
    let provider_impl = get_provider(provider.as_str())
        .ok_or_else(|| format!("no native provider registered for '{}'", provider.as_str()))?;
    let ctx = ProviderContext {
        config: Arc::new(config.clone()),
        toolkit: provider.as_str().to_string(),
        connection_id: connection_id.filter(|s| !s.trim().is_empty()),
    };
    let max = max.unwrap_or(config.task_sources.max_tasks_per_fetch);
    let fetch_filter = filter::to_fetch_filter(&filter_spec, max);
    let tasks = provider_impl
        .fetch_tasks(&ctx, &fetch_filter)
        .await
        .map_err(|e| format!("preview fetch failed: {e}"))?;
    tracing::debug!(count = tasks.len(), "[task_sources:ops] preview_filter");
    Ok(RpcOutcome::new(tasks, vec![]))
}

/// Domain status: enabled flag + source counts.
pub async fn status(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let sources = store::list_sources(config).map_err(|e| e.to_string())?;
    let enabled_count = sources.iter().filter(|s| s.enabled).count();
    Ok(RpcOutcome::new(
        json!({
            "enabled": config.task_sources.enabled,
            "defaultIntervalSecs": config.task_sources.default_interval_secs,
            "sourceCount": sources.len(),
            "enabledSourceCount": enabled_count,
        }),
        vec![],
    ))
}
