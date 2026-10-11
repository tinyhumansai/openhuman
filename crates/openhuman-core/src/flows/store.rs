//! This host's binding of the flow catalog to its workspace.
//!
//! The store itself is `tinyflows_sqlite::flows` — schema, SQL, migrations and
//! concurrency all live there, take a directory, and know nothing about
//! OpenHuman. What is left here is the one fact the crate cannot know: *which*
//! directory this host keeps its catalog in.
//!
//! With a storage backend configured ([`crate::storage`]) each function is
//! served by `tinyflows_drivers::catalog::FlowCatalogDocuments` instead, in
//! the acting agent's scope — the same catalog on the document port.
//!
//! Every function below is that one substitution and nothing else. They are
//! spelled out rather than replaced by a `pub use` so the existing
//! `store::*(config, …)` call sites keep resolving unchanged, and so the seam
//! stays visible: anything appearing in one of these bodies beyond
//! `dir(config)` is host policy that has leaked into persistence.

use crate::config::Config;
use anyhow::Result;
use std::path::PathBuf;
use tinyflows_catalog::{
    Flow, FlowRevision, FlowRun, FlowRunStep, FlowSuggestion, SuggestionStatus,
};

use tinyflows_drivers::catalog::FlowCatalogDocuments;
use tinyflows_sqlite::flows::FlowUpdateError;

/// The document catalog for this call when the host configured a storage
/// backend ([`crate::storage`]), in the acting agent's scope.
pub(crate) fn documents(_config: &Config) -> Result<Option<FlowCatalogDocuments>> {
    Ok(crate::storage::current_scoped()?
        .map(|scoped| FlowCatalogDocuments::new(std::sync::Arc::clone(scoped.documents()))))
}

/// Runs a document-catalog call from this synchronous API.
pub(crate) fn run<T: Send + 'static>(
    future: impl std::future::Future<Output = Result<T>> + Send + 'static,
) -> Result<T> {
    crate::storage::block_on_anyhow(future)
}

/// Where this host keeps the flow catalog: `<workspace_dir>/flows`.
///
/// `flows.db`, `checkpoints.db` and the `drafts/` directory are all created
/// under it by the crate on first use.
pub fn dir(config: &Config) -> PathBuf {
    config.workspace_dir.join("flows")
}

/// Binds [`tinyflows_sqlite::flows::upsert_flow`] to this host's catalog directory.
#[inline]
pub fn upsert_flow(config: &Config, flow: &Flow) -> Result<()> {
    if let Some(docs) = documents(config)? {
        let flow = flow.clone();
        return run(async move { docs.upsert_flow(&flow).await });
    }
    tinyflows_sqlite::flows::upsert_flow(&dir(config), flow)
}

/// Binds [`tinyflows_sqlite::flows::insert_duplicate_flow`] to this host's catalog directory.
#[inline]
pub fn insert_duplicate_flow(config: &Config, source: &Flow, new_name: String) -> Result<Flow> {
    if let Some(docs) = documents(config)? {
        let source = source.clone();
        return run(async move { docs.insert_duplicate_flow(&source, new_name).await });
    }
    tinyflows_sqlite::flows::insert_duplicate_flow(&dir(config), source, new_name)
}

/// Binds [`tinyflows_sqlite::flows::create_flow`] to this host's catalog directory.
#[inline]
pub fn create_flow(
    config: &Config,
    name: String,
    graph: tinyflows::model::WorkflowGraph,
    require_approval: bool,
    enabled: bool,
) -> Result<Flow> {
    if let Some(docs) = documents(config)? {
        return run(async move {
            docs.create_flow(name, graph, require_approval, enabled)
                .await
        });
    }
    tinyflows_sqlite::flows::create_flow(&dir(config), name, graph, require_approval, enabled)
}

/// Binds [`tinyflows_sqlite::flows::get_flow`] to this host's catalog directory.
#[inline]
pub fn get_flow(config: &Config, id: &str) -> Result<Option<Flow>> {
    if let Some(docs) = documents(config)? {
        let id = id.to_string();
        return run(async move { docs.get_flow(&id).await });
    }
    tinyflows_sqlite::flows::get_flow(&dir(config), id)
}

/// Binds [`tinyflows_sqlite::flows::list_flows`] to this host's catalog directory.
#[inline]
pub fn list_flows(config: &Config) -> Result<(Vec<Flow>, usize)> {
    if let Some(docs) = documents(config)? {
        return run(async move { docs.list_flows().await });
    }
    tinyflows_sqlite::flows::list_flows(&dir(config))
}

/// Binds [`tinyflows_sqlite::flows::list_enabled_flows`] to this host's catalog directory.
#[inline]
pub fn list_enabled_flows(config: &Config) -> Result<(Vec<Flow>, usize)> {
    if let Some(docs) = documents(config)? {
        return run(async move { docs.list_enabled_flows().await });
    }
    tinyflows_sqlite::flows::list_enabled_flows(&dir(config))
}

