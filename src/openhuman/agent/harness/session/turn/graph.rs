//! The **chat turn graph** (issue #4249).
//!
//! Per the per-folder `graph.rs` convention, this module owns the chat folder's
//! graph definition, its available tools, and its summarization step — all thin
//! over the shared tinyagents seam
//! ([`run_turn_via_tinyagents_shared`](crate::openhuman::tinyagents::run_turn_via_tinyagents_shared)).
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
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::inference::provider::{ChatMessage, AGENT_TURN_MAX_OUTPUT_TOKENS};
use crate::openhuman::tinyagents::{
    run_turn_via_tinyagents_shared, TinyagentsTurnOutcome, TurnContextMiddleware,
};
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
    pub turn_models: crate::openhuman::tinyagents::TurnModels,
    /// The effective model id for this turn.
    pub model: String,
    /// Provider-ready messages (system + prior history + this turn's user turn,
    /// multimodal markers already expanded).
    pub messages: Vec<ChatMessage>,
    /// The agent's resolved, `Arc`-shared harness tool set.
    pub tools: Arc<Vec<Box<dyn Tool>>>,
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
    pub tool_policy: Option<crate::openhuman::tinyagents::ToolPolicyEnforcement>,
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
    run_turn_via_tinyagents_shared(
        graph.turn_models,
        provider_id,
        &graph.model,
        graph.messages,
        vec![graph.tools],
        visible_tool_names,
        graph.max_iterations,
        // Mirror the harness event stream onto this session's progress sink.
        graph.on_progress,
        // Top-level chat turn — no child-progress attribution.
        None,
        graph.context_window,
        // Mid-flight steering from the session's run queue.
        graph.run_queue,
        // The top-level chat turn surfaces clarifying questions inline rather
        // than pausing the loop, so no early-exit tools here.
        &[],
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
        // Top-level chat turns do not yet carry SDK workspace descriptors.
        None,
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
}
