//! The **channel/CLI turn graph** (issue #4249).
//!
//! Per the per-folder `graph.rs` convention, this is the harness's top-level
//! (channel/CLI) graph definition, its available tools, and its summarization
//! step — all thin over the shared tinyagents seam
//! ([`run_turn_via_tinyagents_shared`]).
//!
//! **Graph.** A single agent-loop turn driven by the tinyagents harness (the
//! canonical channel/CLI path; the legacy `run_tool_call_loop` is removed),
//! covering the loop's control-flow seams (iteration cap, circuit breakers, stop
//! hooks). When the caller supplies an `on_progress` sender the harness event
//! stream is mirrored onto `AgentProgress` (live tool timeline, streaming text
//! deltas, cost/token footer) via the same
//! `OpenhumanEventBridge`
//! the chat route uses.
//!
//! **Available tools.** Reuses the bus handler's `Arc`-shared tool sets
//! (`tools_registry: Arc<Vec<Box<dyn Tool>>>` + per-turn `extra_tools`),
//! advertised via `SharedToolAdapter`
//! and filtered by `visible_tool_names`. `ask_user_clarification` is the
//! early-exit tool: it pauses the turn and returns its question as the turn's
//! text, which the channel relays as the reply.
//!
//! **Summarization.** [`run_channel_turn_via_graph`] resolves the model's
//! effective context window before dispatch so the shared seam runs the
//! context-window summarization step (`tinyagents::summarize`) ahead of the
//! deterministic front-trim.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc::Sender;

use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::tinyagents::run_turn_via_tinyagents_shared;
use crate::openhuman::agent::tinyagents::TurnModelSource;
use crate::openhuman::config::{MultimodalConfig, MultimodalFileConfig};
use crate::openhuman::tools::Tool;

