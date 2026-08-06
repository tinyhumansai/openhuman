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
use tinyagents::graph::checkpoint::{Checkpoint, Checkpointer};
use tinyagents::graph::export::GraphTopology;
use tinyagents::graph::recursion::RecursionPolicy;
use tinyagents::graph::ClosureStateReducer;
use tinyagents::graph::{
    Command, CompiledGraph, GraphBuilder, Interrupt, NodeContext, NodeResult, END,
};
use tinyagents::harness::retry::RetryPolicy;
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
    /// The exact prompt handed to this stage's worker, when it surfaces one.
    /// Persisted into [`StepRecord::prompt`] for per-step provenance (read only
    /// for the execute stage; ignored elsewhere). `None` when the worker does not
    /// surface a prompt — e.g. the deterministic test mock.
    pub(crate) prompt: Option<String>,
}

impl DelegationStageOutput {
    /// A plain non-review stage output (the `approved` flag is unused and no
    /// prompt is surfaced).
    pub(crate) fn done(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            approved: true,
            prompt: None,
        }
    }
}

/// Current on-disk schema version for a checkpointed [`DelegationState`]. Bumped
/// only on a breaking state-shape change — introduced with the
/// `executions: Vec<String>` → `Vec<StepRecord>` migration (issue #3884).
/// Pre-versioned records deserialize to `0` via `#[serde(default)]`, so a resume
/// can tell a stale checkpoint from a current one.
pub(crate) const CURRENT_SCHEMA_VERSION: u32 = 1;

/// One completed execute-stage pass, recorded durably so a resumed run knows
/// exactly how far it got and can render/finalize per step rather than from a
/// flat text log. Replaces the former `executions: Vec<String>` (issue #3884).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StepRecord {
    /// 0-based execute pass: `0` is the first execution, `n` the n-th revision.
    pub(crate) index: usize,
    /// The exact prompt handed to the execute sub-agent for this pass — per-step
    /// provenance and the seam a later plan-edit slice (#3881) diffs against.
    /// Empty when the worker did not surface one.
    #[serde(default)]
    pub(crate) prompt: String,
    /// The sub-agent's result text — the value the former `Vec<String>` entry held.
    pub(crate) result: String,
}

/// Typed working state threaded through (and checkpointed across) the delegation
/// graph. Serde-serializable so a [`Checkpointer`] can persist and restore it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct DelegationState {
    /// The plan produced by the `plan` stage.
    pub(crate) plan: Option<String>,
    /// One record per execution pass (the first plus each revision), typed so a
    /// resumed run can render/finalize per step (widened from `Vec<String>`,
    /// issue #3884).
    pub(crate) executions: Vec<StepRecord>,
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
    /// On-disk schema version, stamped [`CURRENT_SCHEMA_VERSION`] on a fresh run
    /// and defaulting to `0` for pre-versioned checkpoints.
    /// [`run_or_resume_delegation`] expires any checkpoint whose version is below
    /// `CURRENT_SCHEMA_VERSION` (and any that fails to deserialize) instead of
    /// resuming or returning it — so a shape change that stays structurally
    /// decodable is still not misread.
    #[serde(default)]
    pub(crate) schema_version: u32,
}

impl DelegationState {
    /// A fresh run's initial state, stamped with the current schema version so
    /// its checkpoints are self-identifying.
    fn new_run() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            ..Self::default()
        }
    }

    /// The latest execution result text, if any — the projection the review
    /// prompt and the finalize summary read.
    pub(crate) fn last_result(&self) -> Option<&str> {
        self.executions.last().map(|r| r.result.as_str())
    }

    /// The execution result texts in order — the flat projection used for the
    /// durable approval-interrupt payload, kept `Vec<&str>` so that wire shape
    /// is unchanged from the pre-#3884 `Vec<String>`.
    pub(crate) fn executions_texts(&self) -> Vec<&str> {
        self.executions.iter().map(|r| r.result.as_str()).collect()
    }
}

/// Reducer updates emitted by the delegation nodes.
enum DelegationUpdate {
    Plan(String),
    Execution {
        prompt: String,
        result: String,
    },
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

