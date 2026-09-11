//! Multi-stage sub-agent delegation — the OpenHuman-facing seam onto
//! [`tinyagents_graph::delegation`] (issue #4249, #27/#28).
//!
//! The graph itself — the plan→execute⇄review→finalize state machine, its
//! revision budget, checkpoint/resume classification, durable human-approval
//! interrupt and cancellation handling — now lives upstream in the crate, where
//! it is reusable by any host and where its on-disk state shape is pinned by
//! tests. Nothing about it was OpenHuman-specific: the per-stage worker was
//! already injected, so the module named no host config, RPC, event bus or
//! security policy.
//!
//! ```text
//!   plan ─▶ execute ─▶ review ──approved/maxed──▶ finalize ─▶ END
//!             ▲                   │
//!             └─────revise────────┘
//! ```
//!
//! What stays here is this file: the historical spellings the rest of the core
//! imports, plus the one thing that genuinely is ours — the observability sink.
//!
//! # The sink is the reason these are wrappers, not plain re-exports
//!
//! Every delegation run is journalled onto OpenHuman's `tracing` diagnostics
//! through [`GraphTracingSink`](super::observability::GraphTracingSink), which
//! lives in [`super::observability`] alongside the cost catalog, the tool-status
//! classifier and the provider usage tables — host surface that must not follow
//! the graph upstream. The crate instead takes an optional
//! [`DelegationConfig::event_sink`], and the wrappers below attach ours when a
//! caller has not supplied one. Behaviour is unchanged from when the sink was
//! hard-wired into the graph builder: every run through this module is still
//! journalled under the `delegation:graph` label.
//!
//! A caller that sets `event_sink` explicitly keeps its own sink — the wrappers
//! fill a gap, they never override.
//!
//! # Layering
//!
//! Production wiring of the injected stage worker (dispatching each stage
//! through `run_subagent`, and the `SqliteCheckpointer` under the workspace) is
//! a separate concern and lives in
//! [`agent_orchestration::delegation`](crate::openhuman::agent::orchestration::delegation),
//! which depends on both this seam and `subagent_runner` — so this seam stays
//! free of orchestration dependencies.

use std::future::Future;
use std::sync::Arc;

use serde_json::Value;

use super::observability::GraphTracingSink;

// The surface the core imports today, under its historical spellings.
pub(crate) use tinyagents_graph::delegation::{
    delegation_graph_topology, DelegationConfig, DelegationOutcome, DelegationStage,
    DelegationStageOutput, DelegationState,
};

// The rest of the seam. These have no in-tree caller right now — their only
// consumers were the unit tests, which moved upstream with the graph — but they
// are kept re-exported rather than deleted so the durable-approval path
// (`resume_delegation` + `deny_decision` + `PendingApproval`) and the state
// vocabulary (`StepRecord`, `CURRENT_SCHEMA_VERSION`) are reachable under the
// same names as before, and so a caller wiring that path up finds it here — on
// the wrappers that attach the tracing sink — rather than calling the crate
// directly and silently losing the journal.
#[allow(unused_imports)]
pub(crate) use tinyagents_graph::delegation::{
    deny_decision, PendingApproval, StepRecord, CURRENT_SCHEMA_VERSION,
};

/// The `tracing` label every delegation graph run is journalled under. Stable —
/// log queries and dashboards match on it.
const GRAPH_SINK_LABEL: &str = "delegation:graph";

/// Attach OpenHuman's graph tracing sink unless the caller supplied one.
///
/// This is the single place the host's observability is bound to a delegation
/// run, so a new entry point cannot silently lose the journal.
fn with_tracing_sink(mut config: DelegationConfig) -> DelegationConfig {
    if config.event_sink.is_none() {
        config.event_sink = Some(Arc::new(GraphTracingSink::new(GRAPH_SINK_LABEL)));
    }
    config
}

/// Run the plan→execute⇄review→finalize delegation graph, invoking `run_stage`
/// for each stage. Returns the final [`DelegationState`].
///
/// See [`tinyagents_graph::delegation::run_delegation`].
#[allow(dead_code)]
pub(crate) async fn run_delegation<F, Fut>(
    config: DelegationConfig,
    run_stage: F,
) -> Result<DelegationState, String>
where
    F: Fn(DelegationStage, DelegationState) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<DelegationStageOutput, String>> + Send + 'static,
{
    tinyagents_graph::delegation::run_delegation(with_tracing_sink(config), run_stage).await
}

/// Run the delegation graph and report whether it finalized or parked on a
/// durable human-approval interrupt.
///
/// See [`tinyagents_graph::delegation::run_delegation_durable`].
#[allow(dead_code)]
pub(crate) async fn run_delegation_durable<F, Fut>(
    config: DelegationConfig,
    run_stage: F,
) -> Result<DelegationOutcome, String>
where
    F: Fn(DelegationStage, DelegationState) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<DelegationStageOutput, String>> + Send + 'static,
{
    tinyagents_graph::delegation::run_delegation_durable(with_tracing_sink(config), run_stage).await
}

/// Resume a delegation graph parked on a durable human-approval interrupt,
/// delivering the approver's `decision`.
///
/// `decision` accepts the approval RPC's stable wire values (`approve_once` /
/// `approve_always_for_tool` / `deny`), so the existing decision contract routes
/// into the resume unchanged. TTL expiry → pass [`deny_decision`].
///
/// See [`tinyagents_graph::delegation::resume_delegation`].
#[allow(dead_code)]
pub(crate) async fn resume_delegation<F, Fut>(
    config: DelegationConfig,
    decision: Value,
    run_stage: F,
) -> Result<DelegationOutcome, String>
where
    F: Fn(DelegationStage, DelegationState) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<DelegationStageOutput, String>> + Send + 'static,
{
    tinyagents_graph::delegation::resume_delegation(with_tracing_sink(config), decision, run_stage)
        .await
}

/// Run the delegation graph, resuming from the last checkpoint boundary when the
/// configured thread has a live, compatible, non-terminal checkpoint, else
/// starting fresh.
///
/// See [`tinyagents_graph::delegation::run_or_resume_delegation`].
pub(crate) async fn run_or_resume_delegation<F, Fut>(
    config: DelegationConfig,
    run_stage: F,
) -> Result<DelegationOutcome, String>
where
    F: Fn(DelegationStage, DelegationState) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<DelegationStageOutput, String>> + Send + 'static,
{
    tinyagents_graph::delegation::run_or_resume_delegation(with_tracing_sink(config), run_stage)
        .await
}

#[cfg(test)]
#[path = "delegation_tests.rs"]
mod tests;