/// Binds [`tinyflows_sqlite::flows::remove_flow`] to this host's catalog directory.
#[inline]
pub fn remove_flow(config: &Config, id: &str) -> Result<()> {
    if let Some(docs) = documents(config)? {
        let id = id.to_string();
        return run(async move { docs.remove_flow(&id).await });
    }
    tinyflows_sqlite::flows::remove_flow(&dir(config), id)
}

/// Binds [`tinyflows_sqlite::flows::set_enabled`] to this host's catalog directory.
#[inline]
pub fn set_enabled(config: &Config, id: &str, enabled: bool) -> Result<Flow> {
    if let Some(docs) = documents(config)? {
        let id = id.to_string();
        return run(async move { docs.set_enabled(&id, enabled).await });
    }
    tinyflows_sqlite::flows::set_enabled(&dir(config), id, enabled)
}

/// Binds [`tinyflows_sqlite::flows::update_flow_graph`] to this host's catalog directory.
#[inline]
pub fn update_flow_graph(
    config: &Config,
    id: &str,
    name: String,
    graph: tinyflows::model::WorkflowGraph,
    require_approval: bool,
    enabled_override: Option<bool>,
    force_disarm_if_automatic: bool,
    expected_updated_at: Option<&str>,
) -> std::result::Result<Flow, FlowUpdateError> {
    if let Some(docs) = documents(config).map_err(FlowUpdateError::Store)? {
        let id = id.to_string();
        let expected_updated_at = expected_updated_at.map(str::to_string);
        return crate::storage::block_on(async move {
            Ok(docs
                .update_flow_graph(
                    &id,
                    name,
                    graph,
                    require_approval,
                    enabled_override,
                    force_disarm_if_automatic,
                    expected_updated_at.as_deref(),
                )
                .await)
        })
        .map_err(|error| FlowUpdateError::Store(error.into()))?;
    }
    tinyflows_sqlite::flows::update_flow_graph(
        &dir(config),
        id,
        name,
        graph,
        require_approval,
        enabled_override,
        force_disarm_if_automatic,
        expected_updated_at,
    )
}

/// Binds [`tinyflows_sqlite::flows::list_revisions`] to this host's catalog directory.
#[inline]
pub fn list_revisions(config: &Config, flow_id: &str, limit: usize) -> Result<Vec<FlowRevision>> {
    if let Some(docs) = documents(config)? {
        let flow_id = flow_id.to_string();
        return run(async move { docs.list_revisions(&flow_id, limit).await });
    }
    tinyflows_sqlite::flows::list_revisions(&dir(config), flow_id, limit)
}

/// Binds [`tinyflows_sqlite::flows::revision_by_id`] to this host's catalog directory.
#[inline]
pub fn revision_by_id(
    config: &Config,
    flow_id: &str,
    revision_id: &str,
) -> Result<Option<FlowRevision>> {
    if let Some(docs) = documents(config)? {
        let flow_id = flow_id.to_string();
        let revision_id = revision_id.to_string();
        return run(async move { docs.revision_by_id(&flow_id, &revision_id).await });
    }
    tinyflows_sqlite::flows::revision_by_id(&dir(config), flow_id, revision_id)
}

/// Binds [`tinyflows_sqlite::flows::record_run`] to this host's catalog directory.
#[inline]
pub fn record_run(config: &Config, id: &str, status: &str) -> Result<()> {
    if let Some(docs) = documents(config)? {
        let id = id.to_string();
        let status = status.to_string();
        return run(async move { docs.record_run(&id, &status).await });
    }
    tinyflows_sqlite::flows::record_run(&dir(config), id, status)
}

/// Binds [`tinyflows_sqlite::flows::kv_get`] to this host's catalog directory.
#[inline]
pub fn kv_get(config: &Config, namespace: &str, key: &str) -> Result<Option<serde_json::Value>> {
    if let Some(docs) = documents(config)? {
        let namespace = namespace.to_string();
        let key = key.to_string();
        return run(async move { docs.kv_get(&namespace, &key).await });
    }
    tinyflows_sqlite::flows::kv_get(&dir(config), namespace, key)
}

/// Binds [`tinyflows_sqlite::flows::kv_set`] to this host's catalog directory.
#[inline]
pub fn kv_set(
    config: &Config,
    namespace: &str,
    key: &str,
    value: &serde_json::Value,
) -> Result<()> {
    if let Some(docs) = documents(config)? {
        let namespace = namespace.to_string();
        let key = key.to_string();
        let value = value.clone();
        return run(async move { docs.kv_set(&namespace, &key, &value).await });
    }
    tinyflows_sqlite::flows::kv_set(&dir(config), namespace, key, value)
}

