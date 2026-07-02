//! Multi-stage sub-agent delegation expressed as a `tinyagents` orchestration
//! graph (issue #4249, #27/#28).
//!
//! Where [`run_turn_via_tinyagents_shared`](super::run_turn_via_tinyagents_shared)
//! drives *one* agent turn, this module composes *several* sub-agent stages into a
//! durable, resumable state machine — the SDK-native replacement for ad-hoc
//! `run_subagent` chaining:
//!
//! ```text
//!   plan ─▶ execute ─▶ review ──approved/maxed──▶ finalize ─▶ END
//!             ▲                   │
//!             └─────revise────────┘
//! ```
//!
//! Every feature the graph layer offers is exercised here:
//! - **conditional routing** — `review` returns a [`Command`] that routes to
//!   `execute` (revise) or `finalize` (done) based on the stage result;
//! - **recursion bounds** — a [`RecursionPolicy`] caps the `execute ⇄ review`
//!   revision loop as a backstop to the in-state `revisions` counter;
//! - **durable checkpoint/resume** — an optional [`Checkpointer`] persists the
//!   typed [`DelegationState`] at every super-step boundary (`run_with_thread`),
//!   so a crashed or paused run resumes from its last node;
//! - **cooperative cancellation** — a [`CancellationToken`] short-circuits the
//!   pipeline to `finalize` at the next node boundary.
//!
//! The per-stage worker is injected ([`run_delegation`]) so the orchestration
//! mechanics are unit tested with a deterministic mock; production passes a
//! closure that runs each stage through `run_subagent` / the agent harness.

use std::future::Future;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tinyagents::graph::checkpoint::Checkpointer;
use tinyagents::graph::recursion::RecursionPolicy;
use tinyagents::graph::ClosureStateReducer;
use tinyagents::graph::{Command, GraphBuilder, NodeContext, NodeResult, END};
use tinyagents::CancellationToken;

/// Which stage a delegation node is asking the injected worker to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegationStage {
    /// Produce a plan for the task.
    Plan,
    /// Execute the current plan (re-run on revision).
    Execute,
    /// Review the latest execution; may approve or request a revision.
    Review,
}

/// What an injected stage worker returns.
#[derive(Debug, Clone)]
pub struct DelegationStageOutput {
    /// The stage's textual output (plan text, execution result, or review note).
    pub text: String,
    /// Only meaningful for [`DelegationStage::Review`]: `true` approves the
    /// execution and ends the loop; `false` requests another revision.
    pub approved: bool,
}

impl DelegationStageOutput {
    /// A plain non-review stage output (the `approved` flag is unused).
    pub fn done(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            approved: true,
        }
    }
}

/// Typed working state threaded through (and checkpointed across) the delegation
/// graph. Serde-serializable so a [`Checkpointer`] can persist and restore it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DelegationState {
    /// The plan produced by the `plan` stage.
    pub plan: Option<String>,
    /// One entry per execution pass (the first plus each revision).
    pub executions: Vec<String>,
    /// One entry per review pass.
    pub reviews: Vec<String>,
    /// Number of revisions the reviewer requested (loops back to `execute`).
    pub revisions: usize,
    /// Set once the reviewer approves or the revision cap is hit.
    pub approved: bool,
    /// The final synthesized output (set by `finalize`).
    pub final_output: Option<String>,
    /// Set when the run short-circuited because its token was cancelled.
    pub cancelled: bool,
}

/// Reducer updates emitted by the delegation nodes.
enum DelegationUpdate {
    Plan(String),
    Execution(String),
    Review { note: String, approved: bool },
    Final(String),
    Cancelled,
}

/// Configuration for a delegation run.
pub struct DelegationConfig {
    /// Upper bound on reviewer-requested revisions before forcing `finalize`.
    pub max_revisions: usize,
    /// Optional durable checkpointer (e.g. a `FileCheckpointer`). When set with a
    /// `thread_id`, the run persists its state at every super-step boundary.
    pub checkpointer: Option<Arc<dyn Checkpointer<DelegationState>>>,
    /// Thread id for checkpoint keying; required for the checkpointer to persist.
    pub thread_id: Option<String>,
    /// Cooperative cancellation; checked at each node boundary.
    pub cancel: CancellationToken,
}

impl Default for DelegationConfig {
    fn default() -> Self {
        Self {
            max_revisions: 2,
            checkpointer: None,
            thread_id: None,
            cancel: CancellationToken::new(),
        }
    }
}

