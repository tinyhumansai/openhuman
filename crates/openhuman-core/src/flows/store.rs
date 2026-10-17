//! This host's binding of the flow catalog to its workspace.
//!
//! The catalog is `tinyflows_drivers::catalog::FlowCatalogDocuments`: flows,
//! revisions, runs, state, suggestions and drafts as documents on the storage
//! ports, in the acting agent's scope of the storage backend
//! ([`crate::storage`]). tinyflows no
//! longer has a SQLite store: the `flows.db`, `checkpoints.db` and
//! `drafts/*.json` it wrote are imported once, read-only, when the backend is installed
//! ([`crate::storage::import`]).
//!
//! Every function below is that one substitution and nothing else. They are
//! spelled out rather than replaced by a `pub use` so the existing
//! `store::*(config, …)` call sites keep resolving unchanged, and so the seam
//! stays visible: anything appearing in one of these bodies beyond
//! `documents(config)` is host policy that has leaked into persistence.

use crate::config::Config;
use anyhow::Result;
use std::path::PathBuf;
use tinyflows_catalog::store::FlowUpdateError;
use tinyflows_catalog::{
    Flow, FlowRevision, FlowRun, FlowRunStep, FlowSuggestion, SuggestionStatus,
};

use tinyflows_drivers::catalog::FlowCatalogDocuments;

/// Where tinyflows kept its flow files: `<workspace_dir>/flows`
/// (`flows.db`, `checkpoints.db`, `drafts/`). Read once by the importer.
pub fn dir(config: &Config) -> PathBuf {
    config.workspace_dir.join("flows")
}

/// The storage the flow catalog and its checkpoints use for this call: the
/// storage backend in the acting scope ([`crate::storage::local::open`]).
pub(crate) fn scoped(config: &Config) -> Result<crate::storage::ScopedStorage> {
    crate::storage::local::open("flows::store", &config.workspace_dir)?.scoped()
}

/// The document catalog for this call.
pub(crate) fn documents(config: &Config) -> Result<FlowCatalogDocuments> {
    Ok(FlowCatalogDocuments::new(std::sync::Arc::clone(
        scoped(config)?.documents(),
    )))
}

/// Runs a document-catalog call from this synchronous API.
pub(crate) fn run<T: Send + 'static>(
    future: impl std::future::Future<Output = Result<T>> + Send + 'static,
) -> Result<T> {
    crate::storage::block_on_anyhow(future)
}

/// The catalog's `upsert_flow` for this call.
#[inline]
pub fn upsert_flow(config: &Config, flow: &Flow) -> Result<()> {
    let docs = documents(config)?;
    let flow = flow.clone();
    run(async move { docs.upsert_flow(&flow).await })
}

/// The catalog's `insert_duplicate_flow` for this call.
#[inline]
pub fn insert_duplicate_flow(config: &Config, source: &Flow, new_name: String) -> Result<Flow> {
    let docs = documents(config)?;
    let source = source.clone();
    run(async move { docs.insert_duplicate_flow(&source, new_name).await })
}

/// The catalog's `create_flow` for this call.
#[inline]
pub fn create_flow(
    config: &Config,
    name: String,
    graph: tinyflows::model::WorkflowGraph,
    require_approval: bool,
    enabled: bool,
) -> Result<Flow> {
    let docs = documents(config)?;
    run(async move {
        docs.create_flow(name, graph, require_approval, enabled)
            .await
    })
}

/// The catalog's `get_flow` for this call.
#[inline]
pub fn get_flow(config: &Config, id: &str) -> Result<Option<Flow>> {
    let docs = documents(config)?;
    let id = id.to_string();
    run(async move { docs.get_flow(&id).await })
}

/// The catalog's `list_flows` for this call.
#[inline]
pub fn list_flows(config: &Config) -> Result<(Vec<Flow>, usize)> {
    let docs = documents(config)?;
    run(async move { docs.list_flows().await })
}

/// The catalog's `list_enabled_flows` for this call.
#[inline]
pub fn list_enabled_flows(config: &Config) -> Result<(Vec<Flow>, usize)> {
    let docs = documents(config)?;
    run(async move { docs.list_enabled_flows().await })
}

/// The catalog's `remove_flow` for this call.
#[inline]
pub fn remove_flow(config: &Config, id: &str) -> Result<()> {
    let docs = documents(config)?;
    let id = id.to_string();
    run(async move { docs.remove_flow(&id).await })
}