/// Drive a channel/CLI turn on the graph engine. Returns the final assistant
/// text. When `on_progress` is `Some`, the run streams and mirrors progress
/// onto `AgentProgress`; pass `None` for a fire-and-forget final-text turn.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_channel_turn_via_graph(
    source: TurnModelSource,
    history: &mut Vec<ChatMessage>,
    tools_registry: Arc<Vec<Box<dyn Tool>>>,
    extra_tools: Vec<Box<dyn Tool>>,
    visible_tool_names: Option<&HashSet<String>>,
    model: &str,
    temperature: f64,
    max_iterations: usize,
    multimodal: MultimodalConfig,
    multimodal_files: MultimodalFileConfig,
    on_progress: Option<Sender<AgentProgress>>,
) -> Result<String> {
    let extra_arc = Arc::new(extra_tools);

    // The callable set is the visibility whitelist. The runner advertises each via
    // its own `spec()`, deduped by name (extras shadow the registry).
    // Fail-closed allowlist plumbing (issue #4452): the shared seam takes an
    // `Option<HashSet<String>>` where `None` = no filter (all visible tools) and
    // `Some(set)` = exactly those tools. The channel/CLI path's historical
    // convention is "no filter / empty set = every visible tool", so map both a
    // missing filter and an empty set to `None`; only a populated set is treated
    // as an explicit whitelist.
    let allowed: Option<HashSet<String>> = match visible_tool_names {
        Some(set) if !set.is_empty() => Some(set.clone()),
        _ => None,
    };

    // Resolve the model's effective context window (async provider probe) so the
    // harness can run the context-window summarization step (issue #4249) on
    // channel/CLI turns too — long-running channel threads otherwise grew
    // unbounded until the cap error — then build the turn's crate `ChatModel` set.
    // The `Provider` is confined to the seam `TurnModelSource` (issue #4249,
    // Phase 3 / Motion A): the harness graph names crate model types only, and
    // reads native-tool / vision capability + telemetry id off the built bundle.
    let context_window = source.effective_context_window(model).await;
    let turn_models = source.build(model, temperature, context_window)?;

    // Native-tool support drives the durable history-suffix dispatcher (native
    // envelope vs prompt-guided text) at the end of this turn; capture it before
    // `turn_models` is moved into the runner.
    let native_tools = turn_models.native_tools();
    let provider_id = turn_models.provider_id().to_string();

    // Multimodal prep (parity with the chat route's
    // `run_turn_via_tinyagents_session`, issue #4249): rehydrate image
    // placeholders for vision-capable models, then expand `[IMAGE:…]` /
    // `[FILE:…]` markers into provider-ready content before dispatch. The
    // expanded copy is provider-only — it is sent to the model but never
    // persisted back into the channel `history` (see the reconstruction below).
    let mut prepared = history.clone();
    if turn_models.supports_vision()
        && crate::openhuman::agent::multimodal::has_image_placeholders(&prepared)
    {
        prepared = crate::openhuman::agent::multimodal::rehydrate_image_placeholders(&prepared);
    }
    let prepared = crate::openhuman::agent::multimodal::prepare_messages_for_provider(
        &prepared,
        &multimodal,
        &multimodal_files,
    )
    .await
    .map(|prepared| prepared.messages)
    .unwrap_or(prepared);

    tracing::info!(
        model,
        max_iterations,
        observed = on_progress.is_some(),
        context_window,
        "[channel:graph] routing channel turn through tinyagents harness"
    );
    let outcome = run_turn_via_tinyagents_shared(
        turn_models,
        provider_id,
        model,
        prepared,
        vec![extra_arc, tools_registry],
        allowed,
        max_iterations,
        // Mirror the harness event stream onto AgentProgress when the caller
        // (e.g. channel dispatch) supplied a progress sink.
        on_progress,
        // Top-level (parent) turn — no child-progress attribution.
        None,
        // Resolved above — drives the context-window summarization step.
        context_window,
        // No mid-flight steering on the channel path.
        None,
        // Same pause as the chat path: a channel turn's continuation is the
        // user's next message, so ending the turn on the question is the whole
        // mechanism. Without this the model answers its own question (see
        // `session/turn/graph.rs`).
        &["ask_user_clarification"],
        // Channels surface the cap as an error (legacy `ErrorCheckpoint`), so no
        // graceful cap pause/summary here.
        false,
        // Bound the model's per-call output (legacy parity — channel turns ran at
        // the standard per-turn budget).
        Some(crate::openhuman::inference::provider::AGENT_TURN_MAX_OUTPUT_TOKENS),
        // Context middlewares: cache-align + default tool-result byte cap (the
        // channel path has no session `ContextManager` to source config from).
        crate::openhuman::agent::tinyagents::TurnContextMiddleware::defaults(),
        // Channel/CLI path carries its own gating; no session `.tool_policy()`.
        None,
        // Channel turns do not yet carry SDK workspace descriptors.
        None,
        // Interactive channel/CLI turn — never serve a cached model response.
        false,
        // #4457 (defect C): the channel/CLI path has no post-run wrap-up and does
        // NOT emit `TurnCompleted` itself, so let the seam emit the single
        // terminal event (legacy-engine parity).
        false,
    )
    .await?;
    // Append only this turn's typed suffix (assistant tool-calls + tool results +
    // final assistant), serialized with the matching dispatcher so a native tool
    // round persists as the `{content, tool_calls}` / `{tool_call_id, content}`
    // envelope (re-parsed by `convert::chat_message_to_message` next turn) rather
    // than an assistant with no `tool_calls` followed by an orphan `tool` row.
    // Using `outcome.conversation` (the typed messages-since-last-user) avoids
    // indexing into a post-trim `outcome.history` with the pre-trim `prior_len`,
    // which could drop current-turn messages when compaction reshaped the run.
    use crate::openhuman::agent::dispatcher::ToolDispatcher;
    let suffix = if native_tools {
        crate::openhuman::agent::dispatcher::NativeToolDispatcher
            .to_provider_messages(&outcome.conversation)
    } else {
        // History serialization is format-independent for prompt-guided providers
        // (tool calls already ride the visible assistant text); the XML dispatcher
        // renders the flat `[Tool results]` shape.
        crate::openhuman::agent::dispatcher::XmlToolDispatcher
            .to_provider_messages(&outcome.conversation)
    };
    history.extend(suffix);
    if outcome.early_exit_tool.is_some() {
        // Paused on `ask_user_clarification`: the suffix ends on the tool result
        // and there is no final assistant turn, so `outcome.text` (the question)
        // stands in for one. Without this the next turn's history would not show
        // that the agent had asked anything.
        history.push(ChatMessage::assistant(outcome.text.clone()));
    }
    Ok(outcome.text)
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