    let initial = DelegationState::new_run();
    let execution = match thread_id.clone() {
        Some(tid) => graph.run_with_thread(tid, initial).await,
        None => graph.run(initial).await,
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
    let approved = decision_is_approve(&decision);
    tracing::info!(
        approved,
        "[interrupt] resuming durable delegation graph with approval decision"
    );
    let mut command = Command::default();
    command.resume = Some(decision);
    resume_graph(config, command, run_stage).await
}

/// Run the delegation graph, resuming from the last checkpoint boundary when the
/// configured thread has a live, compatible, non-terminal checkpoint, else
/// starting fresh (issue #3884 — node-level checkpoint & resume).
///
/// Classifies the thread's latest checkpoint and routes accordingly:
/// - **resumable** (a crash/failure left a mid-run boundary) → re-run only the
///   not-yet-completed nodes from that boundary via [`CompiledGraph::resume`]
///   with an empty command — never restarting from `plan`, and never re-running
///   an already-completed step (its [`StepRecord`] is restored from the state);
/// - **terminal** (already finalized/cancelled) → return the stored final state
///   without re-running (idempotent re-invocation of a stable thread);
/// - **absent** (no checkpoint) → a fresh durable run;
/// - **incompatible** (an undecodable record — e.g. a pre-#3884 `Vec<String>`
///   `executions` shape) → log, best-effort prune, and start fresh. The decode
///   error is swallowed into a fresh-run decision, never propagated, never a panic.
///
/// Callers that mint a unique `thread_id` per run (today's default) always take
/// the fresh path unchanged; the resume paths activate only when a caller reuses
/// a stable `thread_id`, so this is byte-compatible for existing callers while
/// wiring the resume seam that #3881 (plan-edit resume) builds on.
pub(crate) async fn run_or_resume_delegation<F, Fut>(
    config: DelegationConfig,
    run_stage: F,
) -> Result<DelegationOutcome, String>
where
    F: Fn(DelegationStage, DelegationState) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<DelegationStageOutput, String>> + Send + 'static,
{
    // Without a checkpointer + thread there is nothing to resume from.
    let (Some(cp), Some(tid)) = (config.checkpointer.clone(), config.thread_id.clone()) else {
        return run_delegation_durable(config, run_stage).await;
    };

    match cp.get(tid.as_str(), None).await {
        // A checkpoint written under an older state schema (e.g. a pre-#3884
        // record whose `executions` happened to be empty and so still decoded
        // into `Vec<StepRecord>`) is expired rather than resumed/returned — its
        // semantics may not match the current graph. This is what makes
        // `schema_version` an actual guard, not just documentation, and closes
        // the empty-`executions` gap that a decode failure alone cannot catch.
        Ok(Some(checkpoint)) if checkpoint.state.schema_version < CURRENT_SCHEMA_VERSION => {
            tracing::warn!(
                thread_id = %tid,
                schema_version = checkpoint.state.schema_version,
                current = CURRENT_SCHEMA_VERSION,
                "[delegation] checkpoint predates the current state schema; pruning and starting fresh"
            );
            prune_thread(cp.as_ref(), &tid).await;
            run_delegation_durable(config, run_stage).await
        }
        Ok(Some(checkpoint)) if checkpoint_is_resumable(&checkpoint) => {
            tracing::info!(
                thread_id = %tid,
                "[delegation] resuming durable delegation from its last checkpoint boundary"
            );
            // A crash/failure resume carries no decision value — an empty command
            // simply re-runs the pending node(s) (the crate's `retry` semantics).
            resume_graph(config, Command::default(), run_stage).await
        }
        Ok(Some(checkpoint)) => {
            // Terminal: return the finalized state without re-running. Defensive:
            // if this checkpoint still carried an unconsumed interrupt, surface it
            // instead of silently dropping it. The current routing never produces
            // this (an interrupt boundary schedules its node and is classified
            // resumable above), but a future schedule could, and a dropped pause
            // would strand a run.
            let pending = checkpoint.interrupts.first().map(|i| PendingApproval {
                interrupt_id: i.id.clone(),
                node: i.node.as_str().to_string(),
                payload: i.payload.clone(),
                thread_id: tid.clone(),
            });
            if pending.is_some() {
                tracing::warn!(
                    thread_id = %tid,
                    "[delegation] terminal-classified checkpoint carried a pending interrupt; surfacing it"
                );
            } else {
                tracing::info!(
                    thread_id = %tid,
                    "[delegation] thread already terminal; returning finalized state without re-running"
                );
            }
            Ok(DelegationOutcome {
                state: checkpoint.state,
                pending,
            })
        }
        Ok(None) => {
            tracing::debug!(
                thread_id = %tid,
                "[delegation] no checkpoint for thread; starting a fresh durable run"
            );
            run_delegation_durable(config, run_stage).await
        }
        // Only a *decode / shape-incompatibility* read error expires the
        // checkpoint. An operational error (SQLite busy / I/O / poisoned lock)
        // must NOT silently restart a valid resumable run — it is propagated so
        // durable work is retried by the caller, not dropped.
        Err(e) if is_incompatible_checkpoint_error(&e) => {
            tracing::warn!(
                thread_id = %tid,
                error = %e,
                "[delegation] undecodable/incompatible checkpoint; pruning and starting fresh"
            );
            prune_thread(cp.as_ref(), &tid).await;
            run_delegation_durable(config, run_stage).await
        }
        Err(e) => {
            tracing::error!(
                thread_id = %tid,
                error = %e,
                "[delegation] checkpoint read failed (operational); not restarting — propagating error"
            );
            Err(format!(
                "delegation checkpoint read failed for thread {tid}: {e}"
            ))
        }
    }
}

/// Best-effort prune of a dead/expired checkpoint thread so it is not re-probed
/// forever. Failure to prune is non-fatal (logged at debug).
async fn prune_thread(cp: &dyn Checkpointer<DelegationState>, thread_id: &str) {
    if let Err(e) = cp.delete_thread(thread_id).await {
        tracing::debug!(
            thread_id = %thread_id,
            error = %e,
            "[delegation] could not prune checkpoint thread (non-fatal)"
        );
    }
}

/// Whether a `Checkpointer::get` error is a decode / shape-incompatibility (safe
/// to expire the checkpoint) rather than an operational failure (SQLite busy /
/// I/O / poisoned lock — must not silently restart durable work). The vendored
/// `SqliteCheckpointer` reports both as `TinyAgentsError::Checkpoint(String)` but
/// tags decode failures with a `"decode …"` context (`sqlite.rs`:
/// `decode record` / `decode namespace` / `decode next_nodes`) — the only stable
/// discriminator it exposes.
fn is_incompatible_checkpoint_error(e: &tinyagents::TinyAgentsError) -> bool {
    matches!(e, tinyagents::TinyAgentsError::Checkpoint(msg) if msg.contains("decode"))
}

/// Whether a loaded checkpoint still has work to resume: a non-finalized,
/// non-cancelled run that still schedules a real (non-`END`) node.
fn checkpoint_is_resumable(checkpoint: &Checkpoint<DelegationState>) -> bool {
    if checkpoint.state.final_output.is_some() || checkpoint.state.cancelled {
        return false;
    }
    checkpoint.next_nodes.iter().any(|n| n.as_str() != END)
}

/// Rebuild the delegation graph (its node closures are not serializable — only
/// the typed state is checkpointed) with the same checkpointer + `thread_id`, and
/// re-enter it at the latest checkpoint's pending node(s) via
/// [`CompiledGraph::resume`]. `command` carries an approver's decision for the
/// human-approval interrupt path, or is empty for a plain crash/failure resume.
async fn resume_graph<F, Fut>(
    config: DelegationConfig,
    command: Command<DelegationUpdate>,
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
                DelegationUpdate::Execution { prompt, result } => {
                    let index = s.executions.len();
                    s.executions.push(StepRecord {
                        index,
                        prompt,
                        result,
                    });
                }
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
                    .with_update(DelegationUpdate::Execution {
                        prompt: out.prompt.unwrap_or_default(),
                        result: out.text,
                    })
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
                            "executions": s.executions_texts(),
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
                .map(|r| r.result.clone())
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
        })
        // Adapter-first landing of the crate-native per-node RetryPolicy
        // (tinyagents 1.5.0 `CompiledGraph::with_node_retry`). Conservative:
        // `max_attempts(1)` preserves today's single-attempt semantics exactly
        // (no bespoke retry glue existed here) and backoff sleeping stays off
        // (the default), so a transient node-handler failure surfaces as it does
        // today. This wires the seam so raising the attempt cap / enabling
        // backoff is a one-line, gated follow-up rather than a rewrite.
        .with_node_retry(RetryPolicy::default().with_max_attempts(1));

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
                            prompt: None,
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
        assert_eq!(state.executions[0].index, 0);
        assert_eq!(state.executions[0].result, "EXEC");
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

    /// The boxed-future type every inline test runner returns.
    type BoxedStageFut =
        std::pin::Pin<Box<dyn Future<Output = Result<DelegationStageOutput, String>> + Send>>;

    #[tokio::test]
    async fn execution_records_capture_index_and_prompt() {
        // A worker that surfaces an execute prompt; assert the per-step record
        // carries index + prompt + result (issue #3884).
        let runner = move |stage: DelegationStage, _s: DelegationState| {
            Box::pin(async move {
                match stage {
                    DelegationStage::Plan => Ok(DelegationStageOutput::done("PLAN")),
                    DelegationStage::Execute => Ok(DelegationStageOutput {
                        text: "EXEC".to_string(),
                        approved: true,
                        prompt: Some("EXEC-PROMPT".to_string()),
                    }),
                    DelegationStage::Review => Ok(DelegationStageOutput {
                        text: "APPROVE".to_string(),
                        approved: true,
                        prompt: None,
                    }),
                }
            }) as BoxedStageFut
        };
        let state = run_delegation(DelegationConfig::default(), runner)
            .await
            .expect("runs");
        assert_eq!(state.executions.len(), 1);
        assert_eq!(state.executions[0].index, 0);
        assert_eq!(state.executions[0].result, "EXEC");
        assert_eq!(state.executions[0].prompt, "EXEC-PROMPT");
    }

    #[test]
    fn legacy_string_executions_do_not_deserialize_into_step_records() {
        // A pre-#3884 checkpoint stored `executions` as a flat string array. It
        // must fail to load into the new `Vec<StepRecord>` — which is exactly
        // what makes `run_or_resume_delegation` expire the stale checkpoint
        // instead of misreading it.
        let legacy = r#"{"plan":"P","executions":["raw a","raw b"],"reviews":[],"revisions":0,"approved":true,"final_output":null,"cancelled":false}"#;
        assert!(serde_json::from_str::<DelegationState>(legacy).is_err());
    }

    #[test]
    fn schema_version_defaults_to_zero_and_step_records_round_trip() {
        // A pre-versioned record (no `schema_version`) loads as version 0.
        let unversioned = r#"{"plan":null,"executions":[],"reviews":[],"revisions":0,"approved":false,"final_output":null,"cancelled":false}"#;
        let s: DelegationState = serde_json::from_str(unversioned).expect("loads");
        assert_eq!(s.schema_version, 0);

        // A fresh run stamps the current version and round-trips step records.
        let mut fresh = DelegationState::new_run();
        assert_eq!(fresh.schema_version, CURRENT_SCHEMA_VERSION);
        fresh.executions.push(StepRecord {
            index: 0,
            prompt: "P".to_string(),
            result: "R".to_string(),
        });
        let json = serde_json::to_string(&fresh).expect("serializes");
        let back: DelegationState = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(back.executions[0].result, "R");
        assert_eq!(back.executions[0].prompt, "P");
    }

    #[tokio::test]
    async fn run_or_resume_starts_fresh_without_a_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let cp: Arc<dyn Checkpointer<DelegationState>> = Arc::new(
            tinyagents::graph::checkpoint::FileCheckpointer::new(dir.path()),
        );
        let config = DelegationConfig {
            checkpointer: Some(cp),
            thread_id: Some("fresh-1".to_string()),
            ..DelegationConfig::default()
        };
        let outcome = run_or_resume_delegation(config, flow_runner(0))
            .await
            .expect("runs fresh");
        assert!(outcome.state.final_output.is_some());
        assert_eq!(outcome.state.executions.len(), 1);
        assert_eq!(outcome.state.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn run_or_resume_continues_from_last_boundary_after_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let cp: Arc<dyn Checkpointer<DelegationState>> = Arc::new(
            tinyagents::graph::checkpoint::FileCheckpointer::new(dir.path()),
        );
        // Count stage invocations across BOTH the crashed run and the resume, to
        // prove plan/execute are NOT re-run on resume.
        let plan_runs = Arc::new(AtomicUsize::new(0));
        let exec_runs = Arc::new(AtomicUsize::new(0));
        let review_runs = Arc::new(AtomicUsize::new(0));
        let make_runner = |crash_first_review: bool| {
            let plan_runs = plan_runs.clone();
            let exec_runs = exec_runs.clone();
            let review_runs = review_runs.clone();
            move |stage: DelegationStage, _s: DelegationState| {
                let plan_runs = plan_runs.clone();
                let exec_runs = exec_runs.clone();
                let review_runs = review_runs.clone();
                Box::pin(async move {
                    match stage {
                        DelegationStage::Plan => {
                            plan_runs.fetch_add(1, Ordering::SeqCst);
                            Ok(DelegationStageOutput::done("PLAN"))
                        }
                        DelegationStage::Execute => {
                            exec_runs.fetch_add(1, Ordering::SeqCst);
                            Ok(DelegationStageOutput {
                                text: "EXEC".to_string(),
                                approved: true,
                                prompt: Some("EXEC-PROMPT".to_string()),
                            })
                        }
                        DelegationStage::Review => {
                            let n = review_runs.fetch_add(1, Ordering::SeqCst);
                            if crash_first_review && n == 0 {
                                Err("simulated crash during review".to_string())
                            } else {
                                Ok(DelegationStageOutput {
                                    text: "APPROVE".to_string(),
                                    approved: true,
                                    prompt: None,
                                })
                            }
                        }
                    }
                }) as BoxedStageFut
            }
        };
        let make_config = || DelegationConfig {
            checkpointer: Some(cp.clone()),
            thread_id: Some("resume-1".to_string()),
            ..DelegationConfig::default()
        };

        // Run 1: crashes during the first review. plan + execute completed and
        // were checkpointed at their super-step boundaries; the run returns Err.
        let crashed = run_or_resume_delegation(make_config(), make_runner(true)).await;
        assert!(crashed.is_err(), "first run crashes at review");
        assert_eq!(plan_runs.load(Ordering::SeqCst), 1);
        assert_eq!(exec_runs.load(Ordering::SeqCst), 1);

        // Run 2 (resume): a resumable checkpoint exists → re-enter at the pending
        // node (review), NOT plan; plan/execute must not run again.
        let resumed = run_or_resume_delegation(make_config(), make_runner(false))
            .await
            .expect("resumes");
        assert!(
            resumed.state.final_output.is_some(),
            "resumed run finalizes"
        );
        assert_eq!(
            plan_runs.load(Ordering::SeqCst),
            1,
            "plan not re-run on resume"
        );
        assert_eq!(
            exec_runs.load(Ordering::SeqCst),
            1,
            "execute not re-run on resume"
        );
        assert_eq!(
            resumed.state.executions.len(),
            1,
            "the pre-crash execution survived; no duplicate"
        );
        assert_eq!(resumed.state.executions[0].result, "EXEC");
    }

    #[tokio::test]
    async fn run_or_resume_is_idempotent_on_a_finalized_thread() {
        let dir = tempfile::tempdir().unwrap();
        let cp: Arc<dyn Checkpointer<DelegationState>> = Arc::new(
            tinyagents::graph::checkpoint::FileCheckpointer::new(dir.path()),
        );
        let stage_runs = Arc::new(AtomicUsize::new(0));
        let make_runner = || {
            let stage_runs = stage_runs.clone();
            move |stage: DelegationStage, _s: DelegationState| {
                let stage_runs = stage_runs.clone();
                Box::pin(async move {
                    stage_runs.fetch_add(1, Ordering::SeqCst);
                    match stage {
                        DelegationStage::Review => Ok(DelegationStageOutput {
                            text: "APPROVE".to_string(),
                            approved: true,
                            prompt: None,
                        }),
                        _ => Ok(DelegationStageOutput::done("X")),
                    }
                }) as BoxedStageFut
            }
        };
        let make_config = || DelegationConfig {
            checkpointer: Some(cp.clone()),
            thread_id: Some("done-1".to_string()),
            ..DelegationConfig::default()
        };

        let first = run_or_resume_delegation(make_config(), make_runner())
            .await
            .expect("runs");
        assert!(first.state.final_output.is_some());
        let after_first = stage_runs.load(Ordering::SeqCst);
        assert!(after_first > 0, "the first run invoked stage workers");

        // Re-invoke the SAME thread: a terminal checkpoint → return the finalized
        // state without re-running any stage worker.
        let second = run_or_resume_delegation(make_config(), make_runner())
            .await
            .expect("idempotent");
        assert!(second.state.final_output.is_some());
        assert_eq!(
            stage_runs.load(Ordering::SeqCst),
            after_first,
            "no stage worker re-ran on a finalized thread"
        );
    }

    #[tokio::test]
    async fn incompatible_checkpoint_expires_to_a_fresh_run() {
        // A pre-#3884 checkpoint (executions as `Vec<String>`) left in the store
        // must not crash a resume: `run_or_resume_delegation` expires it and runs
        // fresh, never panicking or surfacing the decode error.
        #[derive(Clone, Debug, Serialize, Deserialize)]
        struct LegacyState {
            plan: Option<String>,
            executions: Vec<String>,
            reviews: Vec<String>,
            revisions: usize,
            approved: bool,
            final_output: Option<String>,
            cancelled: bool,
        }
        let dir = tempfile::tempdir().unwrap();
        // Seed the store as the OLD state type under the thread.
        let legacy_cp: tinyagents::graph::checkpoint::FileCheckpointer<LegacyState> =
            tinyagents::graph::checkpoint::FileCheckpointer::new(dir.path());
        let legacy = Checkpoint {
            thread_id: "legacy-1".to_string(),
            checkpoint_id: "cp-legacy".to_string(),
            run_id: None,
            parent_checkpoint_id: None,
            namespace: vec![],
            state: LegacyState {
                plan: Some("old".to_string()),
                executions: vec!["a".to_string(), "b".to_string()],
                reviews: vec![],
                revisions: 0,
                approved: true,
                final_output: None,
                cancelled: false,
            },
            next_nodes: vec![],
            completed_tasks: vec![],
            pending_writes: vec![],
            interrupts: vec![],
            pending_activations: None,
            barrier_arrivals: vec![],
            metadata: json!({}),
        };
        legacy_cp.put(legacy).await.expect("seed legacy checkpoint");

        // Reopen the SAME store as the current state type and resume: the
        // undecodable record is expired and a fresh run completes.
        let cp: Arc<dyn Checkpointer<DelegationState>> =
            Arc::new(tinyagents::graph::checkpoint::FileCheckpointer::<
                DelegationState,
            >::new(dir.path()));
        let config = DelegationConfig {
            checkpointer: Some(cp),
            thread_id: Some("legacy-1".to_string()),
            ..DelegationConfig::default()
        };
        let outcome = run_or_resume_delegation(config, flow_runner(0))
            .await
            .expect("expires stale checkpoint and runs fresh");
        assert!(outcome.state.final_output.is_some(), "fresh run completed");
        assert_eq!(outcome.state.executions.len(), 1);
        assert_eq!(outcome.state.executions[0].result, "EXEC");
    }

    #[tokio::test]
    async fn checkpoint_below_current_schema_version_expires_to_fresh_run() {
        // A decodable but OLD-schema checkpoint (schema_version defaults to 0 —
        // e.g. a pre-#3884 record whose executions happened to be empty) must be
        // expired, not resumed: `schema_version` is a real guard, not just a doc.
        let dir = tempfile::tempdir().unwrap();
        let seed: tinyagents::graph::checkpoint::FileCheckpointer<DelegationState> =
            tinyagents::graph::checkpoint::FileCheckpointer::new(dir.path());
        let checkpoint = Checkpoint {
            thread_id: "old-schema".to_string(),
            checkpoint_id: "cp-old".to_string(),
            run_id: None,
            parent_checkpoint_id: None,
            namespace: vec![],
            state: DelegationState {
                plan: Some("stale".to_string()),
                ..Default::default()
            },
            next_nodes: vec![],
            completed_tasks: vec![],
            pending_writes: vec![],
            interrupts: vec![],
            pending_activations: None,
            barrier_arrivals: vec![],
            metadata: json!({}),
        };
        assert_eq!(
            checkpoint.state.schema_version, 0,
            "an un-stamped record is version 0"
        );
        seed.put(checkpoint)
            .await
            .expect("seed old-schema checkpoint");

        let cp: Arc<dyn Checkpointer<DelegationState>> =
            Arc::new(tinyagents::graph::checkpoint::FileCheckpointer::<
                DelegationState,
            >::new(dir.path()));
        let config = DelegationConfig {
            checkpointer: Some(cp),
            thread_id: Some("old-schema".to_string()),
            ..DelegationConfig::default()
        };
        let outcome = run_or_resume_delegation(config, flow_runner(0))
            .await
            .expect("expires + fresh");
        assert!(outcome.state.final_output.is_some(), "fresh run completed");
        assert_eq!(
            outcome.state.schema_version, CURRENT_SCHEMA_VERSION,
            "fresh run stamped the current version"
        );
        assert_eq!(
            outcome.state.plan.as_deref(),
            Some("PLAN"),
            "re-planned from scratch, not resumed with the stale plan"
        );
    }

    #[test]
    fn incompatible_checkpoint_error_matches_decode_not_operational() {
        use tinyagents::TinyAgentsError;
        // Decode / shape-incompatibility → safe to expire.
        assert!(is_incompatible_checkpoint_error(
            &TinyAgentsError::Checkpoint(
                "sqlite checkpointer: decode record: invalid type: string".to_string()
            )
        ));
        assert!(is_incompatible_checkpoint_error(
            &TinyAgentsError::Checkpoint("sqlite checkpointer: decode next_nodes: eof".to_string())
        ));
        // Operational failures must NOT be treated as incompatible (they must
        // propagate, not silently restart durable work).
        assert!(!is_incompatible_checkpoint_error(
            &TinyAgentsError::Checkpoint(
                "sqlite checkpointer: query latest checkpoint: database is locked".to_string()
            )
        ));
        assert!(!is_incompatible_checkpoint_error(
            &TinyAgentsError::Checkpoint(
                "sqlite checkpointer: connection lock poisoned".to_string()
            )
        ));
        assert!(!is_incompatible_checkpoint_error(&TinyAgentsError::Resume(
            "no checkpoint".to_string()
        )));
    }

    #[tokio::test]
    async fn terminal_checkpoint_with_a_pending_interrupt_surfaces_it() {
        // A terminal-classified checkpoint that still carries an interrupt must
        // surface it, not silently drop `pending`. (The live routing never
        // produces this shape; the terminal branch is defensive.)
        let dir = tempfile::tempdir().unwrap();
        let seed: tinyagents::graph::checkpoint::FileCheckpointer<DelegationState> =
            tinyagents::graph::checkpoint::FileCheckpointer::new(dir.path());
        let mut state = DelegationState::new_run();
        state.final_output = Some("done".to_string());
        let checkpoint = Checkpoint {
            thread_id: "terminal-interrupt".to_string(),
            checkpoint_id: "cp-ti".to_string(),
            run_id: None,
            parent_checkpoint_id: None,
            namespace: vec![],
            state,
            next_nodes: vec![],
            completed_tasks: vec![],
            pending_writes: vec![],
            interrupts: vec![Interrupt::with_id(
                "intr-1",
                "approval",
                json!({ "kind": "delegation_review" }),
            )],
            pending_activations: None,
            barrier_arrivals: vec![],
            metadata: json!({}),
        };
        seed.put(checkpoint).await.expect("seed terminal+interrupt");

        let cp: Arc<dyn Checkpointer<DelegationState>> =
            Arc::new(tinyagents::graph::checkpoint::FileCheckpointer::<
                DelegationState,
            >::new(dir.path()));
        let config = DelegationConfig {
            checkpointer: Some(cp),
            thread_id: Some("terminal-interrupt".to_string()),
            ..DelegationConfig::default()
        };
        let outcome = run_or_resume_delegation(config, flow_runner(0))
            .await
            .expect("terminal");
        let pending = outcome
            .pending
            .expect("the carried interrupt is surfaced, not dropped");
        assert_eq!(pending.node, "approval");
        assert_eq!(pending.interrupt_id, "intr-1");
        assert_eq!(outcome.state.final_output.as_deref(), Some("done"));
    }
}
