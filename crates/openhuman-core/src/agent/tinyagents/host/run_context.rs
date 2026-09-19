//! OpenHuman's explicit, host-owned state for one agent run.
//!
//! `tinyagents_harness::context::RunContext` owns generic runtime mechanics.
//! This type owns the product values that used to be recovered from ambient
//! task-locals: approval origin, host progress, attachment and artifact scope,
//! parent dispatch state, and the handles a tool needs to continue a run.
//! The shared turn seam receives this as its live OpenHuman carrier. The live
//! middleware registry uses this type as the TinyAgents run context,
//! so every host seam receives the same explicit carrier rather than recovering
//! product state from a task-local.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;

use tokio::sync::mpsc::Sender;

use crate::agent::harness::definition::SandboxMode;
use crate::agent::harness::fork_context::{AgentContextPreparedSource, ParentExecutionContext};
use crate::agent::harness::subagent_runner::SubagentUsage;
use crate::agent::harness::tool_result_artifacts::ToolResultArtifactIndexStore;
use crate::agent::progress::AgentProgress;
use crate::agent::stop_hooks::StopHook;
use crate::agent::tinyagents::turn_outcome::ToolOutcomeSink;
use crate::agent::tinyagents::{
    TurnContextMiddleware, turn_outcome::ToolCallOutcome, turn_policy::ToolPolicyEnforcement,
};
use crate::agent::turn_origin::AgentTurnOrigin;
use tinyinference_llm::model::ResolvedModelRoute;

/// One delegated run's token and cost totals, retained for the parent-turn
/// usage breakdown.
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentUsageEntry {
    pub task_id: String,
    pub agent_id: String,
    pub usage: SubagentUsage,
}

/// Complete usage for a completed root turn, including synchronous children.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LastTurnUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cost_usd: f64,
    pub context_window: u64,
    pub subagents: Vec<SubagentUsageEntry>,
}

/// Runtime-written sidecars for one OpenHuman session transition.
///
/// The host supplies this explicit sink in `before_turn`; the driver and its
/// middleware fill it without mutating a second host history or transcript.
/// `after_commit` reads it only after the runtime durable append succeeds.
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionTurnSidecar {
    pub model_calls: usize,
    pub tool_calls: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cost_usd: f64,
    /// The selected model's context window for this exact request.  The
    /// provider response's generic usage cannot represent this host datum.
    pub context_window: u64,
    /// Completed child runs observed before the root driver returned.  This is
    /// copied into the sidecar before transcript append so transcript billing
    /// and the post-commit UI use the same complete ledger.
    pub subagents: Vec<SubagentUsageEntry>,
    pub duration: Option<Duration>,
    pub tool_outcomes: Vec<ToolCallOutcome>,
    pub hit_cap: bool,
    pub wrap_up_injected: bool,
    pub resolved_route: Option<ResolvedModelRoute>,
}

/// Immutable inputs to the host's pre-dispatch policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchInputs {
    pub pause_requested: bool,
    pub pause_completed_calls: u64,
    pub pause_cap: u64,
    pub remaining: Option<Duration>,
    pub observed_max: Option<Duration>,
    pub observed_samples: u64,
}

/// Outcome of the host's pre-dispatch policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchDecision {
    Allow,
    RefusePaused {
        completed_model_calls: u64,
        cap: u64,
    },
    RefuseBudget {
        remaining_ms: u64,
        observed_max_ms: u64,
        observed_samples: u64,
    },
}

/// Decide whether a child can be started using only evidence recorded on this
/// turn. This stays host policy: TinyAgents owns the loop, not OpenHuman's
/// pause and wall-clock refusal rules.
pub fn decide_dispatch(inputs: DispatchInputs) -> DispatchDecision {
    if inputs.pause_requested {
        return DispatchDecision::RefusePaused {
            completed_model_calls: inputs.pause_completed_calls,
            cap: inputs.pause_cap,
        };
    }
    let (Some(remaining), Some(observed_max)) = (inputs.remaining, inputs.observed_max) else {
        return DispatchDecision::Allow;
    };
    if remaining < observed_max {
        return DispatchDecision::RefuseBudget {
            remaining_ms: remaining.as_millis().min(u128::from(u64::MAX)) as u64,
            observed_max_ms: observed_max.as_millis().min(u128::from(u64::MAX)) as u64,
            observed_samples: inputs.observed_samples,
        };
    }
    DispatchDecision::Allow
}

