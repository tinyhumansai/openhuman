//! The **chat turn graph** (issue #4249).
//!
//! Per the per-folder `graph.rs` convention, this module owns the chat folder's
//! graph definition, its available tools, and its summarization step — all thin
//! over the shared tinyagents seam
//! ([`run_turn_via_tinyagents_shared`](crate::openhuman::agent::tinyagents::run_turn_via_tinyagents_shared)).
//!
//! **Graph.** The top-level interactive chat turn: a single agent-loop turn
//! driven by the tinyagents harness, observed via the session's `on_progress`
//! sink (live tool timeline, streaming text deltas, cost/token footer) and
//! steerable mid-flight through the session run queue. The loop pauses gracefully
//! at the model-call cap so [`core`](super::core) can emit a resumable checkpoint
//! instead of erroring.
//!
//! **Available tools.** The agent's resolved harness tool set (`tools`),
//! advertised via `SharedToolAdapter`
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

use crate::openhuman::agent::harness::run_queue::RunQueue;
use crate::openhuman::agent::harness::{with_current_sandbox_mode, SandboxMode};
use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::tinyagents::{
    run_turn_via_tinyagents_shared, TinyagentsTurnOutcome, TurnContextMiddleware,
};
use crate::openhuman::inference::provider::AGENT_TURN_MAX_OUTPUT_TOKENS;
use crate::openhuman::tools::Tool;

/// Inputs for a single chat-turn graph dispatch. Grouped into a struct so the
/// thin entry point stays readable (the shared seam takes 14 positional args);
/// each field maps to the chat path's variable inputs while the fixed chat-path
/// arguments (no child scope, no early-exit tools, graceful cap pause, per-turn
/// output cap) are applied inside [`run_chat_turn_graph`].
pub(crate) struct ChatTurnGraph {
    /// The turn's crate `ChatModel` set (primary + tier routes + summarizer),
    /// already built by the caller from the session's `TurnModelSource` (issue
    /// #4249, Phase 3 / Motion A). The graph names crate model types only.
    pub turn_models: crate::openhuman::agent::tinyagents::TurnModels,
    /// The effective model id for this turn.
    pub model: String,
    /// Provider-ready messages (system + prior history + this turn's user turn,
    /// multimodal markers already expanded).
    pub messages: Vec<ChatMessage>,
    /// The agent's durable, `Arc`-shared harness tool set.
    pub tools: Arc<Vec<Box<dyn Tool>>>,
    /// The delegation tools synthesised for the current connection set,
    /// carried as their own set rather than merged into `tools` — the channel
    /// path does the same with its per-turn `extra_tools` (see
    /// [`crate::openhuman::agent::harness::graph`]). This is what lets a
    /// mid-session Composio connect reach the model as a *callable* tool and a
    /// revoke withdraw one, without owning `tools`.
    pub synthesized_tools: Arc<Vec<Box<dyn Tool>>>,
    /// Callable-tool whitelist (empty = every visible tool).
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
    pub run_queue: Option<Arc<RunQueue>>,
    /// openhuman context middlewares (cache-align, microcompact, tool-output
    /// budget + payload summarizer) sourced from the session's `ContextManager`.
    pub context_mw: TurnContextMiddleware,
    /// The agent's builder-configured tool policy + session context, enforced at
    /// the tool boundary. `None` when the session has no explicit policy.
    pub tool_policy: Option<crate::openhuman::agent::tinyagents::ToolPolicyEnforcement>,
    /// Optional per-profile workspace descriptor (section D of agent-profile
    /// homes). `Some` when the session's active profile opted into a dedicated
    /// workspace — acting tools then resolve their default cwd to
    /// `<action_dir>/profiles/<id>` via `ToolExecutionContext.workspace`. `None`
    /// (the common case) keeps the shared-`action_dir` cwd behaviour.
    pub workspace_descriptor: Option<tinyagents_harness::workspace::WorkspaceDescriptor>,
    /// Declared sandbox mode for the top-level agent. The chat path scopes it
    /// around the shared harness so acting tools see the same mode as workers.
    pub sandbox_mode: SandboxMode,
}

