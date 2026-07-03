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
use serde_json::{json, Value};
use tinyagents::graph::checkpoint::Checkpointer;
use tinyagents::graph::export::GraphTopology;
use tinyagents::graph::recursion::RecursionPolicy;
use tinyagents::graph::ClosureStateReducer;
use tinyagents::graph::{
    Command, CompiledGraph, GraphBuilder, Interrupt, NodeContext, NodeResult, END,
};
use tinyagents::CancellationToken;

/// Which stage a delegation node is asking the injected worker to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DelegationStage {
    /// Produce a plan for the task.
    Plan,
    /// Execute the current plan (re-run on revision).
    Execute,
    /// Review the latest execution; may approve or request a revision.
    Review,
}

/// What an injected stage worker returns.
#[derive(Debug, Clone)]
pub(crate) struct DelegationStageOutput {
    /// The stage's textual output (plan text, execution result, or review note).
    pub(crate) text: String,
    /// Only meaningful for [`DelegationStage::Review`]: `true` approves the
    /// execution and ends the loop; `false` requests another revision.
    pub(crate) approved: bool,
}

impl DelegationStageOutput {
    /// A plain non-review stage output (the `approved` flag is unused).
    pub(crate) fn done(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            approved: true,
        }
    }
}

/// Typed working state threaded through (and checkpointed across) the delegation
/// graph. Serde-serializable so a [`Checkpointer`] can persist and restore it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct DelegationState {
    /// The plan produced by the `plan` stage.
    pub(crate) plan: Option<String>,
    /// One entry per execution pass (the first plus each revision).
    pub(crate) executions: Vec<String>,
    /// One entry per review pass.
    pub(crate) reviews: Vec<String>,
    /// Number of revisions the reviewer requested (loops back to `execute`).
    pub(crate) revisions: usize,
    /// Set once the reviewer approves or the revision cap is hit.
    pub(crate) approved: bool,
    /// The final synthesized output (set by `finalize`).
    pub(crate) final_output: Option<String>,
    /// Set when the run short-circuited because its token was cancelled.
    pub(crate) cancelled: bool,
    /// The durable human-approval decision, once a resume delivers one:
    /// `Some(true)` = approved, `Some(false)` = denied, `None` = not gated /
    /// still awaiting. Only meaningful when `require_review_approval` is set.
    #[serde(default)]
    pub(crate) human_approved: Option<bool>,
    /// Set when the durable human-approval gate denied the delegated result
    /// (deny semantics: block the action, finalize as denied).
    #[serde(default)]
    pub(crate) denied: bool,
}

/// Reducer updates emitted by the delegation nodes.
enum DelegationUpdate {
    Plan(String),
    Execution(String),
    Review {
        note: String,
        approved: bool,
    },
    /// A durable human-approval decision delivered by a resume command.
    HumanDecision {
        approved: bool,
    },
    Final(String),
    Cancelled,
}

/// Configuration for a delegation run.
pub(crate) struct DelegationConfig {
    /// Upper bound on reviewer-requested revisions before forcing `finalize`.
    pub(crate) max_revisions: usize,
    /// Optional durable checkpointer (e.g. a `FileCheckpointer`). When set with a
    /// `thread_id`, the run persists its state at every super-step boundary.
    pub(crate) checkpointer: Option<Arc<dyn Checkpointer<DelegationState>>>,
    /// Thread id for checkpoint keying; required for the checkpointer to persist.
    pub(crate) thread_id: Option<String>,
    /// Cooperative cancellation; checked at each node boundary.
    pub(crate) cancel: CancellationToken,
    /// When set, an approved review does not finalize directly: the run reaches
    /// a durable **human-approval** interrupt (`NodeResult::Interrupt`) that is
    /// persisted via the checkpointer (Sync durability) and survives a process
    /// restart. The pause is only released by [`resume_delegation`] carrying the
    /// approver's decision. Requires `checkpointer` + `thread_id` (interrupts
    /// require durability).
    ///
    /// This is the **durable** approval boundary — distinct from the interactive
    /// chat-turn approval gate (the 10-min TTL steering pause surfaced via
    /// `ApprovalRequestCard`), which parks a live chat turn in memory and is left
    /// exactly as-is. Durable graphs pause by checkpoint; chat turns pause by
    /// steering. See the `approval` node below.
    pub(crate) require_review_approval: bool,
}

