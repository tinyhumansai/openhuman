//! The **channel/CLI turn graph** (issue #4249).
//!
//! Per the per-folder `graph.rs` convention, this is the harness's top-level
//! (channel/CLI) graph definition, its available tools, and its summarization
//! step — all thin over the shared tinyagents seam
//! ([`TurnSpec::run`]).
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
//! advertised via the canonical shared-tool adapter
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

use crate::agent::progress::AgentProgress;
use crate::agent::tinyagents::TurnModelSource;
use crate::agent::tinyagents::TurnSpec;
use crate::config::{MultimodalConfig, MultimodalFileConfig};
use tinyagents_session::transcript::TranscriptMessage;
use tinytools::Tool;

/// Drive a channel/CLI turn on the graph engine. Returns the explicit turn
/// outcome, including the concrete route selected by the model runtime. When
/// `on_progress` is `Some`, the run streams and mirrors progress onto
/// `AgentProgress`; pass `None` for a fire-and-forget final-text turn.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_channel_turn_via_graph(
    source: TurnModelSource,
    history: &mut Vec<TranscriptMessage>,
    tools_registry: Arc<Vec<Box<dyn Tool>>>,
    extra_tools: Vec<Box<dyn Tool>>,
    visible_tool_names: Option<&HashSet<String>>,
    model: &str,
    temperature: f64,
    max_iterations: usize,
    multimodal: MultimodalConfig,
    multimodal_files: MultimodalFileConfig,
    on_progress: Option<Sender<AgentProgress>>,
    origin: Option<crate::agent::turn_origin::AgentTurnOrigin>,
    sandbox_mode: tinyagents_definition::SandboxMode,
    approval: crate::security::approval::ApprovalTurnContext,
    memory_identity: crate::memory::scope::MemoryIdentity,
) -> Result<crate::agent::tinyagents::TinyagentsTurnOutcome> {
    let extra_arc = Arc::new(extra_tools);
    // A channel/CLI turn is entered without a run context: the model source
    // was built under the turn's context, so bind that one here, once, and
    // carry it on the turn's run context.
    let dispatch = source.dispatch_ctx();

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
    let turn_models = source.build(model, temperature, context_window, None)?;

    // Native-tool support drives the durable history-suffix dispatcher (native
    // envelope vs prompt-guided text) at the end of this turn; capture it before
    // `turn_models` is moved into the runner.
    let native_tools = turn_models.native_tools();
    let provider_id = turn_models.provider_id().to_string();

    // Keep originals and durable references in every entry path. Resolution
    // into provider bytes belongs to the model decorator, after snapshots.
    let mut attachment_workspace = None;
    let mut attachment_config = None;
    for row in history.iter_mut().filter(|row| row.role == "user") {
        if row.content.contains("[FILE:") || row.content.contains("[IMAGE:") {
            if multimodal_files.max_files == 0 {
                anyhow::bail!("attachments are disabled for this channel input");
            }
            if attachment_config.is_none() {
                let mut config = dispatch.load_config().await.map_err(anyhow::Error::msg)?;
                config.multimodal = multimodal.clone();
                config.multimodal_files = multimodal_files.clone();
                attachment_config = Some(config);
            }
            let config = attachment_config
                .as_ref()
                .expect("config loaded for a marker-bearing history row");
            let workspace = Some(config.action_dir.clone());
            attachment_workspace = workspace.clone();
            let scope = crate::agent::attachments::AttachmentAccessScope {
                external_channel: matches!(
                    origin.as_ref(),
                    Some(crate::agent::turn_origin::AgentTurnOrigin::ExternalChannel { .. })
                ),
                workspace: workspace.clone(),
            };
            row.content =
                crate::agent::attachments::stage(&row.content, "channel", config, &scope).await?;
            row.parts = None;
        }
    }
    let prepared = if crate::agent::multimodal::has_image_placeholders(history) {
        crate::agent::multimodal::rehydrate_image_placeholders(history).await
    } else {
        history.clone()
    };

    tracing::info!(
        model,
        max_iterations,
        observed = on_progress.is_some(),
        context_window,
        "[channel:graph] routing channel turn through tinyagents harness"
    );
    let turn_origin = origin.clone();
    let mut run_context = crate::agent::tinyagents::host::OpenHumanRunContext::new()
        .with_core(dispatch.core().cloned());
    run_context.origin = origin;
    run_context.approval = approval;
    run_context.memory_identity = Some(memory_identity);
    run_context.sandbox_mode = Some(sandbox_mode);
    run_context.workspace = attachment_workspace.map(tinytools::WorkspaceDescriptor::new);
    seed_channel_attachments(&mut run_context, &prepared);
    // The channel dispatcher owns this explicit sink. It wins over an embedder
    // scope exactly as it did before this carrier was introduced.
    run_context.progress = on_progress.clone().or(run_context.progress);
    let outcome = TurnSpec {
        run_context,
        turn_models,
        provider_id,
        model,
        history: prepared,
        tool_sets: vec![extra_arc, tools_registry],
        allowed,
        max_iterations,
        // Top-level (parent) turn — no child-progress attribution.
        subagent_scope: None,
        // Resolved above — drives the context-window summarization step.
        context_window,
        // No mid-flight steering on the channel path.
        run_queue: None,
        // Same pause as the chat path: a channel turn's continuation is the
        // user's next message, so ending the turn on the question is the whole
        // mechanism. Without this the model answers its own question (see
        // `session/turn/graph.rs`).
        early_exit_tools: &["ask_user_clarification"],
        // Channels surface the cap as an error (legacy `ErrorCheckpoint`), so no
        // graceful cap pause/summary here.
        pause_at_cap: false,
        // Bound the model's per-call output (legacy parity — channel turns ran at
        // the standard per-turn budget).
        max_output_tokens: Some(crate::inference::provider::AGENT_TURN_MAX_OUTPUT_TOKENS),
        // Context middlewares: cache-align + default tool-result byte cap (the
        // channel path has no session `ContextManager` to source config from).
        context_mw:
            crate::agent::tinyagents::middleware::turn_context::TurnContextMiddleware::defaults(),
        // Channel/CLI path carries its own gating; no session `.tool_policy()`.
        tool_policy: None,
        // Interactive channel/CLI turn — never serve a cached model response.
        deterministic_cacheable: false,
        // #4457 (defect C): the channel/CLI path has no post-run wrap-up and does
        // NOT emit `TurnCompleted` itself, so let the seam emit the single
        // terminal event (legacy-engine parity).
        defer_turn_completed_to_caller: false,
    }
    .run();
    let outcome = match (turn_origin, dispatch.into_core()) {
        (Some(origin), Some(context)) => {
            crate::core::runtime::CoreContext::scope_with_turn_origin(
                context,
                Some(origin),
                outcome,
            )
            .await?
        }
        _ => outcome.await?,
    };
    // Append only this turn's typed suffix (assistant tool-calls + tool results +
    // final assistant), serialized with the matching dispatcher so a native tool
    // round persists as the `{content, tool_calls}` / `{tool_call_id, content}`
    // envelope (re-parsed by `convert::chat_message_to_message` next turn) rather
    // than an assistant with no `tool_calls` followed by an orphan `tool` row.
    // Using `outcome.conversation` (the typed messages-since-last-user) avoids
    // indexing into a post-trim `outcome.history` with the pre-trim `prior_len`,
    // which could drop current-turn messages when compaction reshaped the run.
    let suffix = if native_tools {
        crate::agent::message_convert::provider_messages_from_conversation(
            &tinytools_agent::dialect::NativeDialect,
            &outcome.conversation,
        )
    } else {
        // History serialization is format-independent for prompt-guided providers
        // (tool calls already ride the visible assistant text); the XML dispatcher
        // renders the flat `[Tool results]` shape.
        crate::agent::message_convert::provider_messages_from_conversation(
            &tinytools_agent::dialect::XmlDialect,
            &outcome.conversation,
        )
    };
    history.extend(suffix);
    if outcome.early_exit_tool.is_some() {
        // Paused on `ask_user_clarification`: the suffix ends on the tool result
        // and there is no final assistant turn, so `outcome.text` (the question)
        // stands in for one. Without this the next turn's history would not show
        // that the agent had asked anything.
        history.push(TranscriptMessage::assistant(outcome.text.clone()));
    }
    Ok(outcome)
}

/// Carry the current user images into named vision delegation on CLI/channel turns.
fn seed_channel_attachments(
    context: &mut crate::agent::tinyagents::host::OpenHumanRunContext,
    history: &[TranscriptMessage],
) {
    if let Some(input) = history
        .iter()
        .rev()
        .find(|row| row.role == "user")
        .map(crate::agent::message_convert::chat_message_to_message)
    {
        context.attachment_placeholders =
            Arc::new(crate::agent::attachments::image_references(&input, &[]));
    }
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
