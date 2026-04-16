//! Event bus handlers for the channels domain.
//!
//! The [`ChannelInboundSubscriber`] handles inbound channel messages published
//! by the socket transport layer. It runs the agent inference loop via the web
//! channel provider and sends the reply back through the REST API.

use crate::core::event_bus::{DomainEvent, EventHandler};
use async_trait::async_trait;
use serde_json::json;

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
impl EventHandler for ChannelInboundSubscriber {
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
            raw_data: _,
        } = event
        else {
            return;
        };

        tracing::info!(
            "[channel-inbound] received message from channel='{}' len={}",
            channel,
            message.len()
        );

        let thread_id = format!("channel:{}", channel);
        let client_id = "inbound".to_string();

        let mut event_rx =
            crate::openhuman::channels::providers::web::subscribe_web_channel_events();

        let request_id = match crate::openhuman::channels::providers::web::start_chat(
            &client_id, &thread_id, message, None, None,
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
                send_channel_reply(
                    channel,
                    &format!("Sorry, I couldn't process your message: {err}"),
                )
                .await;
                return;
            }
        };

        let timeout = tokio::time::Duration::from_secs(180);
        let deadline = tokio::time::Instant::now() + timeout;

        // ── Progressive-edit streaming state ──────────────────────────
        // We buffer text/tool deltas and flush them as edits on a
        // timer. If the first edit fails (e.g. the backend doesn't
        // implement the PATCH endpoint for this channel) we latch into
        // `edit_disabled` and fall back to atomic-final delivery.
        let mut streaming_state = StreamingState::default();
        let mut edit_timer = tokio::time::interval(EDIT_FLUSH_INTERVAL);
        edit_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Don't fire immediately; wait for the first tick.
        edit_timer.tick().await;

        // ── Typing indicator state ────────────────────────────────────
        // Telegram's `sendChatAction` keeps the "typing…" UI alive for
        // ~5s, so we re-send every 4s while the turn is in flight. The
        // first call fires immediately; on repeated failures we latch
        // `typing_disabled` to stop hitting a backend that doesn't
        // support it.
        let mut typing_state = TypingState::default();
        let mut typing_timer = tokio::time::interval(TYPING_REFRESH_INTERVAL);
        typing_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Fire immediately on first tick so the indicator shows up as
        // soon as the inbound message is received.
        send_typing_indicator(channel, &mut typing_state).await;
        typing_timer.tick().await; // consume the immediate tick

        loop {
            tokio::select! {
                event = event_rx.recv() => {
                    match event {
                        Ok(ev) if ev.request_id == request_id => {
                            match ev.event.as_str() {
                                "text_delta" => {
                                    if let Some(delta) = ev.delta.as_ref() {
                                        streaming_state.content.push_str(delta);
                                        streaming_state.dirty = true;
                                    }
                                }
                                "tool_call" => {
                                    if let Some(ref name) = ev.tool_name {
                                        streaming_state.last_tool = Some(format!("🔧 {name}…"));
                                        streaming_state.dirty = true;
                                    }
                                }
                                "tool_result" => {
                                    if let Some(ref name) = ev.tool_name {
                                        let ok = ev.success.unwrap_or(true);
                                        streaming_state.last_tool = Some(if ok {
                                            format!("🔧 {name} ✓")
                                        } else {
                                            format!("🔧 {name} ✗")
                                        });
                                        streaming_state.dirty = true;
                                    }
                                }
                                "thinking_delta" => {
                                    if let Some(delta) = ev.delta.as_ref() {
                                        streaming_state.thinking_accumulator.push_str(delta);
                                    }
                                }
                                "chat_done" | "chat:done" => {
                                    let reply = ev.full_response.unwrap_or_default();
                                    // Even when the agent produced no visible
                                    // text, we must close out any draft we
                                    // already posted — otherwise the user is
                                    // left staring at a stale "_working…_"
                                    // message indefinitely.
                                    let reply_text = if reply.trim().is_empty() {
                                        tracing::warn!(
                                            "[channel-inbound] agent returned empty response — finalizing draft with fallback",
                                        );
                                        "(No response from agent.)"
                                    } else {
                                        reply.as_str()
                                    };
                                    tracing::info!(
                                        "[channel-inbound] agent done, replying to channel='{}' len={} streamed_msg_id={:?}",
                                        channel,
                                        reply_text.len(),
                                        streaming_state.message_id,
                                    );
                                    // Send the model's thinking summary as a separate
                                    // message so the user can see the reasoning process.
                                    if !streaming_state.thinking_accumulator.is_empty() {
                                        let summary = format_thinking_summary(
                                            &streaming_state.thinking_accumulator,
                                        );
                                        tracing::debug!(
                                            "[channel-inbound] sending thinking summary to channel='{}' raw_chars={} summary_chars={}",
                                            channel,
                                            streaming_state.thinking_accumulator.len(),
                                            summary.len(),
                                        );
                                        send_channel_reply(channel, &summary).await;
                                    }
                                    // If we've been streaming progressive edits, replace
                                    // the outbound message with the final canonical text.
                                    // Otherwise send a fresh message atomically.
                                    finalize_channel_reply(
                                        channel,
                                        &mut streaming_state,
                                        reply_text,
                                    )
                                    .await;
                                    return;
                                }
                                "chat_error" | "chat:error" => {
                                    let err_msg = ev.message.unwrap_or_else(|| "unknown error".to_string());
                                    tracing::error!("[channel-inbound] agent error: {}", err_msg);
                                    let reply = format!("Sorry, I encountered an error: {err_msg}");
                                    finalize_channel_reply(channel, &mut streaming_state, &reply)
                                        .await;
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
                _ = edit_timer.tick() => {
                    if streaming_state.dirty && !streaming_state.edit_disabled {
                        flush_streaming_edit(channel, &mut streaming_state).await;
                    }
                }
                _ = typing_timer.tick() => {
                    if !typing_state.disabled {
                        send_typing_indicator(channel, &mut typing_state).await;
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    tracing::error!("[channel-inbound] agent timed out after {}s", timeout.as_secs());
                    let reply = "Sorry, the request timed out.";
                    finalize_channel_reply(channel, &mut streaming_state, reply).await;
                    return;
                }
            }
        }
    }
}

/// Minimum interval between progressive edits of the outbound channel
/// message. Tuned to stay comfortably below Telegram's ~1 edit/sec cap
/// per chat. Slack has a similar soft limit.
const EDIT_FLUSH_INTERVAL: tokio::time::Duration = tokio::time::Duration::from_millis(1000);

/// Maximum consecutive edit failures tolerated before giving up on
/// progressive streaming and falling back to atomic-final delivery.
const MAX_EDIT_FAILURES: u32 = 2;

/// How often to re-send the "typing…" indicator while a turn is in
/// flight. Telegram's `sendChatAction` keeps the UI alive for about
/// 5 seconds per call, so we refresh every 4 s to ensure continuity.
const TYPING_REFRESH_INTERVAL: tokio::time::Duration = tokio::time::Duration::from_secs(4);

/// Maximum consecutive typing-indicator failures before we stop
/// trying. One failure is usually "endpoint doesn't exist"; two is
/// enough to conclude the backend doesn't support it on this channel.
const MAX_TYPING_FAILURES: u32 = 2;

/// Per-turn progressive-edit buffer. `dirty=true` means there's new
/// content to flush; `edit_disabled=true` means the backend doesn't
/// support editing for this channel and we should finalize atomically.
#[derive(Default)]
struct StreamingState {
    /// Accumulated visible assistant text from `text_delta` events.
    content: String,
    /// Most recent tool status line (prepended to the message body).
    last_tool: Option<String>,
    /// Backend-assigned message id returned from the initial
    /// `send_channel_message`; subsequent edits target this id.
    message_id: Option<String>,
    /// `true` once a draft message has been posted to the channel,
    /// even when the backend response didn't include an id to target
    /// for future edits. Decouples "a draft exists" from "we can edit
    /// it" so `finalize_channel_reply` won't post a duplicate bubble
    /// when the id was lost.
    draft_sent: bool,
    /// New content has arrived since the last edit flush.
    dirty: bool,
    /// Consecutive edit failures. Reset to zero on every success.
    edit_failures: u32,
    /// Latched when the backend doesn't support edits for this channel
    /// — we stop trying and rely on the final atomic send.
    edit_disabled: bool,
    /// Accumulated model thinking/reasoning text from `thinking_delta` events.
    /// Sent as a separate message before the main response at turn completion.
    thinking_accumulator: String,
}

/// Typing-indicator bookkeeping. One per in-flight turn. Latches
/// `disabled` after repeated failures so channels without typing
/// support stop getting hit every 4 seconds.
#[derive(Default)]
struct TypingState {
    failures: u32,
    disabled: bool,
}

/// Fire a single "typing…" indicator at the channel. Silently
/// latches `disabled` on repeated failure so callers can keep calling
/// this from a timer without accumulating warnings.
async fn send_typing_indicator(channel: &str, state: &mut TypingState) {
    if state.disabled {
        return;
    }
    let Some((client, jwt)) = build_channel_client().await else {
        return;
    };
    match client.send_channel_typing(channel, &jwt).await {
        Ok(_) => {
            if state.failures > 0 {
                tracing::debug!(
                    "[channel-inbound][typing] recovered channel='{}' after {} failure(s)",
                    channel,
                    state.failures,
                );
            }
            state.failures = 0;
        }
        Err(err) => {
            state.failures += 1;
            tracing::debug!(
                "[channel-inbound][typing] indicator failed channel='{}' err={} (failures={}/{})",
                channel,
                err,
                state.failures,
                MAX_TYPING_FAILURES,
            );
            if state.failures >= MAX_TYPING_FAILURES {
                tracing::info!(
                    "[channel-inbound][typing] disabling typing indicator for channel='{}' — backend unsupported",
                    channel,
                );
                state.disabled = true;
            }
        }
    }
}

impl StreamingState {
    fn compose_draft(&self) -> String {
        let mut out = String::new();
        if let Some(ref tool) = self.last_tool {
            out.push_str(tool);
            out.push('\n');
        }
        out.push_str(self.content.trim_end());
        if self.content.is_empty() && self.last_tool.is_none() {
            out.push_str("_working…_");
        }
        out
    }
}

/// Post or edit a draft message carrying the latest buffered text +
/// tool status. On the first call, sends a new message and records its
/// id; on subsequent calls, edits the existing message.
async fn flush_streaming_edit(channel: &str, state: &mut StreamingState) {
    let draft = state.compose_draft();
    if draft.is_empty() {
        return;
    }
    state.dirty = false;

    let Some((client, jwt)) = build_channel_client().await else {
        return;
    };

    if let Some(ref message_id) = state.message_id {
        let body = json!({ "text": draft });
        match client
            .send_channel_edit(channel, message_id, &jwt, body)
            .await
        {
            Ok(_) => {
                tracing::debug!(
                    "[channel-inbound][stream] edit ok channel='{}' msg_id={} chars={}",
                    channel,
                    message_id,
                    draft.len(),
                );
                state.edit_failures = 0;
            }
            Err(err) => {
                state.edit_failures += 1;
                tracing::warn!(
                    "[channel-inbound][stream] edit failed channel='{}' msg_id={} err={} (failures={}/{})",
                    channel,
                    message_id,
                    err,
                    state.edit_failures,
                    MAX_EDIT_FAILURES,
                );
                if state.edit_failures >= MAX_EDIT_FAILURES {
                    tracing::info!(
                        "[channel-inbound][stream] giving up on progressive edits for channel='{}', falling back to atomic delivery",
                        channel,
                    );
                    state.edit_disabled = true;
                }
            }
        }
    } else {
        // Before posting the first visible draft, deliver any accumulated
        // thinking so it appears above the response bubble in the channel.
        if !state.thinking_accumulator.is_empty() {
            let summary = format_thinking_summary(&state.thinking_accumulator);
            tracing::debug!(
                "[channel-inbound][stream] sending thinking summary before first draft channel='{}' raw_chars={} summary_chars={}",
                channel,
                state.thinking_accumulator.len(),
                summary.len(),
            );
            send_channel_reply(channel, &summary).await;
            // Clear so the chat_done handler doesn't send it a second time.
            state.thinking_accumulator.clear();
        }
        let body = json!({ "text": draft });
        match client.send_channel_message(channel, &jwt, body).await {
            Ok(resp) => {
                // A message was posted to the user — record that fact
                // *before* checking for an id. Even if we can't extract
                // one (and thus can't edit it further), we must never
                // later fall back to sending a second atomic message.
                state.draft_sent = true;
                let id = resp
                    .get("id")
                    .or_else(|| resp.get("data").and_then(|d| d.get("id")))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if let Some(id) = id {
                    tracing::debug!(
                        "[channel-inbound][stream] initial draft sent channel='{}' msg_id={}",
                        channel,
                        id,
                    );
                    state.message_id = Some(id);
                } else {
                    tracing::warn!(
                        "[channel-inbound][stream] initial draft sent but response lacked id — disabling progressive edits (finalize will skip sending a duplicate)"
                    );
                    state.edit_disabled = true;
                }
            }
            Err(err) => {
                state.edit_failures += 1;
                tracing::warn!(
                    "[channel-inbound][stream] initial send failed channel='{}' err={} (failures={})",
                    channel,
                    err,
                    state.edit_failures,
                );
                if state.edit_failures >= MAX_EDIT_FAILURES {
                    state.edit_disabled = true;
                }
            }
        }
    }
}

/// Deliver the final canonical reply.
///
/// **Invariant**: if a draft message has already been posted to the
/// channel (`state.draft_sent == true`), we MUST NOT post a second
/// message — that would duplicate the visible bubble on the user's
/// side. When we have an id we attempt one last edit; when the id was
/// lost we leave the draft in place silently. The only path that
/// creates a fresh outbound message is when no draft has been posted
/// at all.
async fn finalize_channel_reply(channel: &str, state: &mut StreamingState, final_text: &str) {
    if let Some(ref message_id) = state.message_id {
        // We committed to a draft earlier in the turn. Always attempt
        // to edit it with the canonical reply, even when we'd
        // previously latched `edit_disabled` during the streaming
        // phase — the user is already looking at that message, so a
        // late edit attempt is still the right call. If the edit
        // fails, leave the draft in place rather than spamming a
        // duplicate bubble.
        if let Some((client, jwt)) = build_channel_client().await {
            let body = json!({ "text": final_text });
            match client
                .send_channel_edit(channel, message_id, &jwt, body)
                .await
            {
                Ok(_) => {
                    tracing::info!(
                        "[channel-inbound] final edit ok channel='{}' msg_id={} chars={}",
                        channel,
                        message_id,
                        final_text.len(),
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        "[channel-inbound] final edit failed channel='{}' msg_id={} err={} — draft left in place (no duplicate message sent)",
                        channel,
                        message_id,
                        err,
                    );
                }
            }
        } else {
            tracing::warn!(
                "[channel-inbound] cannot finalize channel='{}' msg_id={} — backend client unavailable, draft left in place",
                channel,
                message_id,
            );
        }
        return;
    }
    if state.draft_sent {
        // A draft was posted but the backend didn't return an id, so
        // we have nothing to edit. Posting a fresh message here would
        // give the user two bubbles — skip silently.
        tracing::warn!(
            "[channel-inbound] skipping fresh send on channel='{}' — an id-less draft was already posted earlier this turn (duplicate prevented)",
            channel,
        );
        return;
    }
    // No draft exists — this is the first (and only) message for the
    // turn. Safe to send atomically.
    send_channel_reply(channel, final_text).await;
}

/// Construct the REST client + session JWT shared by every outbound
/// channel call on this turn. Returns `None` and logs if either is
/// unavailable so the caller can bail quietly.
async fn build_channel_client() -> Option<(crate::api::rest::BackendOAuthClient, String)> {
    let config = match crate::openhuman::config::rpc::load_config_with_timeout().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[channel-inbound] failed to load config: {}", e);
            return None;
        }
    };
    let api_url = crate::api::config::effective_api_url(&config.api_url);
    let jwt = match crate::api::jwt::get_session_token(&config) {
        Ok(Some(t)) => t,
        Ok(None) => {
            tracing::error!("[channel-inbound] no session JWT — cannot send");
            return None;
        }
        Err(e) => {
            tracing::error!("[channel-inbound] failed to get session token: {}", e);
            return None;
        }
    };
    match crate::api::rest::BackendOAuthClient::new(&api_url) {
        Ok(c) => Some((c, jwt)),
        Err(e) => {
            tracing::error!("[channel-inbound] failed to create API client: {}", e);
            None
        }
    }
}

/// Send a text reply back to a channel via the backend REST API.
async fn send_channel_reply(channel: &str, text: &str) {
    let config = match crate::openhuman::config::rpc::load_config_with_timeout().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[channel-inbound] failed to load config: {}", e);
            return;
        }
    };

    let api_url = crate::api::config::effective_api_url(&config.api_url);
    let jwt = match crate::api::jwt::get_session_token(&config) {
        Ok(Some(t)) => t,
        Ok(None) => {
            tracing::error!("[channel-inbound] no session JWT — cannot reply");
            return;
        }
        Err(e) => {
            tracing::error!("[channel-inbound] failed to get session token: {}", e);
            return;
        }
    };

    let client = match crate::api::rest::BackendOAuthClient::new(&api_url) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[channel-inbound] failed to create API client: {}", e);
            return;
        }
    };

    let body = json!({ "text": text });
    match client.send_channel_message(channel, &jwt, body).await {
        Ok(resp) => {
            tracing::info!(
                "[channel-inbound] reply sent to channel='{}' response={:?}",
                channel,
                resp
            );
        }
        Err(e) => {
            tracing::error!(
                "[channel-inbound] failed to send reply to channel='{}': {}",
                channel,
                e
            );
        }
    }
}