/// The catalog's `set_enabled` for this call.
#[inline]
pub fn set_enabled(config: &Config, id: &str, enabled: bool) -> Result<Flow> {
    let docs = documents(config)?;
    let id = id.to_string();
    run(async move { docs.set_enabled(&id, enabled).await })
}

/// The catalog's `update_flow_graph` for this call.
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
    let docs = documents(config).map_err(FlowUpdateError::Store)?;
    let id = id.to_string();
    let expected_updated_at = expected_updated_at.map(str::to_string);
    crate::storage::block_on(async move {
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
    .map_err(|error| FlowUpdateError::Store(error.into()))?
}

/// The catalog's `list_revisions` for this call.
#[inline]
pub fn list_revisions(config: &Config, flow_id: &str, limit: usize) -> Result<Vec<FlowRevision>> {
    let docs = documents(config)?;
    let flow_id = flow_id.to_string();
    run(async move { docs.list_revisions(&flow_id, limit).await })
}

/// The catalog's `revision_by_id` for this call.
#[inline]
pub fn revision_by_id(
    config: &Config,
    flow_id: &str,
    revision_id: &str,
) -> Result<Option<FlowRevision>> {
    let docs = documents(config)?;
    let flow_id = flow_id.to_string();
    let revision_id = revision_id.to_string();
    run(async move { docs.revision_by_id(&flow_id, &revision_id).await })
}

/// The catalog's `record_run` for this call.
#[inline]
pub fn record_run(config: &Config, id: &str, status: &str) -> Result<()> {
    let docs = documents(config)?;
    let id = id.to_string();
    let status = status.to_string();
    run(async move { docs.record_run(&id, &status).await })
}

/// The catalog's `kv_get` for this call.
#[inline]
pub fn kv_get(config: &Config, namespace: &str, key: &str) -> Result<Option<serde_json::Value>> {
    let docs = documents(config)?;
    let namespace = namespace.to_string();
    let key = key.to_string();
    run(async move { docs.kv_get(&namespace, &key).await })
}

/// The catalog's `kv_set` for this call.
#[inline]
pub fn kv_set(
    config: &Config,
    namespace: &str,
    key: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let docs = documents(config)?;
    let namespace = namespace.to_string();
    let key = key.to_string();
    let value = value.clone();
    run(async move { docs.kv_set(&namespace, &key, &value).await })
}

/// The catalog's `insert_flow_run` for this call.
#[inline]
pub fn insert_flow_run(
    config: &Config,
    id: &str,
    flow_id: &str,
    thread_id: &str,
    started_at: &str,
) -> Result<()> {
    let docs = documents(config)?;
    let id = id.to_string();
    let flow_id = flow_id.to_string();
    let thread_id = thread_id.to_string();
    let started_at = started_at.to_string();
    run(async move {
        docs.insert_flow_run(&id, &flow_id, &thread_id, &started_at)
            .await
    })
}

/// The catalog's `prune_flow_runs` for this call.
#[inline]
pub fn prune_flow_runs(config: &Config, flow_id: &str, keep: usize) -> Result<usize> {
    let docs = documents(config)?;
    let flow_id = flow_id.to_string();
    run(async move { docs.prune_flow_runs(&flow_id, keep).await })
}

/// The catalog's `finish_flow_run` for this call.
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
    let docs = documents(config)?;
    let id = id.to_string();
    let status = status.to_string();
    let finished_at = finished_at.to_string();
    let steps = steps.to_vec();
    let pending_approvals = pending_approvals.to_vec();
    let error = error.map(str::to_string);
    let graph_hash = graph_hash.map(str::to_string);
    run(async move {
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
    })
}

/// The catalog's `upsert_flow_run_step` for this call.
#[inline]
pub fn upsert_flow_run_step(config: &Config, run_id: &str, step: &FlowRunStep) -> Result<()> {
    let docs = documents(config)?;
    let run_id = run_id.to_string();
    let step = step.clone();
    run(async move { docs.upsert_flow_run_step(&run_id, &step).await })
}

/// The catalog's `expire_parked_runs` for this call.
#[inline]
pub fn expire_parked_runs(
    config: &Config,
    cutoff: &str,
    now: &str,
    error_msg: &str,
) -> Result<Vec<(String, String)>> {
    let docs = documents(config)?;
    let cutoff = cutoff.to_string();
    let now = now.to_string();
    let error_msg = error_msg.to_string();
    run(async move { docs.expire_parked_runs(&cutoff, &now, &error_msg).await })
}

