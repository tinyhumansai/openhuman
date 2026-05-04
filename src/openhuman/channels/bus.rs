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

        // ── Filler messages ──────────────────────────────────────────
        // Once progressive edits + thinking streams go quiet (backend
        // doesn't support PATCH, reasoning has finished, etc.) the user
        // can wait 30–90 s seeing no fresh activity. Post a short filler
        // every FILLER_INTERVAL so the chat keeps moving. All filler ids
        // are tracked in `StreamingState.filler_message_ids` and deleted
        // in `finalize_channel_reply` once the real response is on screen.
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
                                        streaming_state.thinking_dirty = true;
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
                    if streaming_state.thinking_dirty && !streaming_state.thinking_edit_disabled {
                        flush_thinking_message(channel, &mut streaming_state).await;
                    }
                    if streaming_state.dirty && !streaming_state.edit_disabled {
                        flush_streaming_edit(channel, &mut streaming_state).await;
                    }
                }
                _ = typing_timer.tick() => {
                    if !typing_state.disabled {
                        send_typing_indicator(channel, &mut typing_state).await;
                    }
                }
                _ = filler_timer.tick() => {
                    if !streaming_state.filler_disabled {
                        send_filler_message(channel, &mut streaming_state).await;
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

/// How often to post a filler "still working" message to the channel
/// so the user keeps seeing activity during long agent turns. Deleted
/// on finalization alongside the ephemeral thinking bubble.
const FILLER_INTERVAL: tokio::time::Duration = tokio::time::Duration::from_secs(13);

/// Maximum consecutive filler-send failures before we stop trying.
/// Same rationale as the thinking/typing latches.
const MAX_FILLER_FAILURES: u32 = 2;

/// Maximum number of Unicode scalars to include in a dynamic filler
/// derived from the thinking accumulator. Keeps each bubble compact.
const MAX_FILLER_CHARS: usize = 200;

/// Fallback rotating pool used when the thinking stream has produced
/// nothing new since the previous filler (or nothing at all). Index in
/// `StreamingState.filler_index` advances only when this branch is hit.
const STATIC_FILLERS: &[&str] = &[
    "💭 Still working on it…",
    "💭 Just a moment…",
    "💭 Almost there…",
];

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
    /// Accumulated LLM reasoning from `thinking_delta` events. Shown
    /// to the user as an ephemeral "💭 Thinking…" message that is
    /// **deleted** once the final response is ready (#600).
    thinking_accumulator: String,
    /// Backend-assigned id of the ephemeral thinking message. Used to
    /// delete it at finalization so the user sees only the clean reply.
    thinking_message_id: Option<String>,
    /// `true` once a thinking message has been posted to the channel.
    thinking_sent: bool,
    /// New thinking content has arrived since the last thinking flush.
    thinking_dirty: bool,
    /// Latched when the first thinking POST succeeded with 200 but the
    /// backend didn't return an id we can edit. Without this latch,
    /// every subsequent `thinking_dirty` tick re-enters the "send new
    /// message" branch and the user sees one italic bubble per
    /// accumulated snippet instead of a single evolving one (#600).
    thinking_edit_disabled: bool,
    /// Ids of ephemeral filler messages posted during long turns, in
    /// send order. Deleted in `finalize_channel_reply` after the
    /// canonical response is on screen.
    filler_message_ids: Vec<String>,
    /// Next entry in `STATIC_FILLERS` to send when we fall back to the
    /// rotating pool (no fresh thinking content to surface). Wraps
    /// modulo pool size.
    filler_index: usize,
    /// Consecutive filler-send failures. Reset to zero on success.
    filler_failures: u32,
    /// Latched when the backend rejects filler sends — stops hitting
    /// a broken endpoint every 13 s.
    filler_disabled: bool,
    /// Last dynamic snippet we posted as a filler. Used to skip a
    /// duplicate post when the thinking accumulator hasn't advanced
    /// enough to produce a new tail slice — we fall through to the
    /// static pool instead so the chat still sees movement.
    last_filler_snippet: Option<String>,
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
        let trimmed = self.content.trim_end();
        if trimmed.is_empty() {
            // No visible text yet — show a placeholder. Tool indicators
            // (🔧 …) are intentionally omitted so the draft only ever
            // contains content that is a clean prefix of the final
            // response. If the draft persists after finalization the
            // user sees benign placeholder text instead of stale tool
            // status lines (#600).
            "_working…_".to_string()
        } else {
            trimmed.to_string()
        }
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
        let body = json!({ "text": draft });
        match client.send_channel_message(channel, &jwt, body).await {
            Ok(resp) => {
                // A message was posted to the user — record that fact
                // *before* checking for an id. Even if we can't extract
                // one (and thus can't edit it further), we must never
                // later fall back to sending a second atomic message.
                state.draft_sent = true;
                let id = extract_message_id(&resp);
                if let Some(id) = id {
                    tracing::debug!(
                        "[channel-inbound][stream] initial draft sent channel='{}' msg_id={}",
                        channel,
                        id,
                    );
                    state.message_id = Some(id);
                } else {
                    tracing::warn!(
                        "[channel-inbound][stream] initial draft sent but response lacked id — disabling progressive edits (finalize will skip sending a duplicate) channel='{}' resp={}",
                        channel,
                        resp,
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

/// Extract a message id from a backend `send_channel_message` response.
/// The backend has used at least three shapes: `{"id":"..."}`,
/// `{"data":{"id":"..."}}`, and `{"messageId":1456,"success":true}` —
/// the last one returns the id as a JSON number, not a string, so
/// `as_str()` alone misses it (#600).
fn extract_message_id(resp: &serde_json::Value) -> Option<String> {
    let candidate = resp
        .get("id")
        .or_else(|| resp.get("messageId"))
        .or_else(|| resp.get("data").and_then(|d| d.get("id")))
        .or_else(|| resp.get("data").and_then(|d| d.get("messageId")))?;
    if let Some(s) = candidate.as_str() {
        return Some(s.to_string());
    }
    if let Some(n) = candidate.as_i64() {
        return Some(n.to_string());
    }
    if let Some(n) = candidate.as_u64() {
        return Some(n.to_string());
    }
    None
}

/// Maximum length of the thinking snippet shown in the ephemeral
/// channel message. Longer reasoning is truncated with "…" to avoid
/// overwhelming the chat.
const MAX_THINKING_DISPLAY_CHARS: usize = 500;

/// Send or edit the ephemeral "💭 Thinking…" message on the channel.
/// This message is deleted when the final response is ready.
async fn flush_thinking_message(channel: &str, state: &mut StreamingState) {
    state.thinking_dirty = false;

    if state.thinking_accumulator.trim().is_empty() {
        return;
    }

    let mut snippet = state.thinking_accumulator.trim().to_string();
    if snippet.len() > MAX_THINKING_DISPLAY_CHARS {
        snippet.truncate(MAX_THINKING_DISPLAY_CHARS);
        snippet.push('…');
    }
    let text = format!("💭 Thinking:\n_{snippet}_");

    let Some((client, jwt)) = build_channel_client().await else {
        return;
    };

    if let Some(ref msg_id) = state.thinking_message_id {
        // Edit existing thinking message with updated content.
        let body = json!({ "text": text });
        if let Err(err) = client.send_channel_edit(channel, msg_id, &jwt, body).await {
            tracing::debug!(
                "[channel-inbound][thinking] edit failed channel='{}' msg_id={} err={}",
                channel,
                msg_id,
                err,
            );
        }
    } else {
        // Send initial thinking message.
        let body = json!({ "text": text });
        match client.send_channel_message(channel, &jwt, body).await {
            Ok(resp) => {
                state.thinking_sent = true;
                let id = extract_message_id(&resp);
                if let Some(id) = id {
                    tracing::debug!(
                        "[channel-inbound][thinking] thinking msg sent channel='{}' msg_id={}",
                        channel,
                        id,
                    );
                    state.thinking_message_id = Some(id);
                } else {
                    tracing::warn!(
                        "[channel-inbound][thinking] thinking msg sent but response lacked id — disabling further thinking flushes (message won't be deletable) channel='{}' resp={}",
                        channel,
                        resp,
                    );
                    state.thinking_edit_disabled = true;
                }
            }
            Err(err) => {
                tracing::warn!(
                    "[channel-inbound][thinking] failed to send thinking msg channel='{}' err={} — disabling further thinking flushes",
                    channel,
                    err,
                );
                state.thinking_edit_disabled = true;
            }
        }
    }
}

/// Pull the most recent `MAX_FILLER_CHARS` Unicode scalars out of the
/// thinking accumulator so we can surface a live snapshot of the agent's
/// reasoning as a filler. Returns `None` when there's nothing to show
/// yet. Trims any partial leading word so the snippet reads cleanly.
fn latest_thinking_snippet(state: &StreamingState) -> Option<String> {
    let acc = state.thinking_accumulator.trim();
    if acc.is_empty() {
        return None;
    }
    let total = acc.chars().count();
    let snippet: String = if total <= MAX_FILLER_CHARS {
        acc.to_string()
    } else {
        acc.chars().skip(total - MAX_FILLER_CHARS).collect()
    };
    let trimmed = snippet
        .trim_start_matches(|c: char| !c.is_whitespace())
        .trim_start()
        .to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Post a fresh filler message to the channel and record its id so
/// `finalize_channel_reply` can delete it once the real response is on
/// screen. Prefers a live snippet of the agent's latest reasoning
/// (`thinking_accumulator`); falls back to the rotating `STATIC_FILLERS`
/// pool when there's no new thinking to show.
async fn send_filler_message(channel: &str, state: &mut StreamingState) {
    let text = match latest_thinking_snippet(state) {
        Some(snippet) if state.last_filler_snippet.as_deref() != Some(snippet.as_str()) => {
            state.last_filler_snippet = Some(snippet.clone());
            format!("💭 _{snippet}…_")
        }
        _ => {
            let pool = STATIC_FILLERS;
            let idx = state.filler_index % pool.len();
            state.filler_index = state.filler_index.wrapping_add(1);
            pool[idx].to_string()
        }
    };

    let Some((client, jwt)) = build_channel_client().await else {
        return;
    };
    let body = json!({ "text": text });
    match client.send_channel_message(channel, &jwt, body).await {
        Ok(resp) => {
            state.filler_failures = 0;
            if let Some(id) = extract_message_id(&resp) {
                tracing::debug!(
                    "[channel-inbound][filler] sent channel='{}' len={} msg_id={}",
                    channel,
                    text.len(),
                    id,
                );
                state.filler_message_ids.push(id);
            } else {
                tracing::warn!(
                    "[channel-inbound][filler] sent but response lacked id — cannot clean up on finalize channel='{}' resp={}",
                    channel,
                    resp,
                );
            }
        }
        Err(err) => {
            state.filler_failures = state.filler_failures.saturating_add(1);
            tracing::warn!(
                "[channel-inbound][filler] send failed channel='{}' err={} (failures={}/{})",
                channel,
                err,
                state.filler_failures,
                MAX_FILLER_FAILURES,
            );
            if state.filler_failures >= MAX_FILLER_FAILURES {
                tracing::info!(
                    "[channel-inbound][filler] disabling filler messages for channel='{}' — backend unsupported",
                    channel,
                );
                state.filler_disabled = true;
            }
        }
    }
}

/// Delete a previously sent message from the channel. Used to clean
/// up ephemeral thinking messages once the final response is ready.
async fn delete_channel_message(channel: &str, message_id: &str) {
    let Some((client, jwt)) = build_channel_client().await else {
        return;
    };
    match client.send_channel_delete(channel, message_id, &jwt).await {
        Ok(_) => {
            tracing::info!(
                "[channel-inbound] deleted ephemeral msg channel='{}' msg_id={}",
                channel,
                message_id,
            );
        }
        Err(err) => {
            tracing::warn!(
                "[channel-inbound] failed to delete ephemeral msg channel='{}' msg_id={} err={}",
                channel,
                message_id,
                err,
            );
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
    // Deliver the canonical reply FIRST, then clean up the ephemeral
    // "💭 Thinking:" bubble. Deleting before the reply would leave the
    // chat empty for a beat; this order keeps something visible at all
    // times (#600).
    'send: {
        if let Some(ref message_id) = state.message_id {
            // We committed to a draft earlier in the turn. Always attempt
            // to edit it with the canonical reply, even when we'd
            // previously latched `edit_disabled` during the streaming
            // phase — the user is already looking at that message, so a
            // late edit attempt is still the right call. If the edit
            // fails, delete the orphan draft and send the final reply
            // as a fresh atomic message so the user always sees it.
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
                            "[channel-inbound] final edit failed channel='{}' msg_id={} err={} — deleting orphan draft and sending fresh atomic reply so user still sees the canonical response",
                            channel,
                            message_id,
                            err,
                        );
                        let orphan = message_id.clone();
                        delete_channel_message(channel, &orphan).await;
                        send_channel_reply(channel, final_text).await;
                    }
                }
            } else {
                tracing::warn!(
                    "[channel-inbound] cannot finalize channel='{}' msg_id={} — backend client unavailable, draft left in place",
                    channel,
                    message_id,
                );
            }
            break 'send;
        }
        if state.draft_sent {
            // A draft was posted but the backend didn't return an id, so
            // we have nothing to edit. Since the draft only contains a
            // clean text prefix (or "_working…_" placeholder), sending the
            // final response as a second bubble is acceptable — leaving
            // the user without the canonical reply is worse (#600).
            tracing::warn!(
                "[channel-inbound] sending fresh reply on channel='{}' — id-less draft exists but user needs the final response",
                channel,
            );
            send_channel_reply(channel, final_text).await;
            break 'send;
        }
        // No draft exists — this is the first (and only) message for the
        // turn. Safe to send atomically.
        send_channel_reply(channel, final_text).await;
    }

    // ── Clean up ephemeral filler + thinking messages ───────────
    // Delete after the canonical reply is already on screen so the
    // chat is never momentarily empty between the two operations.
    // Fillers first (more of them, oldest-first), then the thinking
    // bubble — purely cosmetic ordering.
    let fillers = std::mem::take(&mut state.filler_message_ids);
    for id in fillers {
        delete_channel_message(channel, &id).await;
    }
    if let Some(thinking_id) = state.thinking_message_id.take() {
        delete_channel_message(channel, &thinking_id).await;
    }
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

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