/// Maximum characters of raw thinking content included in the summary sent
/// to the channel. Telegram's hard message limit is 4 096 chars; keeping the
/// body well below that leaves room for the header and the trailing ellipsis.
const MAX_THINKING_CHARS: usize = 1500;

/// Format accumulated thinking content into a readable message for delivery
/// to the channel. Truncates at the last word boundary when the raw content
/// exceeds [`MAX_THINKING_CHARS`], appending `…` to signal the cut.
fn format_thinking_summary(thinking: &str) -> String {
    let trimmed = thinking.trim();
    let body = if trimmed.len() > MAX_THINKING_CHARS {
        // Find a safe char boundary at or before MAX_THINKING_CHARS so we never
        // slice in the middle of a multi-byte UTF-8 sequence.
        let safe_end = (0..=MAX_THINKING_CHARS)
            .rev()
            .find(|&i| trimmed.is_char_boundary(i))
            .unwrap_or(0);
        let slice = &trimmed[..safe_end];
        // Back off further to the last whitespace to avoid cutting mid-word.
        let boundary = slice.rfind(|c: char| c.is_whitespace()).unwrap_or(safe_end);
        format!("{}…", &slice[..boundary])
    } else {
        trimmed.to_string()
    };
    format!("💭 Thinking:\n\n{}", body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event_bus::DomainEvent;

    #[test]
    fn subscriber_metadata_is_stable() {
        let subscriber = ChannelInboundSubscriber::new();
        assert_eq!(subscriber.name(), "channel::inbound_handler");
        assert_eq!(subscriber.domains(), Some(&["channel"][..]));
    }

    #[tokio::test]
    async fn unrelated_events_are_ignored() {
        ChannelInboundSubscriber::default()
            .handle(&DomainEvent::SystemStartup {
                component: "test".into(),
            })
            .await;
    }
}
