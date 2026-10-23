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

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc::Sender;

use crate::agent::harness::fork_context::{AgentContextPreparedSource, ParentExecutionContext};
use crate::agent::subagent_host::SubagentUsage;
use crate::agent::tinyagents::{
    middleware::turn_context::TurnContextMiddleware, turn_policy::ToolPolicyEnforcement,
};
use crate::progress::AgentProgress;
use crate::security::turn_origin::AgentTurnOrigin;
use tinyagents_definition::SandboxMode;
use tinyagents_harness::store::InMemoryStore as ToolResultArtifactIndexStore;
use tinyagents_harness::turn_outcome::{ToolCallOutcome, ToolOutcomeSink};
use tinyagents_hooks::stop::StopHook;
use tinyinference_llm::model::ResolvedModelRoute;

/// Allocate a durable-unique root [`RunConfig`](tinyagents_harness::context::RunConfig).
///
/// A root config id is a cross-turn lifecycle identity, not a display label:
/// callers must retain the returned config/id for every boundary belonging to
/// that one root turn. Child configs are derived with [`RunContext::child`],
/// never by reusing a root literal such as `"agent_turn"`.
pub(crate) fn fresh_root_run_config(kind: &str) -> tinyagents_harness::context::RunConfig {
    tinyagents_harness::context::RunConfig::new(format!("{kind}-{}", uuid::Uuid::new_v4()))
}

/// Builds an owned direct child for the neutral subagent lifecycle.
///
/// Derives lineage before creation and shares cancellation across the tree.
pub(crate) fn direct_subagent_child(
    parent: &tinyagents_harness::context::RunContext<OpenHumanRunContext>,
    task_id: impl Into<String>,
    child_config: tinyagents_harness::context::RunConfig,
) -> tinyagents_harness::Result<(
    tinyagents_orchestration::subagent::SubagentTaskKey,
    tinyagents_harness::context::RunContext<OpenHumanRunContext>,
)> {
    let task_id = task_id.into();
    let task_key =
        tinyagents_orchestration::subagent::SubagentTaskKey::from_context(parent, task_id, None);
    let child_data = parent
        .data
        .child()
        .with_cancellation(parent.cancellation.clone());
    let mut child = parent.child(child_config, child_data)?;
    // TinyAgents gives the child a linked `child_token()` of the parent's, so a
    // parent cancel cascades down but cancelling the child (policy timeout,
    // steering) leaves the parent running. Point the host carrier at that same
    // child token; otherwise it keeps the parent's token and the host tools
    // observe (and could trigger) the wrong scope.
    child.data.cancellation = child.cancellation.clone();
    Ok((task_key, child))
}

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
    /// The turn's cost including children, or `None` when any call's cost is
    /// unknown (no reported charge and no catalogued price). Never a guess.
    pub cost_usd: Option<f64>,
    /// Whether `cost_usd` is entirely provider-charged or includes a catalog
    /// estimate (`Unknown` exactly when `cost_usd` is `None`).
    pub cost_source: tinyagents_harness::cost::CostSource,
    pub context_window: u64,
    /// Tokens the root agent's context held after the turn's final model call
    /// (that call's input plus its reply): the numerator of the context-window
    /// gauge. The token fields above are the turn's spend, summed over every
    /// call, and may exceed the window many times over on a long tool loop.
    /// Sub-agents are excluded, as each runs in its own window.
    pub context_tokens: u64,
    pub subagents: Vec<SubagentUsageEntry>,
    /// Reasoning/thinking tokens the turn's own model calls spent. Reported
    /// only for a turn run with a response-shape scope (library agent turns,
    /// `agent::tinyagents::response_shape`); `0` elsewhere.
    pub reasoning_tokens: u64,
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
    /// The turn's own cost (sub-agents excluded): every call's reported
    /// charge, else catalog estimate, else unknown.
    pub cost: tinyagents_harness::cost::CostTally,
    /// Input and output tokens of the turn's final model call. The totals
    /// above sum every call of the turn (its spend); these are the context the
    /// model last held, the numerator of the context-window gauge.
    pub last_call_input_tokens: u64,
    pub last_call_output_tokens: u64,
    /// The selected model's context window for this exact request.  The
    /// provider response's generic usage cannot represent this host datum.
    pub context_window: u64,
    /// Completed child runs observed before the root driver returned.  This is
    /// copied into the sidecar before transcript append so transcript billing
    /// and the post-commit UI use the same complete ledger.
    pub subagents: Vec<SubagentUsageEntry>,
    pub duration: Option<Duration>,
    /// Typed host refusal captured before the neutral runtime's string error
    /// boundary. Read only by the same logical turn that populated it.
    pub dispatch_refusal: Option<super::super::budget::DispatchRefusal>,
    pub tool_outcomes: Vec<ToolCallOutcome>,
    pub hit_cap: bool,
    pub wrap_up_injected: bool,
    pub resolved_route: Option<ResolvedModelRoute>,
    /// When the driver handed its candidate to the runtime. `after_commit`
    /// subtracts it from its own start to log how long the durable commit
    /// took (`[session-runtime] post-commit`).
    pub driver_finished_at: Option<std::time::Instant>,
    /// Set by the deadline wind-down middleware when it paused this turn
    /// (`tinyagents::deadline_wind_down`).
    pub wind_down: bool,
    /// How the turn was stopped early, if it was (breaker, wind-down,
    /// iteration cap). The driver classifies it; `after_commit` carries it on
    /// `TurnCompleted` so the trace shows a stopped turn at `WARNING`.
    pub stop: Option<tinyagents_harness::turn_stop::TurnStop>,
    /// Deferred native terminal finalized after transcript durability.
    pub deferred_terminal: Option<crate::agent::tinyagents::turn_outcome::DeferredTurnTerminal>,
}