/// Drive the chat turn graph: a thin wrapper over the shared tinyagents seam
/// that pins the chat path's fixed arguments. Returns the turn outcome
/// ([`core`](super::core) folds usage, persists the conversation, and handles a
/// cap-hit checkpoint).
pub(crate) async fn run_chat_turn_graph(graph: ChatTurnGraph) -> Result<TinyagentsTurnOutcome> {
    // Fail-closed allowlist plumbing (issue #4452): the shared seam now takes an
    // `Option<HashSet<String>>` where `None` = no filter (all visible tools) and
    // `Some(set)` = exactly those tools. The chat path's historical convention is
    // "empty `visible_tool_names` = every visible tool", so map an empty set to
    // `None` to preserve that behavior; a populated set stays an explicit filter.
    let visible_tool_names = if graph.visible_tool_names.is_empty() {
        None
    } else {
        Some(graph.visible_tool_names)
    };
    // The turn's crate `ChatModel` set was built by the caller from the session's
    // `TurnModelSource` (issue #4249, Phase 3 / Motion A); the telemetry id rides
    // on the bundle.
    let provider_id = graph.turn_models.provider_id().to_string();
    with_current_sandbox_mode(graph.sandbox_mode, async {
        run_turn_via_tinyagents_shared(
            graph.turn_models,
            provider_id,
            &graph.model,
            graph.messages,
            // Durable set first, synthesised second — the order `tool_specs` and
            // `Agent::all_tool_refs` use, so the name a spec was advertised
            // under resolves to the same instance here. The two sets are
            // disjoint by construction (`builder::drop_synthesized_name_collisions`),
            // so the order never decides a collision; it only keeps every
            // surface enumerating the tools in one sequence.
            vec![graph.tools, graph.synthesized_tools],
            visible_tool_names,
            graph.max_iterations,
            // Mirror the harness event stream onto this session's progress sink.
            graph.on_progress,
            // Top-level chat turn — no child-progress attribution.
            None,
            graph.context_window,
            // Mid-flight steering from the session's run queue.
            graph.run_queue,
            // `ask_user_clarification` pauses this turn: the seam returns the
            // question as `outcome.text`, the caller ends the turn on it, and the
            // user's next message is the answer. No resume plumbing is needed at
            // the top level — unlike a delegated child, a chat turn's natural
            // continuation IS the next user message.
            //
            // This was `&[]` with a comment claiming the chat turn "surfaces
            // clarifying questions inline". It does not, and nothing else did
            // either: the tool's output went back to the model as a successful
            // result, so the model read its own question as answered and carried
            // on. Users watched an "Ask User Clarification" step succeed without
            // ever being asked anything.
            &["ask_user_clarification"],
            // Pause gracefully at the model-call cap so the turn emits a resumable
            // checkpoint instead of erroring or returning a dangling tool cycle.
            true,
            // Bound the main agent's per-call output (legacy parity — the engine
            // capped every turn at `AGENT_TURN_MAX_OUTPUT_TOKENS`).
            Some(AGENT_TURN_MAX_OUTPUT_TOKENS),
            // Context middlewares sourced from the session's ContextManager.
            graph.context_mw,
            // Builder-configured tool policy enforcement (session chat path).
            graph.tool_policy,
            // Per-profile dedicated workspace descriptor (section D). `None` for the
            // common shared-`action_dir` case; `Some` binds acting tools' default
            // cwd to `<action_dir>/profiles/<id>` for a `dedicated_workspace` profile.
            graph.workspace_descriptor,
            // Interactive chat turn — response caching MUST stay off so a live user
            // turn is never served a cached model response (correctness/safety).
            false,
            // #4457 (defect C): defer the terminal `TurnCompleted` to the caller.
            // The session path (`run_turn_impl` in `turn/core.rs`) runs its cap/#4093
            // wrap-up (`summarize_turn_wrapup`) *after* this seam returns and then
            // emits the single `TurnCompleted` itself — a seam-level emit here would
            // fire before that checkpoint streams and duplicate the event.
            true,
        )
        .await
    })
    .await
}