impl Default for DelegationConfig {
    fn default() -> Self {
        Self {
            max_revisions: 2,
            checkpointer: None,
            thread_id: None,
            cancel: CancellationToken::new(),
            require_review_approval: false,
        }
    }
}

/// A durable human-approval pause the delegation graph is parked on.
///
/// Produced when a run reaches the `approval` interrupt (see
/// [`DelegationConfig::require_review_approval`]). The pause is already
/// persisted as a checkpoint keyed by `thread_id`; the approver's decision is
/// delivered later via [`resume_delegation`], which survives a process restart.
#[derive(Debug, Clone)]
pub(crate) struct PendingApproval {
    /// Stable id of the emitted interrupt (matches a resume value to this pause).
    pub(crate) interrupt_id: String,
    /// The node that emitted the interrupt (always `"approval"` here).
    pub(crate) node: String,
    /// Approval-request payload presented to the approver (review notes, etc.).
    pub(crate) payload: Value,
    /// Thread id the paused graph is checkpointed under; the resume key.
    pub(crate) thread_id: String,
}

/// Outcome of a durable delegation run or resume.
#[derive(Debug, Clone)]
pub(crate) struct DelegationOutcome {
    /// The latest committed [`DelegationState`] at the run/resume boundary.
    pub(crate) state: DelegationState,
    /// `Some` when the run is parked on a durable human-approval interrupt;
    /// `None` when the run reached a terminal (finalized) boundary.
    pub(crate) pending: Option<PendingApproval>,
}

/// Run the plan→execute⇄review→finalize delegation graph, invoking `run_stage`
/// for each stage. Returns the final [`DelegationState`].
///
/// `run_stage` is the seam to the agent harness: production passes a closure that
/// dispatches each [`DelegationStage`] to `run_subagent`; tests pass a mock.
///
/// This is the non-gated convenience wrapper: with the default config
/// (`require_review_approval = false`) the graph never interrupts, so the
/// returned state is always terminal.
pub(crate) async fn run_delegation<F, Fut>(
    config: DelegationConfig,
    run_stage: F,
) -> Result<DelegationState, String>
where
    F: Fn(DelegationStage, DelegationState) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<DelegationStageOutput, String>> + Send + 'static,
{
    Ok(run_delegation_durable(config, run_stage).await?.state)
}

/// Run the delegation graph and report whether it finalized or parked on a
/// durable human-approval interrupt.
///
/// When [`DelegationConfig::require_review_approval`] is set and the reviewer
/// approves, the `approval` node emits [`NodeResult::Interrupt`]; the executor
/// persists a checkpoint (Sync durability — the crate default) and returns
/// control here with the interrupt in [`DelegationOutcome::pending`]. Deliver the
/// approver's decision later with [`resume_delegation`] — it may run after a
/// process restart, since the pause lives entirely in the checkpointer.
pub(crate) async fn run_delegation_durable<F, Fut>(
    config: DelegationConfig,
    run_stage: F,
) -> Result<DelegationOutcome, String>
where
    F: Fn(DelegationStage, DelegationState) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<DelegationStageOutput, String>> + Send + 'static,
{
    let thread_id = config.thread_id.clone();
    let mut graph = build_delegation_graph(
        config.max_revisions,
        config.cancel.clone(),
        config.require_review_approval,
        run_stage,
    )?
    .with_event_sink(Arc::new(super::observability::GraphTracingSink::new(
        "delegation:graph",
    )));

    if let Some(cp) = config.checkpointer {
        graph = graph.with_checkpointer(cp);
    }

    tracing::info!(
        max_revisions = config.max_revisions,
        durable = thread_id.is_some(),
        human_gated = config.require_review_approval,
        "[delegation] running sub-agent delegation graph"
    );

    let execution = match thread_id.clone() {
        Some(tid) => graph.run_with_thread(tid, DelegationState::default()).await,
        None => graph.run(DelegationState::default()).await,
    }
    .map_err(|e| format!("delegation graph run failed: {e}"))?;

    Ok(into_outcome(execution, thread_id))
}