/// Shared root-turn state used by all synchronous descendants and siblings to
/// decide whether starting another delegate is still safe.
#[derive(Debug)]
pub struct TurnDispatchState {
    pause_requested: AtomicBool,
    pause_completed_calls: AtomicU64,
    pause_cap: AtomicU64,
    started: Instant,
    budget: Option<Duration>,
    observed_max_ms: AtomicU64,
    observed_samples: AtomicU64,
}

impl TurnDispatchState {
    pub fn new(budget: Option<Duration>) -> Self {
        Self {
            pause_requested: AtomicBool::new(false),
            pause_completed_calls: AtomicU64::new(0),
            pause_cap: AtomicU64::new(0),
            started: Instant::now(),
            budget,
            observed_max_ms: AtomicU64::new(0),
            observed_samples: AtomicU64::new(0),
        }
    }

    pub fn record_pause_requested(&self, completed_model_calls: u64, cap: u64) {
        self.pause_completed_calls
            .store(completed_model_calls, Ordering::SeqCst);
        self.pause_cap.store(cap, Ordering::SeqCst);
        self.pause_requested.store(true, Ordering::SeqCst);
    }

    pub fn record_subagent_elapsed(&self, elapsed: Duration) {
        let ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
        self.observed_samples.fetch_add(1, Ordering::SeqCst);
        self.observed_max_ms.fetch_max(ms, Ordering::SeqCst);
    }

    pub fn snapshot(&self) -> DispatchInputs {
        let observed_max = match self.observed_max_ms.load(Ordering::SeqCst) {
            0 => None,
            ms => Some(Duration::from_millis(ms)),
        };
        DispatchInputs {
            pause_requested: self.pause_requested.load(Ordering::SeqCst),
            pause_completed_calls: self.pause_completed_calls.load(Ordering::SeqCst),
            pause_cap: self.pause_cap.load(Ordering::SeqCst),
            remaining: self
                .budget
                .map(|budget| budget.saturating_sub(self.started.elapsed())),
            observed_max,
            observed_samples: self.observed_samples.load(Ordering::SeqCst),
        }
    }

    pub fn check(&self) -> DispatchDecision {
        decide_dispatch(self.snapshot())
    }
}

