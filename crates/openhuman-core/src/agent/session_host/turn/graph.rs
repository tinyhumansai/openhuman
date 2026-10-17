//! The **chat turn graph** (issue #4249).
//!
//! Per the per-folder `graph.rs` convention, this module owns the chat folder's
//! graph definition, its available tools, and its summarization step — all thin
//! over the hosted TinyAgents invocation seam.
//!
//! **Graph.** The top-level interactive chat turn: a single agent-loop turn
//! driven by the tinyagents harness, observed via the session's `on_progress`
//! sink (live tool timeline, streaming text deltas, cost/token footer) and
//! steerable mid-flight through the session run queue. The loop pauses gracefully
//! at the model-call cap; the session driver then runs its tools-disabled,
//! grounded close when the in-loop conclusion was unavailable.
//!
//! **Available tools.** The agent's resolved harness tool set (`tools`),
//! advertised via the canonical shared-tool adapter
//! and filtered by `visible_tool_names`. The chat turn surfaces clarifying
//! questions inline rather than pausing, so it advertises **no early-exit
//! tools**.
//!
//! **Summarization.** The caller resolves the model's effective context window
//! and passes it as `context_window`, so the shared seam installs the
//! context-window summarization step (`tinyagents::summarize`) ahead of the
//! deterministic front-trim.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc::Sender;

use crate::agent::progress::AgentProgress;
use crate::agent::tinyagents::middleware::turn_context::TurnContextMiddleware;
use crate::agent::tinyagents::{TinyagentsTurnOutcome, TurnSpec};
use crate::inference::provider::AGENT_TURN_MAX_OUTPUT_TOKENS;
use tinyagents_definition::SandboxMode;
use tinyagents_harness::run_queue::RunQueue;
use tinyagents_session::transcript::TranscriptMessage;
use tinytools::Tool;

/// Inputs for a single chat-turn graph dispatch. Grouped into a struct so the
/// thin entry point stays readable (the shared seam takes 14 positional args);
/// each field maps to the chat path's variable inputs while the fixed chat-path
/// arguments (no child scope, no early-exit tools, graceful cap pause, per-turn
/// output cap) are applied inside [`run_chat_turn_graph`].
pub(crate) struct ChatTurnGraph {
    /// The turn's crate `ChatModel` set (primary + tier routes + summarizer),
    /// already built by the caller from the session's `TurnModelSource` (issue
    /// #4249, Phase 3 / Motion A). The graph names crate model types only.
    pub turn_models: crate::agent::tinyagents::TurnModels,
    /// The effective model id for this turn.
    pub model: String,
    /// Provider-ready messages (system + prior history + this turn's user turn,
    /// multimodal markers already expanded).
    pub messages: Vec<TranscriptMessage>,
    /// The agent's durable, `Arc`-shared harness tool set.
    pub tools: Arc<Vec<Box<dyn Tool>>>,
    /// The delegation tools synthesised for the current connection set,
    /// carried as their own set rather than merged into `tools` — the channel
    /// path does the same with its per-turn `extra_tools` (see
    /// [`crate::agent::harness::graph`]). This is what lets a
    /// mid-session Composio connect reach the model as a *callable* tool and a
    /// revoke withdraw one, without owning `tools`.
    pub synthesized_tools: Arc<Vec<Box<dyn Tool>>>,
    /// Callable-tool whitelist captured from the runtime request snapshot.
    /// An empty set is deny-all, never an implicit expansion to the driver's
    /// construction-time registry.
    pub visible_tool_names: HashSet<String>,
    /// Model-call cap for the loop.
    pub max_iterations: usize,
    /// Session progress sink — mirrors the harness event stream onto
    /// `AgentProgress` when `Some`.
    pub on_progress: Option<Sender<AgentProgress>>,
    /// Resolved context window, driving the summarization step. `None` when the
    /// provider does not advertise a window.
    pub context_window: Option<u64>,
    /// Session run queue for mid-flight steering.
    pub run_queue: Option<Arc<RunQueue<crate::agent::queued_turn::QueuedTurn>>>,
    /// openhuman context middlewares (cache-align, microcompact, tool-output
    /// budget + payload summarizer) sourced from the session's `ContextManager`.
    pub context_mw: TurnContextMiddleware,
    /// The agent's builder-configured tool policy + session context, enforced at
    /// the tool boundary. `None` when the session has no explicit policy.
    pub tool_policy: Option<crate::agent::tinyagents::ToolPolicyEnforcement>,
    /// Optional workspace descriptor for acting tools' default cwd. `None`
    /// keeps the shared-`action_dir` cwd behaviour.
    pub workspace_descriptor: Option<tinytools::WorkspaceDescriptor>,
    /// Declared sandbox mode for the top-level agent. The chat path scopes it
    /// around the shared harness so acting tools see the same mode as workers.
    pub sandbox_mode: SandboxMode,
    /// Explicit backend/persistence thread for this root turn.
    pub thread_id: Option<String>,
    /// The root host carrier, built before the turn begins and retained by all
    /// synchronous descendant dispatches.
    pub run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
    /// Durable host authority captured at session construction.
    pub hosted_base: Option<std::sync::Arc<crate::agent::tinyagents::host::OpenHumanHostBase>>,
    /// Stable definition id for hosted resolution.  The transcript-facing name
    /// may contain a thread suffix and is never an authority lookup key.
    pub agent_id: String,
}

