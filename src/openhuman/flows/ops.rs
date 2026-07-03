//! Business logic for the `flows::` domain: validate-on-save CRUD plus the
//! end-to-end `flows_run` path. Delegated to from `schemas.rs`'s `handle_*`
//! RPC/CLI handlers, mirroring `src/openhuman/cron/ops.rs`.

use std::sync::Arc;

use serde_json::{json, Value};
use tinyflows::model::WorkflowGraph;

use crate::openhuman::config::Config;
use crate::openhuman::flows::store;
use crate::openhuman::flows::Flow;
use crate::rpc::RpcOutcome;

/// Overall safety bound on a single `flows_run`. Individual capabilities have
/// their own timeouts (HTTP, sandbox), but a hung LLM/tool call must never let
/// the RPC block indefinitely — this caps the whole run.
const FLOW_RUN_TIMEOUT_SECS: u64 = 600;

/// Runs a raw graph JSON value through `tinyflows::migrate::migrate` (upgrade
/// an older-schema definition to current), deserializes it, and rejects a
/// structurally invalid graph via `tinyflows::validate::validate` — so a bad
/// graph is caught at the door, before it's ever persisted.
fn validate_and_migrate_graph(graph_json: Value) -> Result<WorkflowGraph, String> {
    let migrated = tinyflows::migrate::migrate(graph_json).map_err(|e| e.to_string())?;
    let graph: WorkflowGraph = serde_json::from_value(migrated).map_err(|e| e.to_string())?;
    tinyflows::validate::validate(&graph).map_err(|e| e.to_string())?;
    Ok(graph)
}

/// Creates a new flow from a name and a raw graph JSON value.
pub async fn flows_create(
    config: &Config,
    name: String,
    graph_json: Value,
) -> Result<RpcOutcome<Flow>, String> {
    let graph = validate_and_migrate_graph(graph_json)?;
    tracing::debug!(target: "flows", %name, node_count = graph.nodes.len(), "[flows] flows_create: persisting new flow");
    let flow = store::create_flow(config, name, graph).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(flow, "flow created"))
}

/// Loads one flow by id.
pub async fn flows_get(config: &Config, id: &str) -> Result<RpcOutcome<Flow>, String> {
    let flow = store::get_flow(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{id}' not found"))?;
    Ok(RpcOutcome::single_log(flow, format!("flow loaded: {id}")))
}

/// Lists every saved flow.
pub async fn flows_list(config: &Config) -> Result<RpcOutcome<Vec<Flow>>, String> {
    let flows = store::list_flows(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(flows, "flows listed"))
}

/// Updates a flow's name and/or graph. Re-validates the graph (whether newly
/// supplied or the existing one) before persisting, same as `flows_create`.
pub async fn flows_update(
    config: &Config,
    id: &str,
    name: Option<String>,
    graph_json: Option<Value>,
) -> Result<RpcOutcome<Flow>, String> {
    let existing = store::get_flow(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{id}' not found"))?;

    let new_name = name.unwrap_or(existing.name);
    let graph = match graph_json {
        Some(raw) => validate_and_migrate_graph(raw)?,
        None => {
            tinyflows::validate::validate(&existing.graph).map_err(|e| e.to_string())?;
            existing.graph
        }
    };

    tracing::debug!(target: "flows", flow_id = %id, "[flows] flows_update: persisting changes");
    let updated =
        store::update_flow_graph(config, id, new_name, graph).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        updated,
        format!("flow updated: {id}"),
    ))
}

/// Deletes a flow by id.
pub async fn flows_delete(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    store::remove_flow(config, id).map_err(|e| e.to_string())?;
    tracing::debug!(target: "flows", flow_id = %id, "[flows] flows_delete: removed");
    Ok(RpcOutcome::new(
        json!({ "id": id, "removed": true }),
        vec![format!("flow removed: {id}")],
    ))
}

