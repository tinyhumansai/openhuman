//! Core message processing loop for the channel runtime.
//!
//! Contains:
//! * [`channel_has_approval_surface`] — gate controlling per-channel approval
//!   context scoping.
//! * [`try_route_approval_reply`] — intercepts yes/no approval replies before
//!   dispatching a fresh agent turn.
//! * [`process_channel_message`] — full per-message pipeline: typing, ACK
//!   reaction, history, agent turn, draft updates, reply.
//! * [`run_message_dispatch_loop`] — bounded-concurrency worker loop that feeds
//!   messages into [`process_channel_message`].

use crate::core::event_bus::{
    publish_global, request_native_global, DomainEvent, NativeRequestError,
};
use crate::openhuman::agent::bus::{AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::channels::context::{
    build_memory_context, compact_sender_history, conversation_history_key,
    conversation_memory_key, is_context_window_overflow_error, ChannelRuntimeContext,
};
use crate::openhuman::channels::providers::telegram::TELEGRAM_APPROVAL_CLIENT_ID;
use crate::openhuman::channels::routes::{
    get_or_create_provider, get_route_selection, handle_runtime_command_if_needed,
};
use crate::openhuman::channels::traits;
use crate::openhuman::channels::SendMessage;
use crate::openhuman::inference::provider::{self, ChatMessage};
use crate::openhuman::util::truncate_with_ellipsis;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use super::helpers::{
    build_channel_context_block, log_worker_join_result, select_acknowledgment_reaction,
    spawn_scoped_typing_task, REPLY_LOG_TRUNCATE_CHARS,
};
use super::routing::resolve_target_agent;

/// Whether a channel currently has a registered approval surface — i.e.
/// a subscriber that turns `ApprovalRequested` events into chat messages
/// and a way for the user's reply to flow back into the
/// [`ApprovalGate`]. When `true`, the dispatch loop scopes the agent
/// turn in an [`ApprovalChatContext`] so the gate actually fires for
/// `Prompt`-class tools and intercepts yes/no replies for parked
/// approvals.
///
/// Only Telegram has a surface today (sub-issue 2 of #3098). Discord /
/// Slack / iMessage / Mattermost remain in the legacy "no chat context
/// → silently allow" state until each gets a per-channel surface in a
/// follow-up PR; surfacing approvals there without a subscriber would
/// just TTL-deny every parked call, which is worse than the status quo.
///
/// [`ApprovalChatContext`]: crate::openhuman::approval::ApprovalChatContext
/// [`ApprovalGate`]: crate::openhuman::approval::ApprovalGate
pub(crate) fn channel_has_approval_surface(channel: &str) -> bool {
    channel == TELEGRAM_APPROVAL_CLIENT_ID
}

/// If the inbound message is a yes/no reply for a parked approval on
/// this thread, route it to [`ApprovalGate::decide`] and return `true`.
/// Otherwise return `false` so the caller can dispatch the message as a
/// fresh turn (which intentionally cancels any parked approval — the
/// user is redirecting). Mirrors the web channel intercept at
/// `channels/providers/web.rs:493-525`.
///
/// [`ApprovalGate::decide`]: crate::openhuman::approval::ApprovalGate::decide
async fn try_route_approval_reply(msg: &traits::ChannelMessage) -> bool {
    let Some(gate) = crate::openhuman::approval::ApprovalGate::try_global() else {
        return false;
    };
    let thread_id = conversation_history_key(msg);
    let Some(request_id) = gate.pending_for_thread(&thread_id) else {
        return false;
    };
    let Some(decision) = crate::openhuman::approval::parse_approval_reply(&msg.content) else {
        return false;
    };
    match gate.decide(&request_id, decision) {
        Ok(Some(_)) => {
            tracing::info!(
                "[dispatch] routed chat reply to approval gate channel={} thread_id={} request_id={} decision={}",
                msg.channel,
                thread_id,
                request_id,
                decision.as_str()
            );
            true
        }
        Ok(None) => {
            // The request was already decided / cleared between our
            // `pending_for_thread` check and `decide`. Don't claim the
            // intercept; fall through so the reply lands as a normal turn.
            tracing::warn!(
                "[dispatch] approval reply targeted a non-pending request channel={} thread_id={} request_id={} — dispatching as fresh turn",
                msg.channel,
                thread_id,
                request_id
            );
            false
        }
        Err(err) => {
            tracing::warn!(
                "[dispatch] approval gate decide failed channel={} thread_id={} request_id={}: {err}",
                msg.channel,
                thread_id,
                request_id
            );
            false
        }
    }
}

pub(crate) async fn process_channel_message(
    ctx: Arc<ChannelRuntimeContext>,
    msg: traits::ChannelMessage,
) {
    println!(
        "  💬 [{}] from {}: {}",
        msg.channel,
        msg.sender,
        truncate_with_ellipsis(&msg.content, 80)
    );

    publish_global(DomainEvent::ChannelMessageReceived {
        channel: msg.channel.clone(),
        message_id: msg.id.clone(),
        sender: msg.sender.clone(),
        reply_target: msg.reply_target.clone(),
        content: msg.content.clone(),
        thread_ts: msg.thread_ts.clone(),
        workspace_dir: ctx.workspace_dir.as_ref().clone(),
    });

    let target_channel = ctx.channels_by_name.get(&msg.channel).cloned();
    if handle_runtime_command_if_needed(ctx.as_ref(), &msg, target_channel.as_ref()).await {
        return;
    }

    // Sub-issue 2 of #3098: if this channel has an approval surface and the
    // inbound message is a yes/no reply for a parked approval on this same
    // history key, route it to `ApprovalGate::decide` and return — running
    // a fresh agent turn would cancel the parked tool call. Any other text
    // falls through to the normal dispatch (the user is redirecting). Mirrors
    // the same intercept in `channels/providers/web.rs:493-525`.
    if channel_has_approval_surface(&msg.channel) {
        if try_route_approval_reply(&msg).await {
            return;
        }
    }

    // Fire typing indicator as early as possible — before any async I/O — so the
    // user sees feedback immediately regardless of how fast the LLM responds.
    if let Some(channel) = target_channel.as_ref() {
        if let Err(e) = channel.start_typing(&msg.reply_target).await {
            tracing::debug!(
                "[dispatch] Early typing start failed on {}: {e}",
                channel.name()
            );
        }
    }

    // Send a smart acknowledgment reaction immediately so the user knows the message
    // was received and understood. The LLM may override this later by including its
    // own [REACTION:...] marker, which Telegram replaces atomically.
    if let Some(channel) = target_channel.as_ref() {
        if channel.supports_reactions() && msg.thread_ts.is_some() {
            let ack_emoji = select_acknowledgment_reaction(&msg.content);
            tracing::debug!(
                channel = msg.channel,
                emoji = ack_emoji,
                "[dispatch] Sending acknowledgment reaction"
            );
            let react_content = format!("[REACTION:{ack_emoji}]");
            let channel_for_react = Arc::clone(channel);
            let react_msg =
                SendMessage::new(react_content, &msg.reply_target).in_thread(msg.thread_ts.clone());
            tokio::spawn(async move {
                if let Err(e) = channel_for_react.send(&react_msg).await {
                    tracing::debug!("[dispatch] Acknowledgment reaction failed: {e}");
                }
            });
        }
    }

    let history_key = conversation_history_key(&msg);
    let route = get_route_selection(ctx.as_ref(), &history_key);
    let active_provider = match get_or_create_provider(ctx.as_ref(), &route.provider).await {
        Ok(provider) => provider,
        Err(err) => {
            crate::core::observability::report_error(
                &err,
                "channels",
                "provider_init",
                &[
                    ("channel", msg.channel.as_str()),
                    ("provider", route.provider.as_str()),
                ],
            );
            let safe_err = provider::sanitize_api_error(&err.to_string());
            let message = format!(
                "⚠️ Failed to initialize provider `{}`. Please run `/models` to choose another provider.\nDetails: {safe_err}",
                route.provider
            );
            if let Some(channel) = target_channel.as_ref() {
                let _ = channel
                    .send(
                        &SendMessage::new(message, &msg.reply_target)
                            .in_thread(msg.thread_ts.clone()),
                    )
                    .await;
            }
            return;
        }
    };

    let memory_context =
        build_memory_context(ctx.memory.as_ref(), &msg.content, ctx.min_relevance_score).await;

    if ctx.auto_save_memory {
        let autosave_key = conversation_memory_key(&msg);
        let _ = ctx
            .memory
            .store(
                "",
                &autosave_key,
                &msg.content,
                crate::openhuman::memory::MemoryCategory::Conversation,
                None,
            )
            .await;
    }

    let channel_context = build_channel_context_block(&msg);
    let enriched_message = match (memory_context.is_empty(), channel_context.is_empty()) {
        (true, true) => msg.content.clone(),
        (false, true) => format!("{memory_context}{}", msg.content),
        (true, false) => format!("{channel_context}{}", msg.content),
        (false, false) => format!("{memory_context}{channel_context}{}", msg.content),
    };

    println!("  ⏳ Processing message...");
    let started_at = Instant::now();

    // Build history from per-sender conversation cache
    let mut prior_turns = ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&history_key)
        .cloned()
        .unwrap_or_default();

    let mut history = vec![ChatMessage::system(ctx.system_prompt.as_str())];
    history.append(&mut prior_turns);
    history.push(ChatMessage::user(&enriched_message));

    // Determine if this channel supports streaming draft updates
    let use_streaming = target_channel
        .as_ref()
        .is_some_and(|ch| ch.supports_draft_updates());

    // Set up streaming channel if supported
    let (progress_tx, progress_rx) = if use_streaming {
        let (tx, rx) = tokio::sync::mpsc::channel::<AgentProgress>(64);
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    // Send initial draft message if streaming
    let draft_message_id = if use_streaming {
        if let Some(channel) = target_channel.as_ref() {
            match channel
                .send_draft(
                    &SendMessage::new("...", &msg.reply_target).in_thread(msg.thread_ts.clone()),
                )
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::debug!("Failed to send draft on {}: {e}", channel.name());
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Spawn a task to forward streaming progress to draft updates
    let draft_updater = if let (Some(mut rx), Some(draft_id_ref), Some(channel_ref)) = (
        progress_rx,
        draft_message_id.as_deref(),
        target_channel.as_ref(),
    ) {
        let channel = Arc::clone(channel_ref);
        let reply_target = msg.reply_target.clone();
        let draft_id = draft_id_ref.to_string();
        Some(tokio::spawn(async move {
            let mut accumulated = String::new();
            let mut last_thinking_update = None;
            const THINKING_UPDATE_INTERVAL_MS: u128 = 2000;

            while let Some(progress) = rx.recv().await {
                match progress {
                    AgentProgress::TextDelta { delta, .. } => {
                        accumulated.push_str(&delta);
                        if let Err(e) = channel
                            .update_draft(&reply_target, &draft_id, &accumulated)
                            .await
                        {
                            tracing::debug!("Draft update failed: {e}");
                        }
                    }
                    AgentProgress::ThinkingDelta { .. } => {
                        // Suppress thinking text to Telegram; only show a placeholder if we haven't
                        // started receiving the final answer yet.
                        if accumulated.is_empty() {
                            let now = std::time::Instant::now();
                            let should_update = match last_thinking_update {
                                None => true,
                                Some(last) => {
                                    now.duration_since(last).as_millis()
                                        > THINKING_UPDATE_INTERVAL_MS
                                }
                            };

                            if should_update {
                                if let Err(e) = channel
                                    .update_draft(&reply_target, &draft_id, "Thinking...")
                                    .await
                                {
                                    tracing::debug!("Thinking update failed: {e}");
                                }
                                last_thinking_update = Some(now);
                            }
                        }
                    }
                    AgentProgress::ToolCallStarted { tool_name, .. } => {
                        if accumulated.is_empty() {
                            let _ = channel
                                .update_draft(
                                    &reply_target,
                                    &draft_id,
                                    &format!("Working ({})...", tool_name),
                                )
                                .await;
                        }
                    }
                    _ => {}
                }
            }
        }))
    } else {
        None
    };

    let typing_cancellation = target_channel.as_ref().map(|_| CancellationToken::new());
    // Typing was already started early (before memory/provider setup). Here we only
    // spawn the background refresh task that keeps the indicator alive during long turns.
    let typing_task = match (target_channel.as_ref(), typing_cancellation.as_ref()) {
        (Some(channel), Some(token)) => Some(spawn_scoped_typing_task(
            Arc::clone(channel),
            msg.reply_target.clone(),
            token.clone(),
        )),
        _ => None,
    };

    // Dispatch the agentic turn through the native event bus instead of
    // calling `run_tool_call_loop` directly. The agent domain registers
    // an `agent.run_turn` handler at startup (see
    // `crate::openhuman::agent::bus::register_agent_handlers`); this keeps
    // the channel layer free of direct harness imports and makes the
    // agent side mockable in unit tests via a handler override.
    //
    // The agent handler owns the history vector — we `mem::take` the
    // local one to avoid an unnecessary clone; `history` is not read
    // again below.
    // Pick the active agent for this turn (always orchestrator) and
    // synthesise its delegation tool surface. Fresh disk read of
    // `Config::onboarding_completed` happens inside `resolve_target_agent`.
    let scoping = resolve_target_agent(&msg.channel).await;

    // A channel's explicitly-registered `tools_registry` tools are always visible
    // to the model. The resolved agent's visible-tool scope is meant to filter the
    // ambient/builtin tool surface, not to hide tools the channel deliberately
    // handed in for this turn. Without this, a channel that provides a tool
    // outside the resolved agent's `Named` scope (e.g. a test mock, or a custom
    // channel-specific tool) would be filtered out and surfaced to the model as
    // "unknown tool". When the scope is `Wildcard` (`None`), no filter applies.
    let visible_tool_names = scoping.visible_tool_names.map(|mut set| {
        for tool in ctx.tools_registry.iter() {
            set.insert(tool.name().to_string());
        }
        set
    });

    // Non-web channel turns label themselves as `ExternalChannel` so the
    // approval gate's origin-aware decision tree treats the inbound text
    // as untrusted (remote-attacker-controlled). Cron-driven channel
    // deliveries get a `TrustedAutomation { Cron }` label from the
    // scheduler instead and never reach this dispatch path.
    // Per-sender provenance flows into the origin so a co-channel attacker
    // who reads a leaked `quote_id` / approval prompt from a shared Discord /
    // Slack channel cannot use it from their own session — distinct senders
    // produce distinct origins, and the wallet preparer / parked-approval
    // gates compare these for equality before honouring confirmations.
    let turn_origin = crate::openhuman::agent::turn_origin::AgentTurnOrigin::ExternalChannel {
        channel: msg.channel.clone(),
        sender: Some(msg.sender.clone()),
        reply_target: msg.reply_target.clone(),
        message_id: msg.id.clone(),
    };

    let turn_request = AgentTurnRequest {
        provider: Arc::clone(&active_provider),
        history: std::mem::take(&mut history),
        tools_registry: Arc::clone(&ctx.tools_registry),
        provider_name: route.provider.clone(),
        model: route.model.clone(),
        temperature: ctx.temperature,
        silent: true,
        channel_name: msg.channel.clone(),
        multimodal: ctx.multimodal.clone(),
        // Channel-sourced text is untrusted (Slack / Discord / Telegram
        // / WhatsApp / etc. — anyone who can DM the bot can put bytes
        // here). Operator-supplied defaults at `config.multimodal_files`
        // would otherwise let a remote sender smuggle a marker like
        // `[FILE:/etc/passwd]`, `[FILE:/home/<user>/.ssh/id_rsa]`, or
        // `[FILE:.env]` into the agent prompt — `read_local_file`
        // resolves the path with no workspace confinement, so absolute
        // paths exfiltrate server-local files via a follow-up question.
        //
        // Hard-disable file-marker resolution on this path regardless of
        // operator config; the desktop / web-chat path (where the user
        // owns the local filesystem) goes through a different turn
        // builder and keeps the operator default. Mirrors the triage-arm
        // hardening in `agent::triage::evaluator`.
        multimodal_files:
            crate::openhuman::config::MultimodalFileConfig::for_untrusted_channel_input(),
        max_tool_iterations: ctx.max_tool_iterations,
        on_delta: None, // on_progress handles text deltas now
        target_agent_id: scoping.target_agent_id,
        visible_tool_names,
        extra_tools: scoping.extra_tools,
        on_progress: progress_tx,
        origin: turn_origin,
    };
    tracing::debug!(
        channel = %msg.channel,
        provider = %route.provider,
        model = %route.model,
        "[channels::dispatch] dispatching {AGENT_RUN_TURN_METHOD} via native bus"
    );
    let agent_call = async {
        request_native_global::<AgentTurnRequest, AgentTurnResponse>(
            AGENT_RUN_TURN_METHOD,
            turn_request,
        )
        .await
        .map(|resp| resp.text)
        .map_err(|err| match err {
            // Unwrap handler-returned errors so the underlying
            // message (e.g. "Agent exceeded maximum tool iterations")
            // flows through without being wrapped in bus-transport
            // layer prose. The error-formatting path downstream
            // treats this `anyhow::Error` the same way it did before
            // the bus migration.
            NativeRequestError::HandlerFailed { message, .. } => {
                anyhow::anyhow!(message)
            }
            // Bus-level errors (UnregisteredHandler / TypeMismatch /
            // NotInitialized) surface with their full Display so
            // startup wiring bugs are immediately obvious in logs.
            other => anyhow::anyhow!("[agent.run_turn dispatch] {other}"),
        })
    };
    // Sub-issue 2 of #3098: scope the agent turn in an `ApprovalChatContext`
    // for channels that have a registered approval surface — currently
    // Telegram only via `TelegramApprovalSurfaceSubscriber`. Without this
    // scope the gate's "no chat context → allow straight through" branch
    // (`approval/gate.rs:219-231`) silently bypasses every `Prompt`-class
    // tool call, voiding the `supervised` autonomy tier on the channel.
    // Discord / Slack / iMessage / Mattermost stay in the legacy bypass
    // until each gets its own approval surface in a follow-up PR.
    let llm_result = tokio::time::timeout(Duration::from_secs(ctx.message_timeout_secs), async {
        if channel_has_approval_surface(&msg.channel) {
            let approval_ctx = crate::openhuman::approval::ApprovalChatContext {
                thread_id: history_key.clone(),
                client_id: msg.channel.clone(),
            };
            crate::openhuman::approval::APPROVAL_CHAT_CONTEXT
                .scope(approval_ctx, agent_call)
                .await
        } else {
            agent_call.await
        }
    })
    .await;

    // Wait for draft updater to finish
    if let Some(handle) = draft_updater {
        let _ = handle.await;
    }

    if let Some(token) = typing_cancellation.as_ref() {
        token.cancel();
    }
    if let Some(handle) = typing_task {
        log_worker_join_result(handle.await);
    }

    let (success, response_text) = match llm_result {
        Ok(Ok(response)) => {
            // Save user + assistant turn to per-sender history
            {
                let mut histories = ctx
                    .conversation_histories
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let turns = histories.entry(history_key).or_default();
                turns.push(ChatMessage::user(&enriched_message));
                turns.push(ChatMessage::assistant(&response));
                // Trim to MAX_CHANNEL_HISTORY (keep recent turns)
                while turns.len() > crate::openhuman::channels::context::MAX_CHANNEL_HISTORY {
                    turns.remove(0);
                }
            }
            println!(
                "  🤖 Reply ({}ms): {}",
                started_at.elapsed().as_millis(),
                truncate_with_ellipsis(&response, REPLY_LOG_TRUNCATE_CHARS)
            );
            if let Some(channel) = target_channel.as_ref() {
                if let Some(ref draft_id) = draft_message_id {
                    if let Err(e) = channel
                        .finalize_draft(
                            &msg.reply_target,
                            draft_id,
                            &response,
                            msg.thread_ts.as_deref(),
                        )
                        .await
                    {
                        tracing::warn!("Failed to finalize draft: {e}; sending as new message");
                        let _ = channel
                            .send(
                                &SendMessage::new(&response, &msg.reply_target)
                                    .in_thread(msg.thread_ts.clone()),
                            )
                            .await;
                    }
                } else if let Err(e) = channel
                    .send(
                        &SendMessage::new(&response, &msg.reply_target)
                            .in_thread(msg.thread_ts.clone()),
                    )
                    .await
                {
                    eprintln!("  ❌ Failed to reply on {}: {e}", channel.name());
                }
            }
            (true, response)
        }
        Ok(Err(e)) => {
            if is_context_window_overflow_error(&e) {
                let compacted = compact_sender_history(ctx.as_ref(), &history_key);
                let error_text = if compacted {
                    "⚠️ Context window exceeded for this conversation. I compacted recent history and kept the latest context. Please resend your last message."
                } else {
                    "⚠️ Context window exceeded for this conversation. Please resend your last message."
                };
                eprintln!(
                    "  ⚠️ Context window exceeded after {}ms; sender history compacted={}",
                    started_at.elapsed().as_millis(),
                    compacted
                );
                if let Some(channel) = target_channel.as_ref() {
                    if let Some(ref draft_id) = draft_message_id {
                        let _ = channel
                            .finalize_draft(
                                &msg.reply_target,
                                draft_id,
                                error_text,
                                msg.thread_ts.as_deref(),
                            )
                            .await;
                    } else {
                        let _ = channel
                            .send(
                                &SendMessage::new(error_text, &msg.reply_target)
                                    .in_thread(msg.thread_ts.clone()),
                            )
                            .await;
                    }
                }

                publish_global(DomainEvent::ChannelMessageProcessed {
                    channel: msg.channel.clone(),
                    message_id: msg.id.clone(),
                    sender: msg.sender.clone(),
                    reply_target: msg.reply_target.clone(),
                    content: msg.content.clone(),
                    thread_ts: msg.thread_ts.clone(),
                    response: error_text.to_string(),
                    elapsed_ms: started_at.elapsed().as_millis() as u64,
                    success: false,
                    workspace_dir: ctx.workspace_dir.as_ref().clone(),
                });
                return;
            }

            let error_response = format!("⚠️ Error: {e}");
            eprintln!(
                "  ❌ LLM error after {}ms: {e}",
                started_at.elapsed().as_millis()
            );
            // The typed `AgentError` is flattened to a `String` at the
            // native-bus boundary (`agent::bus` map_err → `e.to_string()`),
            // so the downcast that works in `Agent::run_single` is not an
            // option here — fall back to canonical-phrase substring match.
            // The max-tool-iterations cap is a deterministic agent-state
            // outcome and is already surfaced to the user as the
            // chat-rendered "⚠️ Error: …" message just above. Skip the
            // Sentry funnel (OPENHUMAN-TAURI-98) and emit `log::info!`
            // instead — `Err` propagation through the surrounding match
            // arm is unchanged.
            if crate::openhuman::agent::error::is_max_iterations_error(&e.to_string()) {
                log::info!(
                    target: "channels",
                    "[channels.dispatch] suppressed Sentry emission for max-iteration cap \
                     channel={} provider={} message={}",
                    msg.channel.as_str(),
                    route.provider.as_str(),
                    e
                );
            } else {
                // Route through `report_error_or_expected` so
                // transient-upstream provider HTTP errors that bubbled
                // up via `agent.run_single` (`OpenHuman API error
                // (502 Bad Gateway): …`) get demoted via
                // `is_transient_upstream_http_message` — the agent
                // re-emit at the dispatch layer was previously
                // unconditionally calling `report_error`, which firehoses
                // Sentry under `domain="channels"` even though the same
                // chain was already classified at the provider + agent
                // layers (OPENHUMAN-TAURI-4F ~157ev / -1C ~87ev / -8F
                // ~39ev: provider 5xx that the reliable layer retried
                // and exhausted, then the channels layer re-reported as
                // a fresh per-attempt event). Genuine bugs (404 / 500
                // / unrelated agent failures) still surface — the
                // classifier only demotes the canonical transient
                // shapes documented in
                // `crate::core::observability::expected_error_kind`.
                crate::core::observability::report_error_or_expected(
                    &e,
                    "channels",
                    "dispatch_llm_error",
                    &[
                        ("channel", msg.channel.as_str()),
                        ("provider", route.provider.as_str()),
                    ],
                );
            }
            if let Some(channel) = target_channel.as_ref() {
                if let Some(ref draft_id) = draft_message_id {
                    let _ = channel
                        .finalize_draft(
                            &msg.reply_target,
                            draft_id,
                            &error_response,
                            msg.thread_ts.as_deref(),
                        )
                        .await;
                } else {
                    let _ = channel
                        .send(
                            &SendMessage::new(&error_response, &msg.reply_target)
                                .in_thread(msg.thread_ts.clone()),
                        )
                        .await;
                }
            }
            (false, error_response)
        }
        Err(_) => {
            let timeout_msg = format!("LLM response timed out after {}s", ctx.message_timeout_secs);
            eprintln!(
                "  ❌ {} (elapsed: {}ms)",
                timeout_msg,
                started_at.elapsed().as_millis()
            );
            crate::core::observability::report_error(
                timeout_msg.as_str(),
                "channels",
                "dispatch_llm_timeout",
                &[
                    ("channel", msg.channel.as_str()),
                    ("timeout_secs", &ctx.message_timeout_secs.to_string()),
                ],
            );
            let error_text =
                "⚠️ Request timed out while waiting for the model. Please try again.".to_string();
            if let Some(channel) = target_channel.as_ref() {
                if let Some(ref draft_id) = draft_message_id {
                    let _ = channel
                        .finalize_draft(
                            &msg.reply_target,
                            draft_id,
                            &error_text,
                            msg.thread_ts.as_deref(),
                        )
                        .await;
                } else {
                    let _ = channel
                        .send(
                            &SendMessage::new(&error_text, &msg.reply_target)
                                .in_thread(msg.thread_ts.clone()),
                        )
                        .await;
                }
            }
            (false, error_text)
        }
    };

    publish_global(DomainEvent::ChannelMessageProcessed {
        channel: msg.channel.clone(),
        message_id: msg.id.clone(),
        sender: msg.sender.clone(),
        reply_target: msg.reply_target.clone(),
        content: msg.content.clone(),
        thread_ts: msg.thread_ts.clone(),
        response: response_text,
        elapsed_ms: started_at.elapsed().as_millis() as u64,
        success,
        workspace_dir: ctx.workspace_dir.as_ref().clone(),
    });
}

pub(crate) async fn run_message_dispatch_loop(
    mut rx: tokio::sync::mpsc::Receiver<traits::ChannelMessage>,
    ctx: Arc<ChannelRuntimeContext>,
    max_in_flight_messages: usize,
) {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_in_flight_messages));
    let mut workers = tokio::task::JoinSet::new();

    while let Some(msg) = rx.recv().await {
        let permit = match Arc::clone(&semaphore).acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => break,
        };

        let worker_ctx = Arc::clone(&ctx);
        workers.spawn(async move {
            let _permit = permit;
            process_channel_message(worker_ctx, msg).await;
        });

        while let Some(result) = workers.try_join_next() {
            log_worker_join_result(result);
        }
    }

    while let Some(result) = workers.join_next().await {
        log_worker_join_result(result);
    }
}
