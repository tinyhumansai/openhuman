use crate::core::events::DomainEvent;
use async_trait::async_trait;
use serde_json::{json, Value};
use tinybus::EventHandler;

/// Subscribes to `ChannelInboundMessage` events and runs the agent loop,
/// sending replies back to the originating channel via the backend REST API.
pub struct ChannelInboundSubscriber;

impl Default for ChannelInboundSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

fn channel_message_body_with_idempotency(channel: &str, body: Value) -> Value {
    let intent = tinychannels::outbound_intent_from_legacy_message(channel, body);
    tinychannels::legacy_message_value_from_outbound_intent(&intent)
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

        let mut event_rx = crate::openhuman::web_chat::subscribe_web_channel_events();

        let request_id = match crate::openhuman::web_chat::start_chat(
            &client_id,
            &thread_id,
            message,
            None,
            None,
            None,
            None,
            None,
            crate::openhuman::web_chat::ChatRequestMetadata {
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
                    // Progressive draft/thinking bubbles require edit+delete
                    // support; skip them on channels that lack it (Discord) so
                    // they don't leave un-cleanable placeholder messages.
                    if channel_supports_progressive_ui(channel) {
                        if streaming_state.thinking_dirty && !streaming_state.thinking_edit_disabled {
                            flush_thinking_message(channel, &mut streaming_state).await;
                        }
                        if streaming_state.dirty && !streaming_state.edit_disabled {
                            flush_streaming_edit(channel, &mut streaming_state).await;
                        }
                    }
                }
                _ = typing_timer.tick() => {
                    if !typing_state.disabled {
                        send_typing_indicator(channel, &mut typing_state).await;
                    }
                }
                _ = filler_timer.tick() => {
                    // Fillers ("💭 Still working on it…") are ephemeral and
                    // deleted on finalize — only post them where cleanup works.
                    if channel_supports_progressive_ui(channel) && !streaming_state.filler_disabled {
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

/// Whether a channel supports the progressive-UI placeholders — the
/// evolving draft bubble, the rotating "💭" fillers, and the ephemeral
/// "thinking" bubble. All three rely on the backend supporting **both**
/// message *edit* and *delete*: edit keeps a single bubble evolving in
/// place, delete removes it once the final reply lands. Telegram supports
/// both. Discord's adapter supports **neither** (edits 404, delete is a
/// hard `Delete not supported` stub), so every placeholder becomes a
/// permanent, un-editable, un-deletable message — the channel fills with
/// "💭 Still working on it…" bubbles.
///
/// This is an **allowlist**, not a denylist: only channels confirmed to
/// support edit+delete opt in. A new/unknown adapter therefore fails *safe*
/// (placeholders suppressed) rather than silently re-introducing the spam bug
/// this gate was added to fix.
fn channel_supports_progressive_ui(channel: &str) -> bool {
    // Inbound channels arrive provider-prefixed from the socket layer
    // (e.g. `discord:<guild>`, `tg:<chat>`), so compare the provider prefix,
    // not the whole id — mirroring `channel_is_telegram`.
    let provider = channel.split(':').next().unwrap_or(channel);
    matches!(provider, "telegram" | "tg")
}

/// Why an edit of an already-posted channel message failed.
///
/// These three causes need three different recoveries, and conflating the
/// first two is what broke the live "💭 Thinking:" bubble (#5230).
#[derive(Debug, PartialEq, Eq)]
enum EditFailure {
    /// The backend serves no message-edit route at all, so the edit could
    /// never have succeeded. The message itself is untouched and we still own
    /// it: keep its id so it can still be deleted (or finally edited once the
    /// backend ships the route) and just stop attempting edits.
    RouteUnsupported,
    /// The message really is gone on the provider side (user deleted it, or
    /// the backend GC'd the relay row). The id is worthless — drop it.
    MessageGone,
    /// Anything else (transient 5xx, transport error, rate limit). Counts
    /// against the per-turn failure budget and may be retried.
    Transient,
}

/// Classify an edit failure by typed error rather than by message text, so the
/// recovery a call site picks cannot drift with `#[error(...)]` wording.
fn classify_edit_failure(err: &anyhow::Error) -> EditFailure {
    match err.downcast_ref::<crate::api::rest::BackendApiError>() {
        Some(crate::api::rest::BackendApiError::ChannelEditUnsupported { .. }) => {
            EditFailure::RouteUnsupported
        }
        Some(crate::api::rest::BackendApiError::MessageNotFound { .. }) => EditFailure::MessageGone,
        _ => EditFailure::Transient,
    }
}

/// Providers whose backend answered "no edit route" at least once this
/// process. Whether the route exists is a property of the deployed backend,
/// not of a turn, so re-probing it on every turn only buys a guaranteed-404
/// round-trip per turn forever. Latching it here keeps that to one attempt per
/// provider per process — and it self-heals on the next core start once the
/// backend ships the route (#5230).
static EDIT_UNSUPPORTED_PROVIDERS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashSet<String>>,
> = std::sync::OnceLock::new();

/// Provider key for the edit-capability latch. Inbound channels arrive
/// provider-prefixed (`telegram:<chat>`), and the route's existence is a
/// per-provider fact, so latch on the prefix — same reasoning as
/// [`channel_supports_progressive_ui`].
fn edit_capability_key(channel: &str) -> String {
    channel.split(':').next().unwrap_or(channel).to_string()
}

/// `true` once this process has learned the backend serves no edit route for
/// `channel`'s provider. Callers should skip the request entirely.
fn channel_edits_unsupported(channel: &str) -> bool {
    EDIT_UNSUPPORTED_PROVIDERS
        .get_or_init(Default::default)
        .lock()
        .map(|set| set.contains(&edit_capability_key(channel)))
        .unwrap_or(false)
}

/// Latch `channel`'s provider as having no message-edit route.
fn mark_channel_edits_unsupported(channel: &str) {
    let key = edit_capability_key(channel);
    if let Ok(mut set) = EDIT_UNSUPPORTED_PROVIDERS
        .get_or_init(Default::default)
        .lock()
    {
        if set.insert(key.clone()) {
            tracing::warn!(
                "[channel-inbound][edit] backend serves no message-edit route for provider='{}' — \
                 progressive edits disabled for this process; placeholders will be posted once and \
                 cleaned up by delete instead (#5230)",
                key,
            );
        }
    }
}

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
    /// The backend has no edit route: stop attempting edits but **keep**
    /// `message_id`. The draft is still on the user's screen and we still own
    /// it, so finalization needs the id to delete it before posting the
    /// canonical reply. Dropping the id here is what orphaned the draft and
    /// left a stale "_working…_" bubble (#5230).
    fn latch_draft_edits_unsupported(&mut self) {
        self.edit_disabled = true;
    }

    /// The draft really is gone provider-side: the id is worthless, so forget
    /// it as well as disabling edits.
    fn forget_draft(&mut self) {
        self.message_id = None;
        self.edit_disabled = true;
    }

    /// Thinking-bubble counterpart of [`Self::latch_draft_edits_unsupported`].
    /// Keeping `thinking_message_id` is what lets finalization delete the
    /// ephemeral "💭 Thinking:" bubble instead of leaving it in the chat (#5230).
    fn latch_thinking_edits_unsupported(&mut self) {
        self.thinking_edit_disabled = true;
    }

    /// Thinking-bubble counterpart of [`Self::forget_draft`].
    fn forget_thinking(&mut self) {
        self.thinking_message_id = None;
        self.thinking_edit_disabled = true;
    }

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