/// The catalog's `list_running_run_ids` for this call.
#[inline]
pub fn list_running_run_ids(
    config: &Config,
    started_before: &str,
) -> Result<Vec<(String, String)>> {
    let docs = documents(config)?;
    let started_before = started_before.to_string();
    run(async move { docs.list_running_run_ids(&started_before).await })
}

/// The catalog's `force_run_status_for_test` for this call.
///
/// Test-only: the drivers crate exposes it behind its `test-fixtures` feature,
/// which this crate turns on as a dev-dependency and never in a shipped build.
#[cfg(test)]
#[inline]
pub fn force_run_status_for_test(
    config: &Config,
    id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    let docs = documents(config)?;
    let id = id.to_string();
    let status = status.to_string();
    let error = error.map(str::to_string);
    run(async move {
        docs.force_run_status_for_test(&id, &status, error.as_deref())
            .await
    })
}

/// The catalog's `force_corrupt_graph_json_for_test` for this call.
///
/// Test-only: the drivers crate exposes it behind its `test-fixtures` feature,
/// which this crate turns on as a dev-dependency and never in a shipped build.
#[cfg(test)]
#[inline]
pub fn force_corrupt_graph_json_for_test(
    config: &Config,
    flow_id: &str,
    raw_graph_json: &str,
) -> Result<()> {
    let docs = documents(config)?;
    let flow_id = flow_id.to_string();
    let raw_graph_json = raw_graph_json.to_string();
    run(async move {
        docs.force_corrupt_graph_json_for_test(&flow_id, &raw_graph_json)
            .await
    })
}

/// The catalog's `mark_run_resuming` for this call.
#[inline]
pub fn mark_run_resuming(config: &Config, id: &str) -> Result<bool> {
    let docs = documents(config)?;
    let id = id.to_string();
    run(async move { docs.mark_run_resuming(&id).await })
}

/// The catalog's `mark_run_interrupted` for this call.
#[inline]
pub fn mark_run_interrupted(config: &Config, id: &str, now: &str, reason: &str) -> Result<bool> {
    let docs = documents(config)?;
    let id = id.to_string();
    let now = now.to_string();
    let reason = reason.to_string();
    run(async move { docs.mark_run_interrupted(&id, &now, &reason).await })
}

/// The catalog's `get_flow_run` for this call.
#[inline]
pub fn get_flow_run(config: &Config, id: &str) -> Result<Option<FlowRun>> {
    let docs = documents(config)?;
    let id = id.to_string();
    run(async move { docs.get_flow_run(&id).await })
}

/// The catalog's `list_flow_runs` for this call.
#[inline]
pub fn list_flow_runs(config: &Config, flow_id: &str, limit: usize) -> Result<Vec<FlowRun>> {
    let docs = documents(config)?;
    let flow_id = flow_id.to_string();
    run(async move { docs.list_flow_runs(&flow_id, limit).await })
}

/// The catalog's `list_all_flow_runs` for this call.
#[inline]
pub fn list_all_flow_runs(config: &Config, limit: usize) -> Result<Vec<FlowRun>> {
    let docs = documents(config)?;
    run(async move { docs.list_all_flow_runs(limit).await })
}

/// The catalog's `upsert_suggestions` for this call.
#[inline]
pub fn upsert_suggestions(config: &Config, suggestions: &[FlowSuggestion]) -> Result<usize> {
    let docs = documents(config)?;
    let suggestions = suggestions.to_vec();
    run(async move { docs.upsert_suggestions(&suggestions).await })
}

/// The catalog's `list_suggestions` for this call.
#[inline]
pub fn list_suggestions(
    config: &Config,
    status: Option<SuggestionStatus>,
    limit: usize,
) -> Result<Vec<FlowSuggestion>> {
    let docs = documents(config)?;
    run(async move { docs.list_suggestions(status, limit).await })
}

/// The catalog's `set_suggestion_status` for this call.
#[inline]
pub fn set_suggestion_status(config: &Config, id: &str, status: SuggestionStatus) -> Result<bool> {
    let docs = documents(config)?;
    let id = id.to_string();
    run(async move { docs.set_suggestion_status(&id, status).await })
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