/// Explicit OpenHuman data carried by a top-level or child agent run.
///
/// Shared members (`Arc`s, cancellation and workspace descriptor) retain the
/// identity required by a recursive run tree. [`Self::child`] deliberately
/// allocates a fresh route slot and subagent ledger: a child must not overwrite
/// the provider route or cost roll-up subsequently read by its parent.
#[derive(Clone)]
pub struct OpenHumanRunContext {
    /// Trust/routing source used by OpenHuman approval and attribution policy.
    pub origin: Option<AgentTurnOrigin>,
    /// UI/event progress receiver for this turn tree.
    pub progress: Option<Sender<AgentProgress>>,
    /// Stop policies evaluated after each model call.
    pub stop_hooks: Vec<Arc<dyn StopHook>>,
    /// Parent runtime snapshot used by canonical recursive tool dispatch.
    pub parent: Option<ParentExecutionContext>,
    /// Context-preparation sources already consumed in this turn.
    pub prepared_context_sources: Arc<Vec<AgentContextPreparedSource>>,
    /// File-state identity used to detect stale parent reads after child writes.
    pub file_state_agent_id: Option<String>,
    /// Host-owned artifact index for tool-result references.
    pub(crate) tool_result_artifact_index: Option<Arc<ToolResultArtifactIndexStore>>,
    /// Current-turn attachment placeholders forwarded to vision delegates.
    pub attachment_placeholders: Arc<Vec<String>>,
    /// Dispatch guard shared by synchronous delegates in this run.
    pub dispatch: Option<Arc<TurnDispatchState>>,
    /// Optional task recency restriction for integration tools.
    pub task_recency_window: Option<Duration>,
    /// Sandbox mode of the agent definition executing this turn.
    pub sandbox_mode: Option<SandboxMode>,
    /// Zero-based OpenHuman subagent depth (the root is zero).
    pub spawn_depth: usize,
    /// This run's subagent usage roll-up; intentionally isolated for children.
    pub subagent_usage: Arc<Mutex<Vec<SubagentUsageEntry>>>,
    /// The immediate parent's ledger. A child writes its own completed total to
    /// this explicit handle, while nested children first collect in this run's
    /// isolated ledger. It is never a task-local or a root-global collector.
    parent_subagent_usage: Option<Arc<Mutex<Vec<SubagentUsageEntry>>>>,
    /// Provider/model/host-route observation for this run, written from the
    /// canonical response metadata by typed model middleware. Intentionally
    /// isolated for children.
    pub(crate) resolved_route: Arc<Mutex<Option<ResolvedModelRoute>>>,
    /// Cooperative cancellation shared by the complete recursive run tree.
    pub cancellation: tinyagents_harness::cancel::CancellationToken,
    /// Thread attached to provider requests and host persistence.
    pub thread_id: Option<String>,
    /// Direct canonical workspace descriptor; never use the old harness re-export.
    pub workspace: Option<tinytools::WorkspaceDescriptor>,
    /// Per-turn tool result capture shared with the event bridge.
    pub(crate) tool_outcomes: Option<ToolOutcomeSink>,
    /// Fail-closed OpenHuman tool-policy snapshot for this exact turn.
    pub(crate) tool_policy: Option<ToolPolicyEnforcement>,
    /// Exact executable durable tools selected by the host hook for this turn.
    /// The driver must consume this request-scoped source rather than its
    /// construction-time registry, so later visibility/revocation changes are
    /// authoritative at execution as well as prompt rendering.
    pub(crate) current_tools: Option<Arc<Vec<Box<dyn tinytools::Tool>>>>,
    /// Exact executable dynamic/delegation tools selected with
    /// [`Self::current_tools`] for this turn.
    pub(crate) current_synthesized_tools: Option<Arc<Vec<Box<dyn tinytools::Tool>>>>,
    /// Context middleware snapshot prepared for this exact turn.
    pub(crate) context_middleware: Option<TurnContextMiddleware>,
    /// Model/harness sidecars consumed only after a durable commit.
    pub(crate) session_sidecar: Arc<Mutex<SessionTurnSidecar>>,
    /// Required structured-output contract for this exact host turn. The
    /// driver repairs it before returning a candidate to runtime validation.
    pub(crate) required_output: Option<tinyagents_harness::config::RequiredOutput>,
}

impl Default for OpenHumanRunContext {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenHumanRunContext {
    /// Builds an unbound context. Entry points set only the values their turn
    /// actually owns; `None` is an explicit absence, not an ambient fallback.
    pub fn new() -> Self {
        Self {
            origin: None,
            progress: None,
            stop_hooks: Vec::new(),
            parent: None,
            prepared_context_sources: Arc::new(Vec::new()),
            file_state_agent_id: None,
            tool_result_artifact_index: None,
            attachment_placeholders: Arc::new(Vec::new()),
            dispatch: None,
            task_recency_window: None,
            sandbox_mode: None,
            spawn_depth: 0,
            subagent_usage: Arc::new(Mutex::new(Vec::new())),
            parent_subagent_usage: None,
            resolved_route: Arc::new(Mutex::new(None)),
            cancellation: tinyagents_harness::cancel::CancellationToken::new(),
            thread_id: None,
            workspace: None,
            tool_outcomes: None,
            tool_policy: None,
            current_tools: None,
            current_synthesized_tools: None,
            context_middleware: None,
            session_sidecar: Arc::new(Mutex::new(SessionTurnSidecar::default())),
            required_output: None,
        }
    }

    /// Sets the direct TinyTools workspace descriptor for this run.
    pub fn with_workspace(mut self, workspace: tinytools::WorkspaceDescriptor) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Binds a direct runner to its explicit parent without exposing the
    /// child-ledger link reserved for recursive [`Self::child`] calls.
    pub(crate) fn with_parent(mut self, parent: ParentExecutionContext) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Sets the same cancellation token on this context and its TinyAgents run.
    pub fn with_cancellation(
        mut self,
        cancellation: tinyagents_harness::cancel::CancellationToken,
    ) -> Self {
        self.cancellation = cancellation;
        self
    }

