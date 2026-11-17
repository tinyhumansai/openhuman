//! `RelayChannel`: the reply sink of a relayed turn.
//!
//! The core never talks to the platform here. Each reply the dispatch
//! pipeline sends becomes a [`CHANNEL_OUTBOUND_EVENT`] on the web-channel
//! event bus, which reaches the caller's own `/events` stream; the gateway
//! that relayed the message delivers it to the platform.

use std::sync::Mutex;

use serde_json::json;

use crate::web_chat::{publish_web_channel_event, publish_web_channel_event_in, WebChannelEvent};
use tinychannels_bus::SendMessage;

/// The event name a relayed reply is published under.
pub const CHANNEL_OUTBOUND_EVENT: &str = "channel_outbound";

/// The reply sink for one relayed message.
pub struct RelayChannel {
    /// The platform (`telegram`): the name the pipeline looks the sink up by.
    channel: String,
    /// The `/events` client id the gateway listens on.
    client_id: String,
    /// The conversation's thread.
    thread_id: String,
    /// Correlates the replies with the `channel_relay_inbound` call.
    request_id: String,
    /// The platform message the replies answer.
    reply_to: String,
    /// Every text sent, in order; persisted as the turn's reply.
    sent: Mutex<Vec<String>>,
}

impl RelayChannel {
    pub fn new(
        channel: impl Into<String>,
        client_id: impl Into<String>,
        thread_id: impl Into<String>,
        request_id: impl Into<String>,
        reply_to: impl Into<String>,
    ) -> Self {
        Self {
            channel: channel.into(),
            client_id: client_id.into(),
            thread_id: thread_id.into(),
            request_id: request_id.into(),
            reply_to: reply_to.into(),
            sent: Mutex::new(Vec::new()),
        }
    }

    /// The texts sent so far, oldest first.
    pub fn sent(&self) -> Vec<String> {
        self.sent.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn name(&self) -> &str {
        &self.channel
    }

    pub(crate) async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        tracing::debug!(
            channel = %self.channel,
            thread_id = %self.thread_id,
            request_id = %self.request_id,
            chars = message.content.chars().count(),
            "[channels::relay] publishing channel_outbound"
        );
        self.sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(message.content.clone());
        publish_web_channel_event(self.outbound_event(message));
        Ok(())
    }

    /// The event that carries `message` to the gateway. The routing stamps
    /// (`agent`, `profile`) are left for `publish_web_channel_event`, which
    /// takes them from the publishing context's tenant, so `/events` hands the
    /// reply only to the caller's own stream.
    pub fn outbound_event(&self, message: &SendMessage) -> WebChannelEvent {
        WebChannelEvent::ChannelOutbound {
            context: crate::web_chat::event_context(
                self.client_id.clone(),
                self.thread_id.clone(),
                self.request_id.clone(),
            ),
            full_response: Some(message.content.clone()),
            structured: Some(json!({
                "kind": CHANNEL_OUTBOUND_EVENT,
                "channel": self.channel,
                "chat_id": message.recipient,
                "thread_ts": message.thread_ts,
                "reply_to_message_id": self.reply_to,
                "idempotency_key": message.idempotency_key,
            })),
        }
    }

    /// Publish lossless typed output to the gateway that owns provider delivery.
    /// Host event acceptance has no provider receipt; receipt IDs arrive
    /// separately from the gateway and are never fabricated.
    pub(crate) fn publish_typed_reply(
        &self,
        ctx: &crate::core::runtime::DispatchCtx,
        channel: &tinychannels_bus::ChannelRef,
        conversation: &tinychannels_bus::ConversationRef,
        turn_id: &str,
        sequence: u64,
        segment_index: u32,
        idempotency_key: &str,
        parts: &[tinychannels_bus::OutboundContentPart],
        text: &str,
    ) {
        self.sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(text.to_owned());
        publish_web_channel_event_in(
            ctx,
            WebChannelEvent::ChannelOutbound {
                context: crate::web_chat::event_context(
                    self.client_id.clone(),
                    self.thread_id.clone(),
                    self.request_id.clone(),
                ),
                full_response: Some(text.to_owned()),
                structured: Some(json!({
                    "kind": CHANNEL_OUTBOUND_EVENT,
                    "channel": self.channel,
                    "chat_id": conversation.id,
                    "thread_ts": conversation.thread_id,
                    "reply_to_message_id": self.reply_to,
                    "turn_id": turn_id,
                    "sequence": sequence,
                    "segment_index": segment_index,
                    "idempotency_key": idempotency_key,
                    "channel_ref": channel,
                    "conversation_ref": conversation,
                    "parts": parts,
                })),
            },
        );
    }
}

#[cfg(test)]
#[path = "channel_tests.rs"]
mod tests;