#[path = "dispatch_control.rs"]
mod dispatch_control;
pub use dispatch_control::{decide_dispatch, DispatchDecision, DispatchInputs, TurnDispatchState};

/// Explicit OpenHuman data carried by a top-level or child agent run.
///
/// Shared members (`Arc`s, cancellation and workspace descriptor) retain the
/// identity required by a recursive run tree. [`Self::child`] deliberately
/// allocates a fresh route slot and subagent ledger: a child must not overwrite
/// the provider route or cost roll-up subsequently read by its parent.
#[derive(Clone)]
pub struct OpenHumanRunContext {
    /// Single-use authorization snapshots for workflows admitted in this run.
    pub(crate) workflow_scan_permits: super::operation_context::WorkflowScanPermits,
    /// Explicit shared model-call budget inherited by synchronous children.
    pub model_budget: Option<super::super::budget::ModelBudget>,
    /// Host admission policy shared by this run and its synchronous descendants.
    pub dispatch_admission: Option<Arc<dyn super::super::budget::PhysicalDispatchAdmission>>,
    /// The response format and output cap this root turn asks of every model
    /// call, and the slot its final call is reported into. Carried explicitly
    /// from the caller (`agent_chat`) through the session; children never
    /// inherit it.
    pub response_shape: Option<Arc<tinyagents_harness::middleware::ResponseShapeScope>>,
    /// A host can narrow synchronous delegation depth; children inherit this.
    pub max_spawn_depth: Option<usize>,
    /// Trust/routing source used by OpenHuman approval and attribution policy.
    pub origin: Option<AgentTurnOrigin>,
    /// Where this turn's approvals are surfaced (chat thread/client), whether
    /// it is the flow copilot, and which flow run it belongs to. Inline
    /// children inherit it through [`Self::child`]; a detached child starts
    /// without it ([`Self::detached_child`]).
    pub approval: crate::security::approval::ApprovalTurnContext,
    /// UI/event progress receiver for this turn tree.
    pub progress: Option<Sender<AgentProgress>>,
    /// Stop policies evaluated after each model call.
    pub stop_hooks: Vec<Arc<dyn StopHook<tinyagents_harness::cost::TurnCost>>>,
    /// The turn runs with `TurnOverrides::suppress_active_goal`: the thread
    /// goal must not reach the model this turn. Set by the turn prelude; the
    /// run-state block (which reads the goal from the store) follows it.
    pub goal_suppressed: bool,
    /// Agent/turn tool callbacks, registered after the runtime's global ones.
    pub tool_hooks: Vec<Arc<dyn tinyagents_hooks::tool::ToolHook<OpenHumanRunContext>>>,
    /// Host cap on real tool invocations for this turn tree; `None` leaves the
    /// product default. Inherited by children.
    pub tool_call_limit: Option<usize>,
    /// This turn's input is untrusted data (a host-only reviewer reading a PR
    /// diff), so the security gate built for it does not screen the input.
    pub untrusted_input_turn: bool,
    /// Wall-clock deadline of the top-level turn this run belongs to, set
    /// only by the session turn entry point from the web backstop
    /// (`agent::turn_deadline`). The harness winds the run down and clamps
    /// its wall clock against it. `None` for turns without a backstop.
    pub turn_deadline: Option<tinyagents_harness::turn_deadline::TurnDeadline>,
    /// Parent runtime snapshot used by canonical recursive tool dispatch.
    pub parent: Option<ParentExecutionContext>,
    /// Context-preparation sources already consumed in this turn.
    pub prepared_context_sources: Arc<Vec<AgentContextPreparedSource>>,
    /// Who the run acts as on memory; `None` is the root identity.
    pub memory_identity: Option<tinymemory_tools::layout::identity::Identity>,
    /// The context the turn runs under, read by tools, middleware and the
    /// provider factory instead of the ambient scope. Children inherit it.
    pub core: Option<std::sync::Weak<crate::core::runtime::CoreContext>>,
    /// The turn tree's transcript store (the root session's locator), where
    /// sub-agent transcripts land under the parent's store key. Inherited.
    pub transcripts: Option<Arc<dyn tinyagents_session::transcript::TranscriptLocator>>,
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
    /// Exact child-process environment supplied by the host for this turn.
    pub command_environment: Option<Arc<std::collections::BTreeMap<String, String>>>,
    /// Autonomous skill-run iteration cap lifted onto every sub-agent in the
    /// tree (inherited by `child()` through the clone); `None` for ordinary turns.
    pub autonomous_iter_cap: Option<usize>,
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
    /// The durable root identity selected for this actual host turn. It is
    /// populated before the runtime hands this carrier to the session and is
    /// reused by the hosted harness boundary so a session turn remains one
    /// coherent root lineage rather than two literal-named roots.
    root_run_id: Option<String>,
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
    /// Native session declarations recorded before this turn. These names
    /// explain historical declarations without granting an executable source.
    pub(crate) recorded_tool_names: Arc<std::collections::HashSet<String>>,