/// Binds [`tinyflows_sqlite::flows::insert_flow_run`] to this host's catalog directory.
#[inline]
pub fn insert_flow_run(
    config: &Config,
    id: &str,
    flow_id: &str,
    thread_id: &str,
    started_at: &str,
) -> Result<()> {
    if let Some(docs) = documents(config)? {
        let id = id.to_string();
        let flow_id = flow_id.to_string();
        let thread_id = thread_id.to_string();
        let started_at = started_at.to_string();
        return run(async move {
            docs.insert_flow_run(&id, &flow_id, &thread_id, &started_at)
                .await
        });
    }
    tinyflows_sqlite::flows::insert_flow_run(&dir(config), id, flow_id, thread_id, started_at)
}

/// Binds [`tinyflows_sqlite::flows::prune_flow_runs`] to this host's catalog directory.
#[inline]
pub fn prune_flow_runs(config: &Config, flow_id: &str, keep: usize) -> Result<usize> {
    if let Some(docs) = documents(config)? {
        let flow_id = flow_id.to_string();
        return run(async move { docs.prune_flow_runs(&flow_id, keep).await });
    }
    tinyflows_sqlite::flows::prune_flow_runs(&dir(config), flow_id, keep)
}

/// Binds [`tinyflows_sqlite::flows::finish_flow_run`] to this host's catalog directory.
#[inline]
pub fn finish_flow_run(
    config: &Config,
    id: &str,
    status: &str,
    finished_at: &str,
    steps: &[FlowRunStep],
    pending_approvals: &[String],
    error: Option<&str>,
    graph_hash: Option<&str>,
) -> Result<bool> {
    if let Some(docs) = documents(config)? {
        let id = id.to_string();
        let status = status.to_string();
        let finished_at = finished_at.to_string();
        let steps = steps.to_vec();
        let pending_approvals = pending_approvals.to_vec();
        let error = error.map(str::to_string);
        let graph_hash = graph_hash.map(str::to_string);
        return run(async move {
            docs.finish_flow_run(
                &id,
                &status,
                &finished_at,
                &steps,
                &pending_approvals,
                error.as_deref(),
                graph_hash.as_deref(),
            )
            .await
        });
    }
    tinyflows_sqlite::flows::finish_flow_run(
        &dir(config),
        id,
        status,
        finished_at,
        steps,
        pending_approvals,
        error,
        graph_hash,
    )
}

/// Binds [`tinyflows_sqlite::flows::upsert_flow_run_step`] to this host's catalog directory.
#[inline]
pub fn upsert_flow_run_step(config: &Config, run_id: &str, step: &FlowRunStep) -> Result<()> {
    if let Some(docs) = documents(config)? {
        let run_id = run_id.to_string();
        let step = step.clone();
        return run(async move { docs.upsert_flow_run_step(&run_id, &step).await });
    }
    tinyflows_sqlite::flows::upsert_flow_run_step(&dir(config), run_id, step)
}

/// Binds [`tinyflows_sqlite::flows::expire_parked_runs`] to this host's catalog directory.
#[inline]
pub fn expire_parked_runs(
    config: &Config,
    cutoff: &str,
    now: &str,
    error_msg: &str,
) -> Result<Vec<(String, String)>> {
    if let Some(docs) = documents(config)? {
        let cutoff = cutoff.to_string();
        let now = now.to_string();
        let error_msg = error_msg.to_string();
        return run(async move { docs.expire_parked_runs(&cutoff, &now, &error_msg).await });
    }
    tinyflows_sqlite::flows::expire_parked_runs(&dir(config), cutoff, now, error_msg)
}

/// Binds [`tinyflows_sqlite::flows::list_running_run_ids`] to this host's catalog directory.
#[inline]
pub fn list_running_run_ids(
    config: &Config,
    started_before: &str,
) -> Result<Vec<(String, String)>> {
    if let Some(docs) = documents(config)? {
        let started_before = started_before.to_string();
        return run(async move { docs.list_running_run_ids(&started_before).await });
    }
    tinyflows_sqlite::flows::list_running_run_ids(&dir(config), started_before)
}

/// Binds [`tinyflows_sqlite::flows::force_run_status_for_test`] to this host's catalog directory.
///
/// Test-only: the crate exposes it behind its `test-fixtures` feature, which
/// this crate turns on as a dev-dependency and never in a shipped build.
#[cfg(test)]
#[inline]
pub fn force_run_status_for_test(
    config: &Config,
    id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    if let Some(docs) = documents(config)? {
        let id = id.to_string();
        let status = status.to_string();
        let error = error.map(str::to_string);
        return run(async move {
            docs.force_run_status_for_test(&id, &status, error.as_deref())
                .await
        });
    }
    tinyflows_sqlite::flows::force_run_status_for_test(&dir(config), id, status, error)
}

