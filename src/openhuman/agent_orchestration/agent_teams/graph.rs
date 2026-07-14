//! Team-member execution as a conditional-routing `tinyagents` graph (#4249, B2).
//!
//! This is the `agent_teams` folder's `graph.rs` per the per-folder graph
//! convention: the folder's tinyagents graph definition (member-run state
//! machine) lives here; `runtime.rs` drives it.
//!
//! A teammate's live run is a small state machine: **execute** the worker
//! sub-agent, then route on its terminal outcome to **complete** (quality-gate +
//! idle) or **fail** (release + idle + record), joining at a single **done**
//! finish node. Historically this was a hand-rolled `match` in
//! [`super::runtime::drive_member`]; here it is a real `tinyagents`
//! [`CompiledGraph`] with command-routing:
//!
//! ```text
//!              ┌─ complete ─┐
//!   execute ──►┤            ├─► done (finish)
//!              └─ fail ─────┘
//! ```
//!
//! The worker run and the two reconciliation effects are **injected** as
//! closures so the graph mechanics (conditional routing + the engine-error
//! propagation contract) are unit-testable with trivial stubs while production
//! passes the real `spawn_agent`/`wait_agents` + run-ledger writes.
//!
//! Error contract (preserved from the legacy `drive_member`): `run_worker`
//! returns `Err` only for engine-internal failures (spawn/wait) — those
//! propagate out of the graph so the caller releases the task and idles the
//! member. A worker that *ran* but did not complete is a normal `Failed`
//! outcome handled by `on_failed`, which returns `Ok`.

use std::future::Future;
use std::sync::Arc;

use tinyagents::graph::export::GraphTopology;
use tinyagents::graph::{
    ClosureStateReducer, Command, CompiledGraph, GraphBuilder, NodeContext, NodeResult,
};

use crate::openhuman::tinyagents::observability::GraphTracingSink;

/// Lift an injected effect's `anyhow` error into the graph's error type so it
/// fails the run (and propagates back out via [`run_member_execution_graph`]).
fn graph_err(e: anyhow::Error) -> tinyagents::TinyAgentsError {
    tinyagents::TinyAgentsError::Graph(e.to_string())
}

/// Terminal classification of a teammate worker run, produced by the `execute`
/// node and used to route to `complete` or `fail`.
pub(super) enum MemberOutcome {
    /// The worker completed; `output` is its result summary (completion
    /// evidence).
    Completed { output: String },
    /// The worker ran but did not complete (failed / cancelled / closed /
    /// defensively, non-terminal); `reason` explains why.
    Failed { reason: String },
}

/// Typed state threaded through the member graph: carries the routed payload
/// (worker output on the complete path, failure reason on the fail path) so the
/// terminal node can run the matching reconciliation.
#[derive(Clone, Default)]
struct MemberState {
    payload: Option<String>,
}

/// Reducer update emitted by the member graph nodes.
enum MemberUpdate {
    /// Store the routed payload (output or reason).
    Payload(String),
    /// Terminal node fired; no state change.
    Noop,
}

/// Drive a team member's execution on the conditional-routing graph above.
///
/// Returns `Ok(())` once the member reached a reconciled terminal node, or `Err`
/// when `run_worker` (or a reconciliation closure) failed with an
/// engine-internal error — the caller maps that to release-task + idle-member.
pub(super) async fn run_member_execution_graph<W, WF, C, CF, F, FF>(
    label: &str,
    run_worker: W,
    on_complete: C,
    on_failed: F,
) -> anyhow::Result<()>
where
    W: Fn() -> WF + Clone + Send + Sync + 'static,
    WF: Future<Output = anyhow::Result<MemberOutcome>> + Send + 'static,
    C: Fn(String) -> CF + Clone + Send + Sync + 'static,
    CF: Future<Output = anyhow::Result<()>> + Send + 'static,
    F: Fn(String) -> FF + Clone + Send + Sync + 'static,
    FF: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let graph = build_member_graph(run_worker, on_complete, on_failed)?
        .with_event_sink(Arc::new(GraphTracingSink::new(label.to_string())));

    tracing::debug!(
        target: "orchestration",
        label,
        "[orchestration] driving team member execution on tinyagents graph"
    );

    graph
        .run(MemberState::default())
        .await
        .map_err(|e| anyhow::anyhow!("member graph run failed: {e}"))?;
    Ok(())
}

