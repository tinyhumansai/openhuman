//! This host's binding of the flow catalog to its workspace.
//!
//! The store itself is `tinyflows_sqlite::flows` — schema, SQL, migrations and
//! concurrency all live there, take a directory, and know nothing about
//! OpenHuman. What is left here is the one fact the crate cannot know: *which*
//! directory this host keeps its catalog in.
//!
//! Every function below is that one substitution and nothing else. They are
//! spelled out rather than replaced by a `pub use` so the existing
//! `store::*(config, …)` call sites keep resolving unchanged, and so the seam
//! stays visible: anything appearing in one of these bodies beyond
//! `dir(config)` is host policy that has leaked into persistence.

use crate::openhuman::config::Config;
use anyhow::Result;
use std::path::PathBuf;
use tinyflows_catalog::{
    Flow, FlowRevision, FlowRun, FlowRunStep, FlowSuggestion, SuggestionStatus,
};

pub use tinyflows_sqlite::flows::{FlowUpdateError, MAX_FLOW_RUNS_PER_FLOW};

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
    tinyflows_sqlite::flows::upsert_flow(&dir(config), flow)
}

/// Binds [`tinyflows_sqlite::flows::insert_duplicate_flow`] to this host's catalog directory.
#[inline]
pub fn insert_duplicate_flow(config: &Config, source: &Flow, new_name: String) -> Result<Flow> {
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
    tinyflows_sqlite::flows::create_flow(&dir(config), name, graph, require_approval, enabled)
}

/// Binds [`tinyflows_sqlite::flows::get_flow`] to this host's catalog directory.
#[inline]
pub fn get_flow(config: &Config, id: &str) -> Result<Option<Flow>> {
    tinyflows_sqlite::flows::get_flow(&dir(config), id)
}

/// Binds [`tinyflows_sqlite::flows::list_flows`] to this host's catalog directory.
#[inline]
pub fn list_flows(config: &Config) -> Result<(Vec<Flow>, usize)> {
    tinyflows_sqlite::flows::list_flows(&dir(config))
}

/// Binds [`tinyflows_sqlite::flows::list_enabled_flows`] to this host's catalog directory.
#[inline]
pub fn list_enabled_flows(config: &Config) -> Result<(Vec<Flow>, usize)> {
    tinyflows_sqlite::flows::list_enabled_flows(&dir(config))
}

/// Binds [`tinyflows_sqlite::flows::remove_flow`] to this host's catalog directory.
#[inline]
pub fn remove_flow(config: &Config, id: &str) -> Result<()> {
    tinyflows_sqlite::flows::remove_flow(&dir(config), id)
}

/// Binds [`tinyflows_sqlite::flows::set_enabled`] to this host's catalog directory.
#[inline]
pub fn set_enabled(config: &Config, id: &str, enabled: bool) -> Result<Flow> {
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
    tinyflows_sqlite::flows::list_revisions(&dir(config), flow_id, limit)
}

/// Binds [`tinyflows_sqlite::flows::revision_by_id`] to this host's catalog directory.
#[inline]
pub fn revision_by_id(
    config: &Config,
    flow_id: &str,
    revision_id: &str,
) -> Result<Option<FlowRevision>> {
    tinyflows_sqlite::flows::revision_by_id(&dir(config), flow_id, revision_id)
}

/// Binds [`tinyflows_sqlite::flows::record_run`] to this host's catalog directory.
#[inline]
pub fn record_run(config: &Config, id: &str, status: &str) -> Result<()> {
    tinyflows_sqlite::flows::record_run(&dir(config), id, status)
}

/// Binds [`tinyflows_sqlite::flows::kv_get`] to this host's catalog directory.
#[inline]
pub fn kv_get(config: &Config, namespace: &str, key: &str) -> Result<Option<serde_json::Value>> {
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
    tinyflows_sqlite::flows::kv_set(&dir(config), namespace, key, value)
}

/// Binds [`tinyflows_sqlite::flows::kv_delete`] to this host's catalog directory.
#[inline]
pub fn kv_delete(config: &Config, namespace: &str, key: &str) -> Result<()> {
    tinyflows_sqlite::flows::kv_delete(&dir(config), namespace, key)
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
    tinyflows_sqlite::flows::insert_flow_run(&dir(config), id, flow_id, thread_id, started_at)
}

/// Binds [`tinyflows_sqlite::flows::prune_flow_runs`] to this host's catalog directory.
#[inline]
pub fn prune_flow_runs(config: &Config, flow_id: &str, keep: usize) -> Result<usize> {
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
    tinyflows_sqlite::flows::expire_parked_runs(&dir(config), cutoff, now, error_msg)
}

/// Binds [`tinyflows_sqlite::flows::list_running_run_ids`] to this host's catalog directory.
#[inline]
pub fn list_running_run_ids(
    config: &Config,
    started_before: &str,
) -> Result<Vec<(String, String)>> {
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
    tinyflows_sqlite::flows::force_corrupt_graph_json_for_test(
        &dir(config),
        flow_id,
        raw_graph_json,
    )
}

/// Binds [`tinyflows_sqlite::flows::mark_run_resuming`] to this host's catalog directory.
#[inline]
pub fn mark_run_resuming(config: &Config, id: &str) -> Result<bool> {
    tinyflows_sqlite::flows::mark_run_resuming(&dir(config), id)
}

/// Binds [`tinyflows_sqlite::flows::mark_run_interrupted`] to this host's catalog directory.
#[inline]
pub fn mark_run_interrupted(config: &Config, id: &str, now: &str, reason: &str) -> Result<bool> {
    tinyflows_sqlite::flows::mark_run_interrupted(&dir(config), id, now, reason)
}

/// Binds [`tinyflows_sqlite::flows::get_flow_run`] to this host's catalog directory.
#[inline]
pub fn get_flow_run(config: &Config, id: &str) -> Result<Option<FlowRun>> {
    tinyflows_sqlite::flows::get_flow_run(&dir(config), id)
}

/// Binds [`tinyflows_sqlite::flows::list_flow_runs`] to this host's catalog directory.
#[inline]
pub fn list_flow_runs(config: &Config, flow_id: &str, limit: usize) -> Result<Vec<FlowRun>> {
    tinyflows_sqlite::flows::list_flow_runs(&dir(config), flow_id, limit)
}

/// Binds [`tinyflows_sqlite::flows::list_all_flow_runs`] to this host's catalog directory.
#[inline]
pub fn list_all_flow_runs(config: &Config, limit: usize) -> Result<Vec<FlowRun>> {
    tinyflows_sqlite::flows::list_all_flow_runs(&dir(config), limit)
}

/// Binds [`tinyflows_sqlite::flows::upsert_suggestions`] to this host's catalog directory.
#[inline]
pub fn upsert_suggestions(config: &Config, suggestions: &[FlowSuggestion]) -> Result<usize> {
    tinyflows_sqlite::flows::upsert_suggestions(&dir(config), suggestions)
}

/// Binds [`tinyflows_sqlite::flows::list_suggestions`] to this host's catalog directory.
#[inline]
pub fn list_suggestions(
    config: &Config,
    status: Option<SuggestionStatus>,
    limit: usize,
) -> Result<Vec<FlowSuggestion>> {
    tinyflows_sqlite::flows::list_suggestions(&dir(config), status, limit)
}

/// Binds [`tinyflows_sqlite::flows::set_suggestion_status`] to this host's catalog directory.
#[inline]
pub fn set_suggestion_status(config: &Config, id: &str, status: SuggestionStatus) -> Result<bool> {
    tinyflows_sqlite::flows::set_suggestion_status(&dir(config), id, status)
}
