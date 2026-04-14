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
                                "chat_done" | "chat:done" => {
                                    let reply = ev.full_response.unwrap_or_default();
                                    if reply.trim().is_empty() {
                                        tracing::warn!("[channel-inbound] agent returned empty response");
                                        return;
                                    }
                                    tracing::info!(
                                        "[channel-inbound] agent done, replying to channel='{}' len={} streamed_msg_id={:?}",
                                        channel,
                                        reply.len(),
                                        streaming_state.message_id,
                                    );
                                    // If we've been streaming progressive edits, replace
                                    // the outbound message with the final canonical text.
                                    // Otherwise send a fresh message atomically.
                                    finalize_channel_reply(channel, &mut streaming_state, &reply)
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
    /// New content has arrived since the last edit flush.
    dirty: bool,
    /// Consecutive edit failures. Reset to zero on every success.
    edit_failures: u32,
    /// Latched when the backend doesn't support edits for this channel
    /// — we stop trying and rely on the final atomic send.
    edit_disabled: bool,
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
        match client.send_channel_edit(channel, message_id, &jwt, body).await {
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
                        "[channel-inbound][stream] initial draft sent but response lacked id — disabling progressive edits"
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

/// Deliver the final canonical reply. If progressive edits are active,
/// rewrite the streamed message with the final text; otherwise send a
/// fresh atomic message.
async fn finalize_channel_reply(channel: &str, state: &mut StreamingState, final_text: &str) {
    if let Some(ref message_id) = state.message_id {
        if !state.edit_disabled {
            if let Some((client, jwt)) = build_channel_client().await {
                let body = json!({ "text": final_text });
                if let Err(err) = client
                    .send_channel_edit(channel, message_id, &jwt, body)
                    .await
                {
                    tracing::warn!(
                        "[channel-inbound] final edit failed channel='{}' msg_id={} err={} — sending fresh message",
                        channel,
                        message_id,
                        err,
                    );
                    send_channel_reply(channel, final_text).await;
                }
                return;
            }
        }
    }
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
