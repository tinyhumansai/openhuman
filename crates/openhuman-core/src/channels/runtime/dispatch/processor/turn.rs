//! The per-message pipeline: typing indicator, ACK reaction, durable-session
//! dispatch through the shared runner, and final reply delivery.

use crate::channels::context::{is_context_window_overflow_error, ChannelRuntimeContext};
#[cfg(test)]
use crate::channels::routes::get_or_create_turn_model_source;
use crate::channels::routes::get_route_selection;
use crate::channels::SendMessage;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::util::truncate_with_ellipsis;
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use super::super::helpers::{
    build_channel_context_block, log_worker_join_result, spawn_typing_task,
    REPLY_LOG_TRUNCATE_CHARS,
};
use super::super::host_agent::{self, HostRoute};
use super::admit::{admit_runtime_message, AdmittedInbound};
#[cfg(test)]
use super::admit::{channel_inbound_preparation_request, inbound_console_preview};
use super::approval::channel_has_approval_surface;
use super::RuntimeChannelMessage;

pub(crate) async fn process_channel_runtime_message(
    ctx: Arc<ChannelRuntimeContext>,
    runtime_msg: RuntimeChannelMessage,
) {
    if let Some(admitted) = admit_runtime_message(Arc::clone(&ctx), runtime_msg).await {
        run_admitted_turn(ctx, admitted, None).await;
    }
}

