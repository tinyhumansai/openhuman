//! The `ChannelInboundSubscriber` event handler: runs the agent loop for an
//! inbound channel message and streams the reply back through the REST API.
//!
//! The web-chat event loop is host-owned; the progressive delivery it drives
//! (draft, thinking and filler bubbles, finalize) is
//! `tinychannels::delivery::progressive::ProgressiveReply` over
//! [`BackendProgressiveSender`].

use super::delivery::BackendProgressiveSender;
use crate::core::events::DomainEvent;
use async_trait::async_trait;
use std::sync::Arc;
use tinybus::EventHandler;
use tinychannels::channel::{derive_inbound_client_id, derive_inbound_thread_id};
use tinychannels::delivery::progressive::{
    ProgressiveReply, EDIT_FLUSH_INTERVAL, FILLER_INTERVAL, TYPING_REFRESH_INTERVAL,
};

/// Subscribes to `ChannelInboundMessage` events and runs the agent loop,
/// sending replies back to the originating channel via the backend REST API.
pub struct ChannelInboundSubscriber;

impl Default for ChannelInboundSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelInboundSubscriber {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for ChannelInboundSubscriber {
    fn name(&self) -> &str {
        "channel::inbound_handler"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["channel"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ChannelInboundMessage {
            event_name: _,
            channel,
            message,
            sender,
            reply_target,
            thread_ts,
            raw_data: _,
        } = event
        else {
            return;
        };

        tracing::info!(
            "[channel-inbound] received message from channel='{}' sender={} len={}",
            channel,
            sender.as_deref().unwrap_or("<unknown>"),
            message.len()
        );

        // Discord's backend supplies the author's id for every forwarded DM.
        // If that contract is broken, the legacy channel-only key would let
        // different users share one chat session and its prepared wallet quote.
        if channel.split(':').next() == Some("discord")
            && sender
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            tracing::warn!("[channel-inbound] dropping Discord message without sender id");
            return;
        }

        // Mirror `channels::context::conversation_history_key`: the inbound
        // path must key on `(channel, sender, reply_target, thread_ts)` —
        // not channel alone — or distinct participants in a shared
        // Discord / Slack channel get collapsed into one cached agent
        // session, and the second sender resumes the first's in-flight
        // state (including any prepared wallet quote).
        //
        // Legacy publishers that don't fill in `sender` fall back to the
        // old channel-only key so existing single-DM flows keep working.
        let thread_id = derive_inbound_thread_id(
            channel,
            sender.as_deref(),
            reply_target.as_deref(),
            thread_ts.as_deref(),
        );
        // Per-sender client_id so the `AGENT_TURN_ORIGIN.WebChat.client_id`
        // and the wallet `QuoteOwner.client_id` paired with it differ across
        // distinct senders in the same shared channel. The thread_id is
        // already per-sender via `derive_inbound_thread_id`, and the
        // wallet/approval gates compare both halves of the (thread_id,
        // client_id) owner pair for equality — but a single shared
        // `client_id="inbound"` collapses the surface for any downstream
        // consumer that keys on client_id alone (audit logs, future
        // session-scoped caches, etc.). Build a stable per-sender label
        // here so the surface stays segregated end-to-end.
        let client_id = derive_inbound_client_id(channel, sender.as_deref());

        let mut event_rx = crate::web_chat::subscribe_web_channel_events();
        let mut reply = ProgressiveReply::new(Arc::new(BackendProgressiveSender), channel.clone());

        let request_id = match crate::web_chat::start_chat(
            &client_id,
            &thread_id,
            message,
            None,
            None,
            None,
            None,
            crate::web_chat::ChatRequestMetadata {
                // Tag inbound provider messages so traces classify as
                // run:channel_inbound instead of interactive chat.
                source: Some("channel_inbound".to_string()),
                ..Default::default()
            },
        )
        .await
        {
            Ok(rid) => {
                tracing::debug!(
                    "[channel-inbound] agent started request_id={} thread={}",
                    rid,
                    thread_id
                );
                rid
            }
            Err(err) => {
                tracing::error!("[channel-inbound] start_chat failed: {}", err);
                reply
                    .send_reply(&format!("Sorry, I couldn't process your message: {err}"))
                    .await;
                return;
            }
        };

        let timeout = tokio::time::Duration::from_secs(180);
        let deadline = tokio::time::Instant::now() + timeout;

        // ── Progressive-edit timer ────────────────────────────────────
        // Buffered text/thinking deltas flush as edits on this timer. The
        // driver latches into atomic-final delivery on channels or backends
        // that cannot edit.
        let mut edit_timer = tokio::time::interval(EDIT_FLUSH_INTERVAL);
        edit_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Don't fire immediately; wait for the first tick.
        edit_timer.tick().await;

        // ── Typing indicator ──────────────────────────────────────────
        // Telegram's `sendChatAction` keeps the "typing…" UI alive for
        // ~5s, so re-send every 4s while the turn is in flight. The first
        // call fires immediately.
        let mut typing_timer = tokio::time::interval(TYPING_REFRESH_INTERVAL);
        typing_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Fire immediately on first tick so the indicator shows up as
        // soon as the inbound message is received.
        reply.typing_tick().await;
        typing_timer.tick().await; // consume the immediate tick

        // ── Filler messages ──────────────────────────────────────────
        // Once progressive edits + thinking streams go quiet (backend
        // doesn't support PATCH, reasoning has finished, etc.) the user
        // can wait 30–90 s seeing no fresh activity. Post a short filler
        // every FILLER_INTERVAL so the chat keeps moving. All filler ids
        // are tracked in `StreamingState.filler_message_ids` and deleted
        // by `ProgressiveReply::finalize` once the real response is on screen.
        let mut filler_timer = tokio::time::interval(FILLER_INTERVAL);
        filler_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        filler_timer.tick().await; // consume the immediate tick — first filler fires after FILLER_INTERVAL

        loop {
            tokio::select! {
                event = event_rx.recv() => {
                    match event {
                        Ok(ev) if ev.request_id == request_id => {
                            match ev.event.as_str() {
                                "text_delta" => {
                                    if let Some(delta) = ev.delta.as_ref() {
                                        reply.on_text_delta(delta);
                                    }
                                }
                                "tool_call" => {
                                    if let Some(ref name) = ev.tool_name {
                                        reply.on_tool_call(name);
                                    }
                                }
                                "tool_result" => {
                                    if let Some(ref name) = ev.tool_name {
                                        reply.on_tool_result(name, ev.success.unwrap_or(true));
                                    }
                                }
                                "thinking_delta" => {
                                    if let Some(delta) = ev.delta.as_ref() {
                                        reply.on_thinking_delta(delta);
                                    }
                                }
                                "chat_done" | "chat:done" => {
                                    let full_response = ev.full_response.unwrap_or_default();
                                    // Even when the agent produced no visible
                                    // text, we must close out any draft we
                                    // already posted — otherwise the user is
                                    // left staring at a stale "_working…_"
                                    // message indefinitely.
                                    let reply_text = if full_response.trim().is_empty() {
                                        tracing::warn!(
                                            "[channel-inbound] agent returned empty response — finalizing draft with fallback",
                                        );
                                        "(No response from agent.)"
                                    } else {
                                        full_response.as_str()
                                    };
                                    tracing::info!(
                                        "[channel-inbound] agent done, replying to channel='{}' len={} streamed_msg_id={:?}",
                                        channel,
                                        reply_text.len(),
                                        reply.state().message_id,
                                    );
                                    // If we've been streaming progressive edits, replace
                                    // the outbound message with the final canonical text.
                                    // Otherwise send a fresh message atomically.
                                    reply.finalize(reply_text).await;
                                    return;
                                }
                                "chat_error" | "chat:error" => {
                                    let err_msg = ev.message.unwrap_or_else(|| "unknown error".to_string());
                                    tracing::error!("[channel-inbound] agent error: {}", err_msg);
                                    reply
                                        .finalize(&format!("Sorry, I encountered an error: {err_msg}"))
                                        .await;
                                    return;
                                }
                                // New terminal event (see web_chat::ops::channel_ops /
                                // start_chat) — emitted alongside
                                // `chat_error{error_type:"cancelled"}` for one
                                // release. That legacy event already returns
                                // above, ending this loop before `chat_cancelled`
                                // for the same request_id would be observed, so
                                // this arm only fires standalone (a cancel path
                                // that stops emitting the legacy event, or one
                                // that never did — e.g. the parallel-turn
                                // cooperative-cancel path) and never double-ends
                                // a turn already finalized by `chat_error`.
                                "chat_cancelled" => {
                                    tracing::info!(
                                        "[channel-inbound] turn cancelled reason={:?}",
                                        ev.cancel_reason
                                    );
                                    reply.finalize("Cancelled.").await;
                                    return;
                                }
                                _ => {}
                            }
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("[channel-inbound] event bus lagged, skipped {} events", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::error!("[channel-inbound] event bus closed unexpectedly");
                            return;
                        }
                    }
                }
                // Draft/thinking/filler bubbles need edit+delete support; the
                // driver skips them on channels without it (Discord) so they
                // never leave un-cleanable placeholder messages.
                _ = edit_timer.tick() => reply.edit_tick().await,
                _ = typing_timer.tick() => reply.typing_tick().await,
                _ = filler_timer.tick() => reply.filler_tick().await,
                _ = tokio::time::sleep_until(deadline) => {
                    tracing::error!("[channel-inbound] agent timed out after {}s", timeout.as_secs());
                    reply.finalize("Sorry, the request timed out.").await;
                    return;
                }
            }
        }
    }
}