/// Resume a delegation graph parked on a durable human-approval interrupt,
/// delivering the approver's `decision` through `Command { resume: .. }`.
///
/// The graph is rebuilt (its node closures are not serializable — only the typed
/// state is checkpointed) with the same checkpointer + `thread_id`, then
/// re-entered at the interrupted node via [`CompiledGraph::resume`] (the
/// `ResumeTarget::Latest` checkpoint). `decision` maps to approve/deny via
/// [`decision_is_approve`], so passing the approval RPC's `ApprovalDecision`
/// (serialized with its stable `as_str()` wire value — `approve_once` /
/// `approve_always_for_tool` / `deny`) routes the existing decision contract
/// into the resume **without changing that contract**.
///
/// TTL expiry → resume-with-deny: call this with [`deny_decision`] to preserve
/// the existing timeout-deny behavior for a pause that was never answered.
pub(crate) async fn resume_delegation<F, Fut>(
    config: DelegationConfig,
    decision: Value,
    run_stage: F,
) -> Result<DelegationOutcome, String>
where
    F: Fn(DelegationStage, DelegationState) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<DelegationStageOutput, String>> + Send + 'static,
{
    let thread_id = config
        .thread_id
        .clone()
        .ok_or_else(|| "delegation resume requires a thread_id".to_string())?;
    let cp = config
        .checkpointer
        .clone()
        .ok_or_else(|| "delegation resume requires a checkpointer".to_string())?;

    let graph = build_delegation_graph(
        config.max_revisions,
        config.cancel.clone(),
        config.require_review_approval,
        run_stage,
    )?
    .with_event_sink(Arc::new(super::observability::GraphTracingSink::new(
        "delegation:graph",
    )))
    .with_checkpointer(cp);

    let approved = decision_is_approve(&decision);
    tracing::info!(
        thread_id = %thread_id,
        approved,
        "[interrupt] resuming durable delegation graph with approval decision"
    );

    let mut command = Command::default();
    command.resume = Some(decision);

    let execution = graph
        .resume(thread_id.clone(), command)
        .await
        .map_err(|e| format!("delegation graph resume failed: {e}"))?;

    Ok(into_outcome(execution, Some(thread_id)))
}

/// The canonical deny decision used for TTL-expiry resume (resume-with-deny),
/// serialized to the approval RPC's stable `deny` wire value.
pub(crate) fn deny_decision() -> Value {
    json!("deny")
}

/// Fold a finished/paused graph execution into a [`DelegationOutcome`],
/// surfacing the first pending interrupt (if the run parked on one).
fn into_outcome(
    execution: tinyagents::graph::GraphExecution<DelegationState>,
    thread_id: Option<String>,
) -> DelegationOutcome {
    let pending = execution.interrupts.first().map(|i| {
        tracing::info!(
            interrupt_id = %i.id,
            node = %i.node.as_str(),
            "[interrupt] delegation run parked on durable human-approval interrupt"
        );
        PendingApproval {
            interrupt_id: i.id.clone(),
            node: i.node.as_str().to_string(),
            payload: i.payload.clone(),
            thread_id: thread_id.clone().unwrap_or_default(),
        }
    });
    DelegationOutcome {
        state: execution.state,
        pending,
    }
}