/// Build (but do not run) the member-execution `CompiledGraph`. Shared by
/// [`run_member_execution_graph`] and [`member_graph_topology`] so the graph's
/// structure has one definition.
fn build_member_graph<W, WF, C, CF, F, FF>(
    run_worker: W,
    on_complete: C,
    on_failed: F,
) -> anyhow::Result<CompiledGraph<MemberState, MemberUpdate>>
where
    W: Fn() -> WF + Clone + Send + Sync + 'static,
    WF: Future<Output = anyhow::Result<MemberOutcome>> + Send + 'static,
    C: Fn(String) -> CF + Clone + Send + Sync + 'static,
    CF: Future<Output = anyhow::Result<()>> + Send + 'static,
    F: Fn(String) -> FF + Clone + Send + Sync + 'static,
    FF: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let mut builder = GraphBuilder::<MemberState, MemberUpdate>::new().set_reducer(
        ClosureStateReducer::new(|mut s: MemberState, u: MemberUpdate| {
            if let MemberUpdate::Payload(p) = u {
                s.payload = Some(p);
            }
            Ok(s)
        }),
    );

    // `execute`: run the worker, classify its outcome, and route accordingly.
    builder = builder.add_node("execute", move |_s: MemberState, _c: NodeContext| {
        let run_worker = run_worker.clone();
        async move {
            match run_worker().await.map_err(graph_err)? {
                MemberOutcome::Completed { output } => Ok(NodeResult::Command(
                    Command::default()
                        .with_update(MemberUpdate::Payload(output))
                        .with_goto(["complete"]),
                )),
                MemberOutcome::Failed { reason } => Ok(NodeResult::Command(
                    Command::default()
                        .with_update(MemberUpdate::Payload(reason))
                        .with_goto(["fail"]),
                )),
            }
        }
    });

    // `complete`: quality-gate + idle via the injected reconciliation.
    builder = builder.add_node("complete", move |s: MemberState, _c: NodeContext| {
        let on_complete = on_complete.clone();
        async move {
            on_complete(s.payload.unwrap_or_default())
                .await
                .map_err(graph_err)?;
            Ok(NodeResult::Update(MemberUpdate::Noop))
        }
    });

    // `fail`: release + idle + record via the injected reconciliation.
    builder = builder.add_node("fail", move |s: MemberState, _c: NodeContext| {
        let on_failed = on_failed.clone();
        async move {
            on_failed(s.payload.unwrap_or_default())
                .await
                .map_err(graph_err)?;
            Ok(NodeResult::Update(MemberUpdate::Noop))
        }
    });

    let graph = builder
        .add_node("done", |_s: MemberState, _c: NodeContext| async move {
            Ok(NodeResult::Update(MemberUpdate::Noop))
        })
        .add_edge("complete", "done")
        .add_edge("fail", "done")
        .set_entry("execute")
        .mark_command_routing("execute")
        .set_finish("done")
        .compile()
        .map_err(|e| anyhow::anyhow!("member graph compile failed: {e}"))?;
    Ok(graph)
}

/// Structure-only [`GraphTopology`] of the member-execution graph for debug /
/// inspection (issue #4249, Phase 4). Built with no-op stub closures — the
/// topology exposes only node names, edges, and routing, never closure bodies.
pub(crate) fn member_graph_topology() -> anyhow::Result<GraphTopology> {
    let graph = build_member_graph(
        || async {
            Ok(MemberOutcome::Completed {
                output: String::new(),
            })
        },
        |_: String| async { Ok(()) },
        |_: String| async { Ok(()) },
    )?;
    Ok(graph.topology())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn completed_outcome_routes_to_complete() {
        let completed = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let c = completed.clone();
        let f = failed.clone();
        run_member_execution_graph(
            "test:complete",
            || async {
                Ok(MemberOutcome::Completed {
                    output: "ok".into(),
                })
            },
            move |out| {
                let c = c.clone();
                async move {
                    assert_eq!(out, "ok");
                    c.store(true, Ordering::SeqCst);
                    Ok(())
                }
            },
            move |_reason| {
                let f = f.clone();
                async move {
                    f.store(true, Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .await
        .expect("graph runs");
        assert!(completed.load(Ordering::SeqCst), "complete path ran");
        assert!(!failed.load(Ordering::SeqCst), "fail path did not run");
    }

    #[tokio::test]
    async fn failed_outcome_routes_to_fail() {
        let completed = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let c = completed.clone();
        let f = failed.clone();
        run_member_execution_graph(
            "test:fail",
            || async {
                Ok(MemberOutcome::Failed {
                    reason: "boom".into(),
                })
            },
            move |_out| {
                let c = c.clone();
                async move {
                    c.store(true, Ordering::SeqCst);
                    Ok(())
                }
            },
            move |reason| {
                let f = f.clone();
                async move {
                    assert_eq!(reason, "boom");
                    f.store(true, Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .await
        .expect("graph runs");
        assert!(failed.load(Ordering::SeqCst), "fail path ran");
        assert!(
            !completed.load(Ordering::SeqCst),
            "complete path did not run"
        );
    }

    #[tokio::test]
    async fn engine_error_from_worker_propagates() {
        let result = run_member_execution_graph(
            "test:err",
            || async { Err(anyhow::anyhow!("spawn failed")) },
            |_out| async { Ok(()) },
            |_reason| async { Ok(()) },
        )
        .await;
        assert!(result.is_err(), "worker engine error propagates out");
    }
}