/// Binds [`tinyflows_sqlite::flows::force_corrupt_graph_json_for_test`] to this host's catalog directory.
///
/// Test-only: the crate exposes it behind its `test-fixtures` feature, which
/// this crate turns on as a dev-dependency and never in a shipped build.
#[cfg(test)]
#[inline]
pub fn force_corrupt_graph_json_for_test(
    config: &Config,
    flow_id: &str,
    raw_graph_json: &str,
) -> Result<()> {
    if let Some(docs) = documents(config)? {
        let flow_id = flow_id.to_string();
        let raw_graph_json = raw_graph_json.to_string();
        return run(async move {
            docs.force_corrupt_graph_json_for_test(&flow_id, &raw_graph_json)
                .await
        });
    }
    tinyflows_sqlite::flows::force_corrupt_graph_json_for_test(
        &dir(config),
        flow_id,
        raw_graph_json,
    )
}

/// Binds [`tinyflows_sqlite::flows::mark_run_resuming`] to this host's catalog directory.
#[inline]
pub fn mark_run_resuming(config: &Config, id: &str) -> Result<bool> {
    if let Some(docs) = documents(config)? {
        let id = id.to_string();
        return run(async move { docs.mark_run_resuming(&id).await });
    }
    tinyflows_sqlite::flows::mark_run_resuming(&dir(config), id)
}

/// Binds [`tinyflows_sqlite::flows::mark_run_interrupted`] to this host's catalog directory.
#[inline]
pub fn mark_run_interrupted(config: &Config, id: &str, now: &str, reason: &str) -> Result<bool> {
    if let Some(docs) = documents(config)? {
        let id = id.to_string();
        let now = now.to_string();
        let reason = reason.to_string();
        return run(async move { docs.mark_run_interrupted(&id, &now, &reason).await });
    }
    tinyflows_sqlite::flows::mark_run_interrupted(&dir(config), id, now, reason)
}

/// Binds [`tinyflows_sqlite::flows::get_flow_run`] to this host's catalog directory.
#[inline]
pub fn get_flow_run(config: &Config, id: &str) -> Result<Option<FlowRun>> {
    if let Some(docs) = documents(config)? {
        let id = id.to_string();
        return run(async move { docs.get_flow_run(&id).await });
    }
    tinyflows_sqlite::flows::get_flow_run(&dir(config), id)
}

/// Binds [`tinyflows_sqlite::flows::list_flow_runs`] to this host's catalog directory.
#[inline]
pub fn list_flow_runs(config: &Config, flow_id: &str, limit: usize) -> Result<Vec<FlowRun>> {
    if let Some(docs) = documents(config)? {
        let flow_id = flow_id.to_string();
        return run(async move { docs.list_flow_runs(&flow_id, limit).await });
    }
    tinyflows_sqlite::flows::list_flow_runs(&dir(config), flow_id, limit)
}

/// Binds [`tinyflows_sqlite::flows::list_all_flow_runs`] to this host's catalog directory.
#[inline]
pub fn list_all_flow_runs(config: &Config, limit: usize) -> Result<Vec<FlowRun>> {
    if let Some(docs) = documents(config)? {
        return run(async move { docs.list_all_flow_runs(limit).await });
    }
    tinyflows_sqlite::flows::list_all_flow_runs(&dir(config), limit)
}

/// Binds [`tinyflows_sqlite::flows::upsert_suggestions`] to this host's catalog directory.
#[inline]
pub fn upsert_suggestions(config: &Config, suggestions: &[FlowSuggestion]) -> Result<usize> {
    if let Some(docs) = documents(config)? {
        let suggestions = suggestions.to_vec();
        return run(async move { docs.upsert_suggestions(&suggestions).await });
    }
    tinyflows_sqlite::flows::upsert_suggestions(&dir(config), suggestions)
}

/// Binds [`tinyflows_sqlite::flows::list_suggestions`] to this host's catalog directory.
#[inline]
pub fn list_suggestions(
    config: &Config,
    status: Option<SuggestionStatus>,
    limit: usize,
) -> Result<Vec<FlowSuggestion>> {
    if let Some(docs) = documents(config)? {
        return run(async move { docs.list_suggestions(status, limit).await });
    }
    tinyflows_sqlite::flows::list_suggestions(&dir(config), status, limit)
}

/// Binds [`tinyflows_sqlite::flows::set_suggestion_status`] to this host's catalog directory.
#[inline]
pub fn set_suggestion_status(config: &Config, id: &str, status: SuggestionStatus) -> Result<bool> {
    if let Some(docs) = documents(config)? {
        let id = id.to_string();
        return run(async move { docs.set_suggestion_status(&id, status).await });
    }
    tinyflows_sqlite::flows::set_suggestion_status(&dir(config), id, status)
}