/// Run the plan→execute⇄review→finalize delegation graph, invoking `run_stage`
/// for each stage. Returns the final [`DelegationState`].
///
/// `run_stage` is the seam to the agent harness: production passes a closure that
/// dispatches each [`DelegationStage`] to `run_subagent`; tests pass a mock.
pub async fn run_delegation<F, Fut>(
    config: DelegationConfig,
    run_stage: F,
) -> Result<DelegationState, String>
where
    F: Fn(DelegationStage, DelegationState) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<DelegationStageOutput, String>> + Send + 'static,
{
    let max_revisions = config.max_revisions;
    let cancel = config.cancel.clone();

    let mut builder = GraphBuilder::<DelegationState, DelegationUpdate>::new().set_reducer(
        ClosureStateReducer::new(|mut s: DelegationState, u: DelegationUpdate| {
            match u {
                DelegationUpdate::Plan(p) => s.plan = Some(p),
                DelegationUpdate::Execution(e) => s.executions.push(e),
                DelegationUpdate::Review { note, approved } => {
                    s.reviews.push(note);
                    s.approved = approved;
                    if !approved {
                        s.revisions += 1;
                    }
                }
                DelegationUpdate::Final(f) => s.final_output = Some(f),
                DelegationUpdate::Cancelled => s.cancelled = true,
            }
            Ok(s)
        }),
    );

    // plan: produce the plan, then route to execute (or finalize if cancelled).
    let run_plan = run_stage.clone();
    let cancel_plan = cancel.clone();
    builder = builder.add_node("plan", move |s: DelegationState, _c: NodeContext| {
        let run_plan = run_plan.clone();
        let cancel = cancel_plan.clone();
        async move {
            if cancel.is_cancelled() {
                return Ok(NodeResult::Command(
                    Command::default()
                        .with_update(DelegationUpdate::Cancelled)
                        .with_goto(["finalize"]),
                ));
            }
            let out = run_plan(DelegationStage::Plan, s)
                .await
                .map_err(to_node_err)?;
            Ok(NodeResult::Command(
                Command::default()
                    .with_update(DelegationUpdate::Plan(out.text))
                    .with_goto(["execute"]),
            ))
        }
    });

    // execute: run the plan; route to review.
    let run_exec = run_stage.clone();
    let cancel_exec = cancel.clone();
    builder = builder.add_node("execute", move |s: DelegationState, _c: NodeContext| {
        let run_exec = run_exec.clone();
        let cancel = cancel_exec.clone();
        async move {
            if cancel.is_cancelled() {
                return Ok(NodeResult::Command(
                    Command::default()
                        .with_update(DelegationUpdate::Cancelled)
                        .with_goto(["finalize"]),
                ));
            }
            let out = run_exec(DelegationStage::Execute, s)
                .await
                .map_err(to_node_err)?;
            Ok(NodeResult::Command(
                Command::default()
                    .with_update(DelegationUpdate::Execution(out.text))
                    .with_goto(["review"]),
            ))
        }
    });

    // review: approve (→ finalize) or request a revision (→ execute), bounded by
    // `max_revisions` so a never-approving reviewer still terminates.
    let run_review = run_stage.clone();
    let cancel_review = cancel.clone();
    builder = builder.add_node("review", move |s: DelegationState, _c: NodeContext| {
        let run_review = run_review.clone();
        let cancel = cancel_review.clone();
        async move {
            if cancel.is_cancelled() {
                return Ok(NodeResult::Command(
                    Command::default()
                        .with_update(DelegationUpdate::Cancelled)
                        .with_goto(["finalize"]),
                ));
            }
            let revisions = s.revisions;
            let out = run_review(DelegationStage::Review, s)
                .await
                .map_err(to_node_err)?;
            // Approve when the reviewer is satisfied OR the revision budget is spent.
            let approved = out.approved || revisions >= max_revisions;
            let next = if approved { "finalize" } else { "execute" };
            Ok(NodeResult::Command(
                Command::default()
                    .with_update(DelegationUpdate::Review {
                        note: out.text,
                        approved,
                    })
                    .with_goto([next]),
            ))
        }
    });

    // finalize: synthesize the final output from the accumulated state, then end.
    builder = builder.add_node(
        "finalize",
        move |s: DelegationState, _c: NodeContext| async move {
            let summary = s
                .executions
                .last()
                .cloned()
                .unwrap_or_else(|| "<no execution>".to_string());
            let final_text = if s.cancelled {
                format!("cancelled after {} execution(s)", s.executions.len())
            } else {
                summary
            };
            Ok(NodeResult::Command(
                Command::default()
                    .with_update(DelegationUpdate::Final(final_text))
                    .with_goto([END]),
            ))
        },
    );

    builder = builder
        .set_entry("plan")
        .mark_command_routing("plan")
        .mark_command_routing("execute")
        .mark_command_routing("review")
        .mark_command_routing("finalize");

    let mut graph = builder
        .compile()
        .map_err(|e| format!("delegation graph compile failed: {e}"))?
        .with_event_sink(Arc::new(super::observability::GraphTracingSink::new(
            "delegation:graph",
        )))
        // Bound the execute⇄review loop as a backstop to the in-state counter:
        // each of execute/review may be visited at most max_revisions + 1 times.
        .with_recursion_policy(RecursionPolicy {
            max_visits_per_node: Some(max_revisions + 2),
            max_total_steps: (max_revisions + 1) * 4 + 8,
            ..RecursionPolicy::default()
        });

    if let Some(cp) = config.checkpointer {
        graph = graph.with_checkpointer(cp);
    }

    tracing::info!(
        max_revisions,
        durable = config.thread_id.is_some(),
        "[delegation] running sub-agent delegation graph"
    );

    let execution = match config.thread_id {
        Some(thread_id) => {
            graph
                .run_with_thread(thread_id, DelegationState::default())
                .await
        }
        None => graph.run(DelegationState::default()).await,
    }
    .map_err(|e| format!("delegation graph run failed: {e}"))?;

    Ok(execution.state)
}