/// Map an approval decision value onto approve/deny. Accepts the approval RPC's
/// stable string forms (`approve_once`, `approve_always_for_tool`, `deny`), a
/// bare bool, or an object carrying `approved`/`decision` — so the existing
/// decision contract routes into `Command::resume` unchanged.
fn decision_is_approve(decision: &Value) -> bool {
    match decision {
        Value::Bool(b) => *b,
        Value::String(s) => matches!(
            s.as_str(),
            "approve_once" | "approve_always_for_tool" | "approve" | "approved"
        ),
        Value::Object(m) => {
            if let Some(b) = m.get("approved").and_then(Value::as_bool) {
                return b;
            }
            m.get("decision")
                .and_then(Value::as_str)
                .map(|d| d.starts_with("approve"))
                .unwrap_or(false)
        }
        _ => false,
    }
}

/// Build (but do not run) the delegation `CompiledGraph`. Shared by
/// [`run_delegation`] and [`delegation_graph_topology`] so the graph's structure
/// has one definition.
fn build_delegation_graph<F, Fut>(
    max_revisions: usize,
    cancel: CancellationToken,
    require_review_approval: bool,
    run_stage: F,
) -> Result<CompiledGraph<DelegationState, DelegationUpdate>, String>
where
    F: Fn(DelegationStage, DelegationState) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<DelegationStageOutput, String>> + Send + 'static,
{
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
                DelegationUpdate::HumanDecision { approved } => {
                    s.human_approved = Some(approved);
                    s.denied = !approved;
                    // A denial overrides the reviewer's in-graph approval: the
                    // human gate is the final authority on whether the result
                    // may finalize.
                    if !approved {
                        s.approved = false;
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
            // An approved result routes to the durable human-approval gate when
            // the run is human-gated; otherwise it finalizes directly. A
            // not-approved result always loops back to `execute` for a revision.
            let next = if !approved {
                "execute"
            } else if require_review_approval {
                "approval"
            } else {
                "finalize"
            };
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

    // approval (only when human-gated): a durable human-in-the-loop pause.
    //
    // First entry (`ctx.resume` is `None`): emit `NodeResult::Interrupt`. The
    // executor persists a boundary checkpoint (Sync durability) and returns
    // control to the caller — the pause now survives a process restart. Nothing
    // finalizes until a resume arrives.
    //
    // Re-entry (`ctx.resume` is `Some(decision)`): the approver's decision was
    // delivered via `Command { resume: .. }`. Apply approve/deny and route to
    // `finalize` (deny is honoured there as a blocked/denied result). This is a
    // durability mechanism for the PAUSE only — it grants no new approval
    // authority and never bypasses the security/approval boundary.
    //
    // Durable-vs-chat boundary: this pause is a *checkpointed graph interrupt*,
    // distinct from the interactive chat-turn approval gate (10-min TTL steering
    // pause via `ApprovalRequestCard`), which parks a live in-memory chat turn
    // and is deliberately left untouched.
    if require_review_approval {
        builder = builder.add_node("approval", move |s: DelegationState, ctx: NodeContext| {
            async move {
                match ctx.resume {
                    None => {
                        let payload = json!({
                            "kind": "delegation_review",
                            "reviews": s.reviews,
                            "executions": s.executions,
                            "revisions": s.revisions,
                        });
                        tracing::info!(
                            revisions = s.revisions,
                            "[interrupt] delegation review reached durable human-approval gate; pausing"
                        );
                        Ok(NodeResult::Interrupt(Interrupt::with_id(
                            "delegation-review-approval",
                            "approval",
                            payload,
                        )))
                    }
                    Some(decision) => {
                        let approved = decision_is_approve(&decision);
                        tracing::info!(
                            approved,
                            "[interrupt] delegation review resumed with human decision"
                        );
                        Ok(NodeResult::Command(
                            Command::default()
                                .with_update(DelegationUpdate::HumanDecision { approved })
                                .with_goto(["finalize"]),
                        ))
                    }
                }
            }
        });
    }

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
            } else if s.denied {
                format!(
                    "denied by reviewer after {} execution(s)",
                    s.executions.len()
                )
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

    if require_review_approval {
        builder = builder
            .mark_command_routing("approval")
            .mark_interrupt("approval");
    }

    let graph = builder
        .compile()
        .map_err(|e| format!("delegation graph compile failed: {e}"))?
        // Bound the execute⇄review loop as a backstop to the in-state counter:
        // each of execute/review may be visited at most max_revisions + 1 times.
        .with_recursion_policy(RecursionPolicy {
            max_visits_per_node: Some(max_revisions + 2),
            max_total_steps: (max_revisions + 1) * 4 + 8,
            ..RecursionPolicy::default()
        });

    Ok(graph)
}

/// Structure-only [`GraphTopology`] of the delegation graph for debug /
/// inspection (issue #4249, Phase 4). Built with a no-op stub stage worker —
/// the topology exposes only node names, edges, and routing, never closure
/// bodies.
pub(crate) fn delegation_graph_topology() -> Result<GraphTopology, String> {
    let graph = build_delegation_graph(
        DelegationConfig::default().max_revisions,
        CancellationToken::new(),
        // Topology export uses the non-gated shape (the four revision-loop
        // nodes); the durable `approval` interrupt node is additive and only
        // present when a run is human-gated.
        false,
        |_stage, _state| async { Ok(DelegationStageOutput::done("")) },
    )?;
    Ok(graph.topology())
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
    async fn human_gated_run_parks_on_interrupt_then_resume_approves() {
        let dir = tempfile::tempdir().unwrap();
        let cp: Arc<dyn Checkpointer<DelegationState>> = Arc::new(
            tinyagents::graph::checkpoint::FileCheckpointer::new(dir.path()),
        );
        let make_config = || DelegationConfig {
            require_review_approval: true,
            checkpointer: Some(cp.clone()),
            thread_id: Some("hg-approve".to_string()),
            ..DelegationConfig::default()
        };

        // First pass: reviewer approves on the first review, so the run reaches
        // the durable human-approval gate and parks on an interrupt.
        let outcome = run_delegation_durable(make_config(), flow_runner(0))
            .await
            .expect("runs");
        let pending = outcome.pending.expect("parked on the approval interrupt");
        assert_eq!(pending.node, "approval");
        assert_eq!(pending.thread_id, "hg-approve");
        assert!(
            outcome.state.final_output.is_none(),
            "must not finalize while paused for human approval"
        );

        // Simulated process restart: `resume_delegation` rebuilds a fresh graph
        // from the same checkpointer + thread and re-enters via Command::resume.
        let resumed = resume_delegation(make_config(), json!("approve_once"), flow_runner(0))
            .await
            .expect("resumes");
        assert!(resumed.pending.is_none(), "resume clears the pause");
        assert_eq!(resumed.state.human_approved, Some(true));
        assert!(!resumed.state.denied);
        assert!(
            resumed.state.final_output.is_some(),
            "resumes from checkpoint to finalize"
        );
    }

    #[tokio::test]
    async fn ttl_expiry_resume_with_deny_blocks_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let cp: Arc<dyn Checkpointer<DelegationState>> = Arc::new(
            tinyagents::graph::checkpoint::FileCheckpointer::new(dir.path()),
        );
        let make_config = || DelegationConfig {
            require_review_approval: true,
            checkpointer: Some(cp.clone()),
            thread_id: Some("hg-deny".to_string()),
            ..DelegationConfig::default()
        };

        let outcome = run_delegation_durable(make_config(), flow_runner(0))
            .await
            .expect("runs");
        assert!(outcome.pending.is_some(), "parks awaiting approval");

        // TTL expiry → resume-with-deny preserves the timeout-deny behavior.
        let resumed = resume_delegation(make_config(), deny_decision(), flow_runner(0))
            .await
            .expect("resumes");
        assert_eq!(resumed.state.human_approved, Some(false));
        assert!(resumed.state.denied, "deny is honoured as a blocked result");
        assert!(
            !resumed.state.approved,
            "human deny overrides the reviewer's in-graph approval"
        );
        assert!(resumed
            .state
            .final_output
            .as_deref()
            .unwrap_or_default()
            .contains("denied"));
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