    /// Tools the session serves through `tool_search` for this turn although
    /// their own exposure is `Direct` (the agent definition's
    /// `deferred_tools`). Turn assembly registers them as `Deferred`, so the
    /// harness keeps them off the wire, indexes them for search and still
    /// admits a call by name. Empty for every turn without such a list.
    pub(crate) deferred_tool_names: Arc<std::collections::HashSet<String>>,
    /// Tool rules for this turn (`crate::tools::rules`): the operator's and
    /// the agent definition's layers, evaluated in this turn's channel/agent
    /// context. Installed as the harness `RunPolicy::tool_rules`, which
    /// applies them to the catalogue, `tool_search` and every call. A child
    /// run inherits its parent's layers (see [`Self::child`]) and may only add
    /// to them. `None` restricts nothing.
    pub(crate) tool_rules: Option<Arc<tinyagents_harness::tool::ToolRulePolicy>>,
    /// Context middleware snapshot prepared for this exact turn.
    pub(crate) context_middleware: Option<TurnContextMiddleware>,
    /// Model/harness sidecars consumed only after a durable commit.
    pub(crate) session_sidecar: Arc<Mutex<SessionTurnSidecar>>,
    /// Required structured-output contract for this exact host turn. The
    /// driver repairs it before returning a candidate to runtime validation.
    pub(crate) required_output: Option<tinyagents_harness::config::RequiredOutput>,
    /// The tool dialect the owning session composed its system prompt for,
    /// as the harness policy spells it. The turn harness runs the same
    /// dialect so a text protocol strips its schemas off the wire and builds
    /// the registry that recovers positional / code-style calls; `Auto`
    /// (the default) leaves the harness to choose from the model profile.
    pub(crate) tool_dialect: tinyagents_harness::config::ToolDispatcher,
    /// This turn's memory (`memory::lifecycle::hooks::MemoryTurn`): the pack
    /// recalled before the model ran, which `MemoryPackMiddleware` adds to
    /// every model request of the turn ephemerally (never committed), and the
    /// binding the compaction summarizer recalls under. A child run has its
    /// own and does not inherit it.
    pub(crate) memory_turn: Option<Arc<crate::memory::lifecycle::hooks::MemoryTurn>>,
    /// Shell commands this turn left running past their yield point
    /// (`sandbox::shell_processes`). Each run, child runs included, has its
    /// own; dropping the last copy at the end of the run stops them.
    pub(crate) shell_processes: Arc<crate::sandbox::shell_processes::ShellProcesses>,
}