/// Map an injected-stage error string into a graph node error.
fn to_node_err(e: String) -> tinyagents::TinyAgentsError {
    tinyagents::TinyAgentsError::Model(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// A reviewer that rejects the first `reject_first` executions, then approves,
    /// driving the execute⇄review revision loop.
    fn flow_runner(
        reject_first: usize,
    ) -> impl Fn(
        DelegationStage,
        DelegationState,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<DelegationStageOutput, String>> + Send>,
    > + Clone
           + Send
           + Sync
           + 'static {
        let reviews = Arc::new(AtomicUsize::new(0));
        move |stage, _state| {
            let reviews = reviews.clone();
            Box::pin(async move {
                match stage {
                    DelegationStage::Plan => Ok(DelegationStageOutput::done("PLAN")),
                    DelegationStage::Execute => Ok(DelegationStageOutput::done("EXEC")),
                    DelegationStage::Review => {
                        let n = reviews.fetch_add(1, Ordering::SeqCst);
                        Ok(DelegationStageOutput {
                            text: format!("review-{n}"),
                            approved: n >= reject_first,
                        })
                    }
                }
            })
        }
    }

    #[tokio::test]
    async fn approves_first_pass_no_revision() {
        let state = run_delegation(DelegationConfig::default(), flow_runner(0))
            .await
            .expect("runs");
        assert_eq!(state.plan.as_deref(), Some("PLAN"));
        assert_eq!(state.executions.len(), 1, "one execution, no revision");
        assert_eq!(state.reviews.len(), 1);
        assert_eq!(state.revisions, 0);
        assert!(state.approved);
        assert_eq!(state.final_output.as_deref(), Some("EXEC"));
    }

    #[tokio::test]
    async fn revises_then_approves() {
        // Reject the first review → one revision (a second execute+review).
        let state = run_delegation(DelegationConfig::default(), flow_runner(1))
            .await
            .expect("runs");
        assert_eq!(state.executions.len(), 2, "initial + one revised execution");
        assert_eq!(state.reviews.len(), 2);
        assert_eq!(state.revisions, 1);
        assert!(state.approved);
    }

    #[tokio::test]
    async fn revision_budget_caps_a_never_approving_reviewer() {
        // Reviewer never approves on its own; the max_revisions cap forces finalize.
        let config = DelegationConfig {
            max_revisions: 2,
            ..DelegationConfig::default()
        };
        let state = run_delegation(config, flow_runner(999))
            .await
            .expect("runs");
        // revisions counted: 1st review (rev 1), 2nd review (rev 2), 3rd review
        // hits revisions>=2 → forced approve. So 3 executions, 3 reviews.
        assert_eq!(state.revisions, 2, "stops at the revision budget");
        assert!(state.approved, "forced-approved at the cap");
        assert_eq!(state.executions.len(), 3);
    }

    #[tokio::test]
    async fn cancellation_short_circuits_to_finalize() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let ran = Arc::new(Mutex::new(Vec::<DelegationStage>::new()));
        let ran2 = ran.clone();
        let runner = move |stage: DelegationStage, _s: DelegationState| {
            let ran = ran2.clone();
            Box::pin(async move {
                ran.lock().unwrap().push(stage);
                Ok::<_, String>(DelegationStageOutput::done("X"))
            }) as std::pin::Pin<Box<dyn Future<Output = _> + Send>>
        };
        let config = DelegationConfig {
            cancel,
            ..DelegationConfig::default()
        };
        let state = run_delegation(config, runner).await.expect("runs");
        assert!(state.cancelled, "state flagged cancelled");
        assert!(state.final_output.is_some());
        assert!(
            ran.lock().unwrap().is_empty(),
            "no stage worker ran once cancelled at the plan boundary"
        );
    }

    #[tokio::test]
    async fn durable_checkpointer_persists_thread_state() {
        let dir = tempfile::tempdir().unwrap();
        let cp: Arc<dyn Checkpointer<DelegationState>> = Arc::new(
            tinyagents::graph::checkpoint::FileCheckpointer::new(dir.path()),
        );
        let config = DelegationConfig {
            checkpointer: Some(cp.clone()),
            thread_id: Some("run-1".to_string()),
            ..DelegationConfig::default()
        };
        let state = run_delegation(config, flow_runner(1)).await.expect("runs");
        assert!(state.approved);
        // The checkpointer recorded the run under its thread id.
        let threads = cp.list_threads().await.expect("list threads");
        assert!(
            threads.iter().any(|t| t == "run-1"),
            "thread persisted, saw {threads:?}"
        );
        let checkpoints = cp.list("run-1").await.expect("list checkpoints");
        assert!(
            !checkpoints.is_empty(),
            "at least one super-step boundary checkpoint persisted"
        );
    }
}