/// Drive the chat turn graph: a thin wrapper over the shared tinyagents seam
/// that pins the chat path's fixed arguments. Returns the loop outcome; the
/// session driver owns post-loop grounded close and records all final usage in
/// its post-commit sidecar.
pub(crate) async fn run_chat_turn_graph(graph: ChatTurnGraph) -> Result<TinyagentsTurnOutcome> {
    let hosted_base = graph.hosted_base.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "hosted root invocation is unavailable because the session has no hosted authority"
        )
    })?;
    // `DriverRequest.tools` is the model-visible authority for this invocation.
    // The shared seam's `Some(empty)` spelling is intentionally a deny-all;
    // translating it to `None` would re-expose every construction-time tool
    // when a host deliberately supplied an empty snapshot.
    let visible_tool_names = Some(graph.visible_tool_names);
    // The turn's crate `ChatModel` set was built by the caller from the session's
    // `TurnModelSource` (issue #4249, Phase 3 / Motion A); the telemetry id rides
    // on the bundle.
    let provider_id = graph.turn_models.provider_id().to_string();
    async {
        let mut run_context = graph.run_context;
        run_context.progress = graph.on_progress.clone().or(run_context.progress);
        run_context.workspace = graph.workspace_descriptor.clone().or(run_context.workspace);
        run_context.sandbox_mode = Some(graph.sandbox_mode);
        run_context.thread_id = graph.thread_id;

        // Pause gracefully at the model-call cap so the turn emits a resumable
        // checkpoint instead of erroring or returning a dangling tool cycle.
        // The runtime's durable `after_commit` callback emits the terminal
        // event after any driver-grounded close has become part of the
        // candidate history, so the turn itself must not: a seam-level event
        // would arrive before that close and duplicate the lifecycle projection.
        TurnSpec {
            run_context,
            turn_models: graph.turn_models,
            provider_id,
            model: &graph.model,
            history: graph.messages,
            // Durable set first, synthesised second — the order `tool_specs` and
            // `OpenHumanSessionHost::all_tool_refs` use, so the name a spec was advertised
            // under resolves to the same instance here. The two sets are
            // disjoint by construction (`builder::drop_synthesized_name_collisions`),
            // so the order never decides a collision; it only keeps every
            // surface enumerating the tools in one sequence.
            tool_sets: vec![graph.tools, graph.synthesized_tools],
            allowed: visible_tool_names,
            max_iterations: graph.max_iterations,
            subagent_scope: None,
            context_window: graph.context_window,
            // Mid-flight steering from the session's run queue.
            run_queue: graph.run_queue,
            // `ask_user_clarification` pauses this turn: the turn returns the
            // question as `outcome.text`, the caller ends the turn on it, and the
            // user's next message is the answer. No resume plumbing is needed at
            // the top level — unlike a delegated child, a chat turn's natural
            // continuation IS the next user message.
            early_exit_tools: &["ask_user_clarification"],
            pause_at_cap: true,
            // Bound the main agent's per-call output (legacy parity — the engine
            // capped every turn at `AGENT_TURN_MAX_OUTPUT_TOKENS`).
            max_output_tokens: Some(AGENT_TURN_MAX_OUTPUT_TOKENS),
            // Context middlewares sourced from the session's ContextManager.
            context_mw: graph.context_mw,
            // Builder-configured tool policy enforcement (session chat path).
            tool_policy: graph.tool_policy,
            deterministic_cacheable: false,
            defer_turn_completed_to_caller: true,
        }
        .run_hosted(hosted_base, graph.agent_id)
        .await
    }
    .await
}