    /// Builds the explicit child state for a recursive invocation.
    ///
    /// This inheritance rule is tested in B1 and is ready for the later live
    /// plumbing. Cancellation, workspace, progress, policy hooks and dispatch
    /// state are inherited. Route observation and usage accounting are isolated,
    /// so a completed child cannot mutate facts subsequently persisted for its
    /// parent.
    pub fn child(&self) -> Self {
        let mut child = self.clone();
        child.spawn_depth = self.spawn_depth.saturating_add(1);
        child.file_state_agent_id = None;
        child.parent_subagent_usage = Some(self.subagent_usage.clone());
        child.subagent_usage = Arc::new(Mutex::new(Vec::new()));
        child.resolved_route = Arc::new(Mutex::new(None));
        child
    }

    /// Builds the explicit carrier for detached background work.
    ///
    /// A detached task keeps the authority, thread and workspace descriptor it
    /// needs to execute and deliver its own result, but it is no longer part of
    /// the originating turn. In particular it cannot consume that turn's
    /// dispatch budget, append to its usage ledger, or observe a cancellation
    /// request that only stopped the interactive turn.
    pub fn detached_child(&self) -> Self {
        let mut child = self.child();
        child.dispatch = None;
        child.subagent_usage = Arc::new(Mutex::new(Vec::new()));
        child.parent_subagent_usage = None;
        child.cancellation = tinyagents_harness::cancel::CancellationToken::new();
        child
    }

    /// Converts this host context into TinyAgents' canonical run context.
    ///
    /// The canonical context receives the exact same cancellation and direct
    /// `tinytools::WorkspaceDescriptor`; no type alias, re-export or task-local
    /// bridge is involved.
    pub fn into_tinyagents(
        self,
        mut config: tinyagents_harness::context::RunConfig,
    ) -> tinyagents_harness::context::RunContext<Self> {
        if config.thread_id.is_none() {
            if let Some(thread_id) = self.thread_id.as_deref() {
                config = config.with_thread(thread_id);
            }
        }
        let cancellation = self.cancellation.clone();
        let workspace = self.workspace.clone();
        let context = tinyagents_harness::context::RunContext::new(config, self)
            .with_cancellation(cancellation);
        match workspace {
            Some(workspace) => context.with_workspace(workspace),
            None => context,
        }
    }

    /// Returns this context's file-state identity for explicit tool plumbing.
    pub fn file_state_scope(&self) -> Option<&str> {
        self.file_state_agent_id.as_deref()
    }

    /// Records a child usage entry without relying on a task-local collector.
    pub fn append_subagent_usage(&self, entry: SubagentUsageEntry) {
        self.subagent_usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(entry);
    }

    /// Record a completed child on the immediate parent's ledger. This makes
    /// completed children visible at the root without leaking siblings into one
    /// another's in-flight ledger. Direct/root callers retain their own entry.
    pub fn record_completed_subagent_usage(&self, entry: SubagentUsageEntry) {
        let ledger = self
            .parent_subagent_usage
            .as_ref()
            .unwrap_or(&self.subagent_usage);
        ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(entry);
    }

    /// Snapshot child totals after the child has returned. A snapshot, rather
    /// than a task-local drain, preserves totals even when sibling futures are
    /// cancelled or one child fails after another has completed.
    pub fn subagent_usage_entries(&self) -> Vec<SubagentUsageEntry> {
        self.subagent_usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Promote already-completed descendants when this run cannot produce its
    /// own terminal outcome. This is intentionally a snapshot: callers invoke
    /// it only from one terminal finalizer, so every completed descendant is
    /// promoted once without exposing a shared, in-flight sibling ledger.
    pub fn promote_completed_descendant_usage(&self) {
        for entry in self.subagent_usage_entries() {
            self.record_completed_subagent_usage(entry);
        }
    }

    /// Resolves whether a file-state scope has been assigned to this context.
    /// Keeping the conversion here makes file-state callers use the explicit
    /// context instead of discovering an ambient identifier.
    pub fn file_state_agent_id(&self) -> Option<String> {
        self.file_state_agent_id.clone()
    }
}

#[cfg(test)]
#[path = "run_context_tests.rs"]
mod tests;