/// The host callbacks and caps a turn's harness assembly installs, taken from
/// the explicit run context.
#[derive(Clone, Default)]
pub(crate) struct HarnessHostPolicy {
    /// Stop policies evaluated after each model call.
    pub(crate) stop_hooks: Vec<Arc<dyn StopHook<tinyagents_harness::cost::TurnCost>>>,
    /// Agent/turn tool callbacks, registered after the runtime's global ones.
    pub(crate) tool_hooks: Vec<Arc<dyn tinyagents_hooks::tool::ToolHook<OpenHumanRunContext>>>,
    /// The host's cap on real tool invocations.
    pub(crate) tool_call_cap: Option<usize>,
}

pub use super::operation_context::{approval_for_tool, OpenHumanRunExt};
pub(crate) use super::operation_context::{
    command_environment_for_tool, dispatch_ctx_for_tool, memory_identity_for_tool,
    memory_in_prompt_for_tool, parent_agent_id_for_tool, sandbox_mode_for_tool,
    shell_processes_for_tool, task_recency_window_for_tool, turn_origin_for_tool,
    HostOperationContext,
};
#[cfg(test)]
pub(crate) use super::operation_context::{test_turn_run, tool_context_with_approval};

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
            workflow_scan_permits: Arc::new(Mutex::new(Vec::new())),
            model_budget: None,
            dispatch_admission: None,
            response_shape: None,
            max_spawn_depth: None,
            origin: None,
            approval: Default::default(),
            progress: None,
            stop_hooks: Vec::new(),
            goal_suppressed: false,
            tool_hooks: Vec::new(),
            tool_call_limit: None,
            untrusted_input_turn: false,
            turn_deadline: None,
            parent: None,
            prepared_context_sources: Arc::new(Vec::new()),
            memory_identity: None,
            core: None,
            transcripts: None,
            file_state_agent_id: None,
            tool_result_artifact_index: None,
            attachment_placeholders: Arc::new(Vec::new()),
            dispatch: None,
            task_recency_window: None,
            command_environment: None,
            autonomous_iter_cap: None,
            sandbox_mode: None,
            spawn_depth: 0,
            subagent_usage: Arc::new(Mutex::new(Vec::new())),
            parent_subagent_usage: None,
            resolved_route: Arc::new(Mutex::new(None)),
            cancellation: tinyagents_harness::cancel::CancellationToken::default(),
            thread_id: None,
            root_run_id: None,
            workspace: None,
            tool_outcomes: None,
            tool_policy: None,
            current_tools: None,
            current_synthesized_tools: None,
            recorded_tool_names: Arc::new(std::collections::HashSet::new()),
            deferred_tool_names: Arc::new(std::collections::HashSet::new()),
            tool_rules: None,
            context_middleware: None,
            session_sidecar: Arc::new(Mutex::new(SessionTurnSidecar::default())),
            required_output: None,
            tool_dialect: tinyagents_harness::config::ToolDispatcher::Auto,
            memory_turn: None,
            shell_processes: crate::sandbox::shell_processes::ShellProcesses::new(),
        }
    }

    pub(crate) fn record_workflow_scan_permit(
        &self,
        workflow_id: &str,
        inputs: &serde_json::Value,
        call_id: &str,
        digest: &str,
    ) {
        let Ok(inputs) = serde_json::to_string(inputs) else {
            return;
        };
        self.workflow_scan_permits
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((
                workflow_id.to_owned(),
                inputs,
                call_id.to_owned(),
                digest.to_owned(),
            ));
    }

    pub(crate) fn take_workflow_scan_permit(
        &self,
        workflow_id: &str,
        inputs: &serde_json::Value,
        call_id: &str,
        digest: &str,
    ) -> bool {
        let Ok(inputs) = serde_json::to_string(inputs) else {
            return false;
        };
        let mut permits = self
            .workflow_scan_permits
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let Some(index) = permits.iter().position(|(id, args, call, hash)| {
            id == workflow_id && args == &inputs && call == call_id && hash == digest
        }) else {
            return false;
        };
        permits.remove(index);
        true
    }

    pub(crate) fn take_workflow_scan_permit_digest(
        &self,
        workflow_id: &str,
        inputs: &serde_json::Value,
        call_id: &str,
    ) -> Option<String> {
        let inputs = serde_json::to_string(inputs).ok()?;
        let mut permits = self
            .workflow_scan_permits
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let index = permits.iter().rposition(|(id, args, call, _)| {
            id == workflow_id && args == &inputs && call == call_id
        })?;
        Some(permits.remove(index).3)
    }

    /// Pins the harness tool dialect to the one the session prompt speaks.
    #[must_use]
    pub(crate) fn with_tool_dialect(
        mut self,
        dialect: tinyagents_harness::config::ToolDispatcher,
    ) -> Self {
        self.tool_dialect = dialect;
        self
    }

    /// Binds the context this run acts under, held weakly so a run kept past its turn never pins the tenant.
    #[must_use]
    pub fn with_core(mut self, core: Option<Arc<crate::core::runtime::CoreContext>>) -> Self {
        self.core = core.as_ref().map(Arc::downgrade);
        self
    }

    /// Binds the trusted source of this run's actions.
    #[must_use]
    pub fn with_origin(mut self, origin: AgentTurnOrigin) -> Self {
        self.origin = Some(origin);
        self
    }

    /// The dispatch context this run's readers use: its own `core`, else the
    /// calling task's (a run assembled by a caller that did not bind one).
    pub fn dispatch_ctx(&self) -> crate::core::runtime::DispatchCtx {
        crate::core::runtime::DispatchCtx::from_weak(self.core.as_ref())
    }

    /// The connected-identities prompt section, read under this run's context.
    pub(crate) fn connected_identities_md(&self) -> String {
        crate::integrations::render_connected_identities(&self.dispatch_ctx())
    }

    /// Sets the direct TinyTools workspace descriptor for this run.
    pub fn with_workspace(mut self, workspace: tinytools::WorkspaceDescriptor) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Creates (once) this host turn's durable-unique root config.
    ///
    /// The returned root id is stable across the session/runtime and hosted
    /// harness boundaries for this one actual turn. Callers creating a new
    /// root must call this once rather than construct a config from a repeated
    /// literal; callers creating children must use [`direct_subagent_child`].
    pub(crate) fn root_run_config(&mut self, kind: &str) -> tinyagents_harness::context::RunConfig {
        let run_id = self
            .root_run_id
            .get_or_insert_with(|| fresh_root_run_config(kind).run_id.as_str().to_owned())
            .clone();
        tinyagents_harness::context::RunConfig::new(run_id)
    }

    /// Binds a direct runner to its explicit parent without exposing the
    /// child-ledger link reserved for recursive [`Self::child`] calls.
    pub(crate) fn with_parent(mut self, parent: ParentExecutionContext) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Installs this turn's parent snapshot, binding the run's live progress
    /// sink to it.
    ///
    /// The snapshot is built from a prelude that captured `on_progress` when
    /// the runtime session was first created. A web-chat turn checks out a
    /// cached session — so that build is a no-op — and only afterwards calls
    /// `set_on_progress`, leaving the prelude's copy `None` for the rest of the
    /// session's life. Sub-agent spawn and completion are the only progress
    /// events that ride this parent sink instead of the harness event
    /// projection, so a stale `None` drops them outright: no `subagent_spawned`
    /// socket event, no run-ledger row, and a "Background tasks" panel that
    /// reads "none running" while sub-agents are working. The run context's own
    /// sink is the live one (it is what `driver.rs` hands the turn graph), so
    /// prefer it and keep the snapshot as the fallback.
    ///
    /// Returns the installed parent so a caller that must launch a sub-agent
    /// *before* the rest of the turn is assembled — `inject_triggered_memory_agent_context`
    /// is the one such caller — hands it the bound context rather than the
    /// stale snapshot it started from.
    pub(crate) fn attach_parent(
        &mut self,
        mut parent: ParentExecutionContext,
    ) -> &ParentExecutionContext {
        parent.on_progress = self.progress.clone().or(parent.on_progress);
        self.parent.insert(parent)
    }

    /// The callbacks and caps this turn's harness assembly installs.
    pub(crate) fn harness_host_policy(&self) -> HarnessHostPolicy {
        HarnessHostPolicy {
            stop_hooks: self.stop_hooks.clone(),
            tool_hooks: self.tool_hooks.clone(),
            tool_call_cap: self.tool_call_limit,
        }
    }

    /// The real tool invocation budget for a turn of `max_iterations`: the
    /// product default, narrowed by the host's explicit cap.
    pub(crate) fn tool_call_limit_for(&self, max_iterations: usize) -> usize {
        crate::agent::tinyagents::turn_policy::tool_call_limit(max_iterations, self.tool_call_limit)
    }

    /// Applies the host environment an entry point handed to the session:
    /// shared budget, delegation depth, cancellation, deadline and hooks.
    #[must_use]
    pub(crate) fn with_host_env(mut self, env: &crate::agent::turn_env::HostTurnEnv) -> Self {
        self.model_budget = env.model_budget.clone().or(self.model_budget);
        self.dispatch_admission = env.dispatch_admission.clone().or(self.dispatch_admission);
        self.max_spawn_depth = env.spawn_depth_limit.or(self.max_spawn_depth);
        if let Some(cancellation) = &env.cancellation {
            self.cancellation = cancellation.clone();
        }
        self.turn_deadline = env.turn_deadline.or(self.turn_deadline);
        self.tool_call_limit = env.tool_call_limit.or(self.tool_call_limit);
        self.command_environment = env.command_environment.clone().or(self.command_environment);
        self.memory_identity = env.memory_identity.clone().or(self.memory_identity);
        self.core = env.core.clone().or(self.core);
        self.stop_hooks
            .extend(env.hooks.stop_hooks().iter().cloned());
        self.tool_hooks
            .extend(env.hooks.tool_hooks().iter().cloned());
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

    /// Builds a child context that inherits authority, cancellation and shared
    /// workspace policy while keeping route observation and usage isolated.
    pub fn child(&self) -> Self {
        let mut child = self.clone();
        child.spawn_depth = self.spawn_depth.saturating_add(1);
        child.file_state_agent_id = None;
        child.parent_subagent_usage = Some(self.subagent_usage.clone());
        child.subagent_usage = Arc::new(Mutex::new(Vec::new()));
        child.resolved_route = Arc::new(Mutex::new(None));
        child.memory_turn = None;
        child.response_shape = None;
        child.shell_processes = crate::sandbox::shell_processes::ShellProcesses::new();
        child
    }

    /// Builds the explicit carrier for detached background work.
    ///
    /// A detached task keeps the authority, thread and workspace descriptor it
    /// needs to execute and deliver its own result, but it is no longer part of
    /// the originating turn's dispatch timer or subagent usage roll-up. It
    /// still inherits the shared monetary budget and physical-dispatch policy,
    /// and it does not observe cancellation that only stopped the interactive
    /// turn.
    pub fn detached_child(&self) -> Self {
        let mut child = self.child();
        // The chat surface belongs to the turn that started it. A detached
        // child's parks route through the origin and are marked detached, so
        // their cards survive the parent turn's end (#5499).
        child.approval = Default::default();
        child.dispatch = None;
        child.subagent_usage = Arc::new(Mutex::new(Vec::new()));
        child.parent_subagent_usage = None;
        // Detached work has its own terminal boundary; its typed refusal must
        // never be mistaken for the interactive parent's failure.
        child.session_sidecar = Arc::new(Mutex::new(SessionTurnSidecar::default()));
        child.cancellation = tinyagents_harness::cancel::CancellationToken::new();
        child
    }

    /// Converts this host context into TinyAgents' canonical run context.
    ///
    /// The canonical context receives the exact same cancellation and direct
    /// `tinytools::WorkspaceDescriptor`; no type alias, re-export or task-local
    /// bridge is involved.
    pub fn into_tinyagents(
        mut self,
        mut config: tinyagents_harness::context::RunConfig,
    ) -> tinyagents_harness::context::RunContext<Self> {
        // Imported transport facts and explicit child context must survive entry
        // into the native driver, which scopes the depth recorded in RunConfig.
        let depth = config
            .depth()
            .max(self.spawn_depth)
            .max(tinyagents_harness::context::current_run_depth().map_or(0, |facts| facts.depth));
        self.spawn_depth = depth;
        config = config.with_depth(depth);
        if config.thread_id.is_none() {
            if let Some(thread_id) = self.thread_id.as_deref() {
                config = config.with_thread(thread_id);
            }
        }
        let cancellation = self.cancellation.clone();
        let workspace = self.workspace.clone();
        let run_ext = OpenHumanRunExt::from_run_context(&self);
        let host_operations = Arc::new(HostOperationContext {
            workflow_scan_permits: self.workflow_scan_permits.clone(),
            origin: self.origin.clone(),
            approval: self.approval.clone(),
            sandbox_mode: self.sandbox_mode,
            parent_agent_id: self
                .parent
                .as_ref()
                .map(|parent| parent.agent_definition_id.clone()),
            memory_identity: self.memory_identity.clone(),
            memory_turn: self.memory_turn.clone(),
            core: self.core.clone(),
        });
        let context = tinyagents_harness::context::RunContext::new(config, self)
            .with_cancellation(cancellation)
            .with_state_view(host_operations)
            .with_host(run_ext);
        match workspace {
            Some(workspace) => context.with_workspace(workspace),
            None => context,
        }
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

#[cfg(test)]
#[path = "run_context_workflow_scan_tests.rs"]
mod workflow_scan_tests;
