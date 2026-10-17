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
//! through [`GraphTracingSink`](tinyagents_graph::GraphTracingSink), which
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
//! [`agent_orchestration::delegation`](crate::agent::orchestration::delegation),
//! which depends on both this seam and `subagent_host` — so this seam stays
//! free of orchestration dependencies.

use std::future::Future;
use std::sync::Arc;

use tinyagents_graph::GraphTracingSink;

use tinyagents_graph::delegation::{
    DelegationConfig, DelegationOutcome, DelegationStage, DelegationStageOutput, DelegationState,
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

/// Run the delegation graph, resuming from the last checkpoint boundary when the
/// configured thread has a live, compatible, non-terminal checkpoint, else
/// starting fresh.
///
/// See [`tinyagents_graph::delegation::run_or_resume_delegation`].
pub(crate) async fn run_or_resume_with_tracing<F, Fut>(
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
#[path = "../delegation_tests.rs"]
mod tests;