/// Run the agent turn for one admitted message (or one merged batch).
///
/// `handoff` is released right before the turn is handed to the shared turn
/// runner (or when the turn ends early), which lets the dispatch loop start
/// the next batch of the same conversation after this one in receive order.
pub(crate) async fn run_admitted_turn(
    ctx: Arc<ChannelRuntimeContext>,
    admitted: AdmittedInbound,
    handoff: Option<tokio::sync::oneshot::Sender<()>>,
) {
    let AdmittedInbound {
        msg,
        envelope,
        history_key,
        canonical_session_key,
        session_key,
        tenant,
        native_target,
        typed_envelope_input,
        imported_history,
        legacy_history,
    } = admitted;
    let target_channel = ctx.channels_by_name.get(&msg.channel).cloned();
    // A channel bound to a host agent nobody answers for is refused before
    // any typing or provider work; see `host_agent`.
    let bound_agent = match host_agent::route(ctx.as_ref(), &msg.channel) {
        HostRoute::Unbound => None,
        HostRoute::Agent(host) => Some(host),
        HostRoute::Missing(agent_id) => {
            host_agent::refuse_missing_agent(
                ctx.as_ref(),
                &msg,
                target_channel.as_ref(),
                &agent_id,
            )
            .await;
            return;
        }
    };

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
    // Keep the reaction owned by this worker so logout also aborts a pending send.
    let mut _acknowledgment_task = None;
    if let Some(channel) = target_channel.as_ref() {
        if channel.supports_reactions() && msg.thread_ts.is_some() {
            let ack_result = crate::channels::context::module_client(&ctx)
                .await
                .call_confidential::<tinychannels_bus::module_control::ChannelControlResult>(
                    "tinychannels",
                    tinychannels_bus::methods::PREPARE_CHANNEL_CONTROL,
                    (
                        tinychannels_bus::module_control::ChannelControlRequest::Acknowledgment {
                            content: msg.content.clone(),
                        },
                    ),
                )
                .await;
            let ack_emoji = match ack_result {
                Ok(tinychannels_bus::module_control::ChannelControlResult::Acknowledgment(
                    emoji,
                )) => Some(emoji),
                Ok(_) => {
                    crate::modules::report::malformed_reply("tinychannels");
                    None
                }
                Err(_) => None,
            };
            if let Some(ack_emoji) = ack_emoji {
                tracing::debug!(
                    channel = msg.channel,
                    emoji = ack_emoji,
                    "[dispatch] Sending acknowledgment reaction"
                );
                let react_content = format!("[REACTION:{ack_emoji}]");
                let channel_for_react = Arc::clone(channel);
                let react_msg = SendMessage::new(react_content, &msg.reply_target)
                    .in_thread(msg.thread_ts.clone());
                _acknowledgment_task = Some(AbortOnDropHandle::new(tokio::spawn(async move {
                    if let Err(e) = channel_for_react
                        .send_with_outbound_intent(&react_msg)
                        .await
                    {
                        tracing::debug!("[dispatch] Acknowledgment reaction failed: {e}");
                    }
                })));
            }
        }
    }

    let route = get_route_selection(ctx.as_ref(), &history_key);
    #[cfg(test)]
    let active_turn_model_source = if ctx.config.is_none() {
        match get_or_create_turn_model_source(ctx.as_ref(), &route.provider).await {
            Ok(source) => Some(source),
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
                let safe_err = tinyinference_core::sanitize::sanitize_api_error(&err.to_string());
                let message = format!(
                    "⚠️ Failed to initialize provider `{}`. Please run `/models` to choose another provider.\nDetails: {safe_err}",
                    route.provider
                );
                if let Some(channel) = target_channel.as_ref() {
                    let _ = channel
                        .send_with_outbound_intent(
                            &SendMessage::new(message, &msg.reply_target)
                                .in_thread(msg.thread_ts.clone()),
                        )
                        .await;
                }
                return;
            }
        }
    } else {
        None
    };

    // No per-turn memory recall or autosave: memory v2 reaches the session
    // through `context.md` (injected by the session host) and ingests the
    // committed turn from `ConversationTurnCommitted`.
    let channel_context = build_channel_context_block(&msg);
    let enriched_message = if channel_context.is_empty() {
        msg.content.clone()
    } else {
        format!("{channel_context}{}", msg.content)
    };

    println!("  ⏳ Processing message...");
    let started_at = Instant::now();

    // The persisted TinyAgents transcript, bound below by `session_key`, is
    // the sole history source. The injected-model graph remains available to
    // unit fixtures only; a production channel turn without a host config is
    // refused rather than routed through a second execution graph.

    let typing_cancellation = target_channel.as_ref().map(|_| CancellationToken::new());
    let _typing_cancel_on_drop = typing_cancellation
        .as_ref()
        .map(|token| token.clone().drop_guard());
    // Typing was already started early (before memory/provider setup). Here we only
    // spawn the background refresh task that keeps the indicator alive during long turns.
    let typing_task = match (target_channel.as_ref(), typing_cancellation.as_ref()) {
        (Some(channel), Some(token)) => Some(AbortOnDropHandle::new(spawn_typing_task(
            &ctx.dispatch_ctx(),
            Arc::clone(channel),
            msg.reply_target.clone(),
            token.clone(),
        ))),
        _ => None,
    };

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
    // Sub-issue 2 of #3098: give the agent turn an `ApprovalTurnContext` for
    // every channel with the `chat_approvals` capability (served by
    // `ChannelApprovalSurfaceSubscriber`). Without a chat context the gate's
    // "no chat context -> allow straight through" branch silently bypasses
    // every `Prompt`-class tool call, voiding the `supervised` autonomy tier
    // on the channel.
    let approval = if channel_has_approval_surface(&msg.channel) {
        crate::security::approval::ApprovalTurnContext::chat(
            crate::security::approval::ApprovalChatContext {
                thread_id: history_key.clone(),
                client_id: msg.channel.clone(),
                request_id: Some(msg.id.clone()),
            },
        )
    } else {
        crate::security::approval::ApprovalTurnContext::default()
    };

    // A channel bound to a host agent (`agent.channel_agents`) runs as that
    // agent; every other channel goes over the bus to the orchestrator.
    let mut configured_agent_error = None;
    let selected_host = match bound_agent {
        Some(host) => Some(*host),
        None if ctx.config.is_some() => {
            match host_agent::configured_agent(ctx.as_ref(), &route).await {
                Ok(host) => Some(host),
                Err(error) => {
                    tracing::warn!(channel = %msg.channel, %error, "[dispatch] configured channel agent could not be built");
                    configured_agent_error = Some(error);
                    None
                }
            }
        }
        None => None,
    };
    let native_preparation = if typed_envelope_input {
        if let Some(host) = selected_host.as_ref() {
            let host_dispatch = crate::core::runtime::DispatchCtx::new(Arc::clone(&host.context));
            match super::native_media::resolve_typed_message(
                &ctx,
                &host_dispatch,
                &host.config,
                &envelope,
                &canonical_session_key,
                &channel_context,
            )
            .await
            {
                Ok(input) => Some(Ok(input)),
                Err(error) => Some(Err(error)),
            }
        } else {
            Some(Err("native media has no selected host agent".to_owned()))
        }
    } else {
        None
    };
    let (native_message_input, native_staged_parts, native_staged_lease, native_preparation_error) =
        match native_preparation {
            Some(Ok(Some(prepared))) => (
                Some(prepared.message),
                prepared.staged_parts,
                Some(prepared.staged_lease),
                None,
            ),
            Some(Ok(None)) | None => (None, Vec::new(), None, None),
            Some(Err(error)) => (None, Vec::new(), None, Some(error)),
        };
    let channel_session = selected_host.as_ref().and_then(|host| {
        let account_id = envelope
            .channel
            .account_id
            .as_deref()
            .filter(|account| !account.trim().is_empty())?;
        Some(tinyflows_bus::schedule::ChannelSessionIdentity {
            tenant_key: crate::core::runtime::tenant::tenant_key(&tenant, ""),
            agent_id: host.definition.id.clone(),
            session_key: canonical_session_key.clone(),
            provider: envelope.channel.id.clone(),
            account_id: Some(account_id.to_owned()),
            conversation: tinyflows_bus::schedule::ChannelConversationIdentity {
                kind: envelope.conversation.kind.as_session_segment().to_string(),
                id: envelope.conversation.id.clone(),
                scope_id: envelope.conversation.scope_id.clone(),
                parent_id: envelope.conversation.parent_id.clone(),
                thread_id: envelope.conversation.thread_id.clone(),
                topic_id: envelope.conversation.topic_id.clone(),
            },
        })
    });
    let turn_origin = crate::security::turn_origin::AgentTurnOrigin::ExternalChannel {
        sender_name: msg.sender_name.clone(),
        channel: msg.channel.clone(),
        sender: Some(msg.sender.clone()),
        reply_target: msg.reply_target.clone(),
        message_id: msg.id.clone(),
        history_key: Some(history_key.clone()),
        channel_session,
    };
    drop(handoff);
    let execution = match selected_host {
        Some(host) => {
            let request_id = format!("{}:{}:{}", msg.channel, msg.id, uuid::Uuid::new_v4());
            let trusted = Arc::new(crate::agent::turn_host::TrustedAgentTurn {
                host_agent: host,
                channel: msg.channel.clone(),
                session_key: session_key.clone(),
                event_session_id: history_key.clone(),
                origin: turn_origin.clone(),
                approval: approval.clone(),
                relay_thread: ctx.relay_thread.clone(),
                imported_history,
                legacy_history,
                native_target: native_target.clone(),
            });
            super::super::native_runner::run(
                &ctx.dispatch_ctx(),
                session_key.clone(),
                request_id,
                enriched_message.clone(),
                ctx.message_timeout_secs,
                native_message_input,
                native_staged_parts,
                native_staged_lease,
                Some(envelope.message_id.clone()),
                native_preparation_error,
                trusted,
            )
            .await
        }
        None if configured_agent_error.is_some() => {
            let error = configured_agent_error.expect("checked configured-agent failure");
            super::super::native_runner::NativeTurnExecution {
                result: Err(error),
                delivery_succeeded: None,
                queued: false,
            }
        }
        #[cfg(test)]
        None => {
            let timeout_secs = ctx
                .message_timeout_secs
                .max(tinychannels_bus::module_runtime::MIN_CHANNEL_MESSAGE_TIMEOUT_SECS);
            let result = match tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                super::bus_turn::dispatch_test_turn(
                    ctx.as_ref(),
                    &msg,
                    &route,
                    &enriched_message,
                    active_turn_model_source,
                    None,
                    turn_origin,
                    approval,
                ),
            )
            .await
            {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(_) => Err(crate::web_chat::turn_timeout_error_message(timeout_secs)),
            };
            super::super::native_runner::NativeTurnExecution {
                result,
                delivery_succeeded: None,
                queued: false,
            }
        }
        #[cfg(not(test))]
        None => super::super::native_runner::NativeTurnExecution {
            result: Err("channel turn has no configured host agent".to_owned()),
            delivery_succeeded: None,
            queued: false,
        },
    };
    if execution.queued {
        return;
    }
    let native_delivery_succeeded = execution.delivery_succeeded.unwrap_or(false);
    let llm_result = execution.result.map_err(anyhow::Error::msg);

    if let Some(token) = typing_cancellation.as_ref() {
        token.cancel();
    }
    if let Some(handle) = typing_task {
        log_worker_join_result(handle.await);
    }

    let (success, response_text, response_provider, response_model) = match llm_result {
        Ok(response) => {
            let resolved_provider = response
                .resolved_provider
                .clone()
                .unwrap_or_else(|| route.provider.clone());
            let resolved_model = response
                .resolved_model
                .clone()
                .unwrap_or_else(|| route.model.clone());
            let response_text = response.text;
            println!(
                "  🤖 Reply ({}ms): {}",
                started_at.elapsed().as_millis(),
                truncate_with_ellipsis(&response_text, REPLY_LOG_TRUNCATE_CHARS)
            );
            let delivered = if native_target.is_some() {
                native_delivery_succeeded
            } else if let Some(channel) = target_channel.as_ref() {
                match channel
                    .send_with_outbound_intent(
                        &SendMessage::new(&response_text, &msg.reply_target)
                            .in_thread(msg.thread_ts.clone()),
                    )
                    .await
                {
                    Ok(()) => true,
                    Err(e) => {
                        eprintln!("  ❌ Failed to reply on {}: {e}", channel.name());
                        false
                    }
                }
            } else {
                false
            };
            (delivered, response_text, resolved_provider, resolved_model)
        }
        Err(e) => {
            if is_context_window_overflow_error(&ctx, &e)
                .await
                .unwrap_or(false)
            {
                let error_text = "⚠️ Context window exceeded for this conversation. Please resend your last message.";
                eprintln!(
                    "  ⚠️ Context window exceeded after {}ms",
                    started_at.elapsed().as_millis()
                );
                if native_target.is_none() {
                    if let Some(channel) = target_channel.as_ref() {
                        let _ = channel
                            .send_with_outbound_intent(
                                &SendMessage::new(error_text, &msg.reply_target)
                                    .in_thread(msg.thread_ts.clone()),
                            )
                            .await;
                    }
                }

                BUS.publish(DomainEvent::ChannelMessageProcessed {
                    channel: msg.channel.clone(),
                    message_id: msg.id.clone(),
                    sender: msg.sender.clone(),
                    reply_target: msg.reply_target.clone(),
                    content: msg.content.clone(),
                    thread_ts: msg.thread_ts.clone(),
                    response: error_text.to_string(),
                    provider: route.provider.clone(),
                    model: route.model.clone(),
                    elapsed_ms: started_at.elapsed().as_millis() as u64,
                    success: false,
                    workspace_dir: ctx.workspace_dir.as_ref().clone(),
                });
                return;
            }

            let classified = crate::web_chat::classify_inference_error(&e.to_string());
            let error_response = classified.message;
            eprintln!(
                "  ❌ LLM error after {}ms: {e}",
                started_at.elapsed().as_millis()
            );
            // The typed `AgentError` is flattened to a `String` at the
            // native-bus boundary (`agent::bus` map_err → `e.to_string()`),
            // so the downcast that works in `OpenHumanSessionHost::run_single` is not an
            // option here — fall back to canonical-phrase substring match.
            // The max-tool-iterations cap is a deterministic agent-state
            // outcome and is already surfaced to the user as the
            // chat-rendered "⚠️ Error: …" message just above. Skip the
            // Sentry funnel (OPENHUMAN-TAURI-98) and emit `log::info!`
            // instead — `Err` propagation through the surrounding match
            // arm is unchanged.
            if tinyagents_harness::agent_error::is_max_iterations_error(&e.to_string()) {
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
            if native_target.is_none() {
                if let Some(channel) = target_channel.as_ref() {
                    let _ = channel
                        .send_with_outbound_intent(
                            &SendMessage::new(&error_response, &msg.reply_target)
                                .in_thread(msg.thread_ts.clone()),
                        )
                        .await;
                }
            }
            (
                false,
                error_response,
                route.provider.clone(),
                route.model.clone(),
            )
        }
    };

    BUS.publish(DomainEvent::ChannelMessageProcessed {
        channel: msg.channel.clone(),
        message_id: msg.id.clone(),
        sender: msg.sender.clone(),
        reply_target: msg.reply_target.clone(),
        content: msg.content.clone(),
        thread_ts: msg.thread_ts.clone(),
        response: response_text,
        provider: response_provider,
        model: response_model,
        elapsed_ms: started_at.elapsed().as_millis() as u64,
        success,
        workspace_dir: ctx.workspace_dir.as_ref().clone(),
    });
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