/// Enables or disables a flow. B1 note: this does not yet gate anything at
/// run time (`flows_run` still runs a disabled flow on demand, mirroring
/// `cron::rpc::cron_run`'s "Run Now always works" behavior) — `enabled` will
/// gate automatic trigger-driven dispatch once `FlowTriggerSubscriber`
/// (`src/openhuman/flows/bus.rs`) is wired up to actually invoke flows (B2).
pub async fn flows_set_enabled(
    config: &Config,
    id: &str,
    enabled: bool,
) -> Result<RpcOutcome<Flow>, String> {
    let flow = store::set_enabled(config, id, enabled).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        flow,
        format!("flow {id} enabled={enabled}"),
    ))
}

/// Runs a flow end-to-end: compile → build capabilities → durable
/// checkpointed run → record the outcome onto the flow's summary fields.
///
/// Uses `tinyflows::engine::run_with_checkpointer` (not the simpler `run`) so
/// a run that pauses at a human-in-the-loop approval gate is durably
/// checkpointed and can survive a process restart (resumed later via a
/// `flows_resume` RPC — B2+; see
/// `my_docs/ohxtf/b1-engine-seam-domain/07-execution-and-hitl.md`).
pub async fn flows_run(
    config: &Config,
    flow_id: &str,
    input: Value,
) -> Result<RpcOutcome<Value>, String> {
    let flow = store::get_flow(config, flow_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{flow_id}' not found"))?;

    // `store::get_flow` already ran the stored `graph_json` through
    // `tinyflows::migrate::migrate` before deserializing, so `flow.graph` is
    // always on the current schema here.
    let compiled = tinyflows::compiler::compile(&flow.graph).map_err(|e| e.to_string())?;

    let config_arc = Arc::new(config.clone());
    // Scope the state store per-flow so two flows never collide on a state key.
    let caps =
        crate::openhuman::tinyflows::build_capabilities(config_arc, format!("flow:{flow_id}"));
    let checkpointer =
        crate::openhuman::tinyflows::open_flow_checkpointer(config).map_err(|e| e.to_string())?;
    let thread_id = format!("flow:{flow_id}:{}", uuid::Uuid::new_v4());

    tracing::debug!(
        target: "flows",
        flow_id = %flow_id,
        thread_id = %thread_id,
        "[flows] flows_run: starting checkpointed run"
    );

    // Record a failed attempt so `last_run_at`/`last_status` reflect reality
    // (a stop-policy engine/capability failure or a timeout) rather than
    // leaving the prior success/pending state on the flow.
    let record_failed = || {
        if let Err(rec_err) = store::record_run(config, flow_id, "failed") {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                error = %rec_err,
                "[flows] flows_run: failed to record failed run"
            );
        }
    };

    let run =
        tinyflows::engine::run_with_checkpointer(&compiled, input, &caps, checkpointer, &thread_id);
    let outcome = match tokio::time::timeout(
        std::time::Duration::from_secs(FLOW_RUN_TIMEOUT_SECS),
        run,
    )
    .await
    {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(e)) => {
            record_failed();
            tracing::warn!(target: "flows", flow_id = %flow_id, error = %e, "[flows] flows_run: run failed");
            return Err(e.to_string());
        }
        Err(_elapsed) => {
            record_failed();
            tracing::warn!(target: "flows", flow_id = %flow_id, timeout_secs = FLOW_RUN_TIMEOUT_SECS, "[flows] flows_run: run timed out");
            return Err(format!("flow run timed out after {FLOW_RUN_TIMEOUT_SECS}s"));
        }
    };

    let status = if outcome.pending_approvals.is_empty() {
        "completed"
    } else {
        "pending_approval"
    };
    store::record_run(config, flow_id, status).map_err(|e| e.to_string())?;

    tracing::info!(
        target: "flows",
        flow_id = %flow_id,
        status,
        pending_approvals = outcome.pending_approvals.len(),
        "[flows] flows_run: finished"
    );

    Ok(RpcOutcome::single_log(
        json!({
            "output": outcome.output,
            "pending_approvals": outcome.pending_approvals,
            "thread_id": thread_id,
        }),
        format!("flow run {status}"),
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
