//! Channel message and thread routes on the authenticated backend client.
//!
//! Ported from the core's `backend/client/channel.rs`; the routes are unchanged.

use anyhow::Result;
use reqwest::Method;
use serde_json::{json, Value};

use openhuman_embed::__host::security::credentials::session_support::BackendCredential;

use super::backend_client::BackendClient;

impl BackendClient {
    /// Sends a message to a communication channel.
    pub async fn send_channel_message(
        &self,
        channel: &str,
        credential: impl Into<BackendCredential>,
        message_body: Value,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded = urlencoding::encode(channel);
        self.authed_json(
            credential,
            Method::POST,
            &format!("channels/{encoded}/messages"),
            Some(message_body),
        )
        .await
    }

    /// Signals "the agent is typing…" on a channel that supports it
    /// (Telegram's `sendChatAction`, Slack's typing event, …). The backend
    /// resolves the target chat from the channel integration metadata and
    /// is responsible for hitting the provider-native API.
    ///
    /// Telegram keeps the typing indicator alive for ~5 seconds per call,
    /// so callers should re-invoke every ~4 s for as long as the turn is
    /// in flight. Returns `Err` if the backend doesn't support typing for
    /// this channel — caller should swallow the error silently.
    pub async fn send_channel_typing(
        &self,
        channel: &str,
        credential: impl Into<BackendCredential>,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded = urlencoding::encode(channel);
        self.authed_json(
            credential,
            Method::POST,
            &format!("channels/{encoded}/typing"),
            Some(json!({})),
        )
        .await
    }

    /// Edits an existing channel message. Used by the progressive-edit
    /// streaming path (Telegram / Slack) to coalesce live deltas into a
    /// single evolving outbound message rather than spamming the chat
    /// with one bubble per token.
    ///
    /// `message_id` is the backend-returned id of the message that was
    /// first sent via [`Self::send_channel_message`]. Returns the
    /// updated message record, or an `Err` if the backend does not
    /// support editing for this channel (caller should fall back to
    /// atomic-final delivery).
    ///
    /// **The deployed backend does not implement this route yet** (#5230): its
    /// channel router has `POST /:channel/messages` and `DELETE
    /// /:channel/messages/:messageId` only, so every call currently returns the
    /// unmatched-route 404, which [`authed_json`](Self::authed_json) surfaces as
    /// [`HostedApiError::ChannelEditUnsupported`](super::HostedApiError::ChannelEditUnsupported). Callers must degrade rather
    /// than treat that as the message having been deleted. Keep this method: it
    /// starts working unchanged the moment the backend adds the route.
    pub async fn send_channel_edit(
        &self,
        channel: &str,
        message_id: &str,
        credential: impl Into<BackendCredential>,
        edit_body: Value,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        anyhow::ensure!(!message_id.is_empty(), "message_id is required");
        let encoded_channel = urlencoding::encode(channel);
        let encoded_id = urlencoding::encode(message_id);
        self.authed_json(
            credential,
            Method::PATCH,
            &format!("channels/{encoded_channel}/messages/{encoded_id}"),
            Some(edit_body),
        )
        .await
    }

    /// Deletes a message from a communication channel. Used to clean up
    /// ephemeral messages (e.g. thinking indicators) after the final
    /// response has been delivered.
    pub async fn send_channel_delete(
        &self,
        channel: &str,
        message_id: &str,
        credential: impl Into<BackendCredential>,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        anyhow::ensure!(!message_id.is_empty(), "message_id is required");
        let encoded_channel = urlencoding::encode(channel);
        let encoded_id = urlencoding::encode(message_id);
        self.authed_json(
            credential,
            Method::DELETE,
            &format!("channels/{encoded_channel}/messages/{encoded_id}"),
            None,
        )
        .await
    }

    /// Sends a reaction (e.g. emoji) to a message in a channel.
    pub async fn send_channel_reaction(
        &self,
        channel: &str,
        credential: impl Into<BackendCredential>,
        reaction_body: Value,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded = urlencoding::encode(channel);
        self.authed_json(
            credential,
            Method::POST,
            &format!("channels/{encoded}/reactions"),
            Some(reaction_body),
        )
        .await
    }

    /// Creates a new thread in a communication channel.
    pub async fn create_channel_thread(
        &self,
        channel: &str,
        credential: impl Into<BackendCredential>,
        title: &str,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        anyhow::ensure!(!title.trim().is_empty(), "title is required");
        let encoded = urlencoding::encode(channel);
        let body = serde_json::json!({ "title": title.trim() });
        self.authed_json(
            credential,
            Method::POST,
            &format!("channels/{encoded}/threads"),
            Some(body),
        )
        .await
    }

    /// Updates an existing thread (e.g., closing or reopening it).
    pub async fn update_channel_thread(
        &self,
        channel: &str,
        credential: impl Into<BackendCredential>,
        thread_id: &str,
        action: &str,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        anyhow::ensure!(!thread_id.trim().is_empty(), "threadId is required");
        anyhow::ensure!(
            action == "close" || action == "reopen",
            "action must be 'close' or 'reopen'"
        );
        let encoded_channel = urlencoding::encode(channel);
        let encoded_thread = urlencoding::encode(thread_id.trim());
        let body = serde_json::json!({ "action": action });
        self.authed_json(
            credential,
            Method::PATCH,
            &format!("channels/{encoded_channel}/threads/{encoded_thread}"),
            Some(body),
        )
        .await
    }

    /// Lists threads in a communication channel, optionally filtering by status.
    pub async fn list_channel_threads(
        &self,
        channel: &str,
        credential: impl Into<BackendCredential>,
        active_filter: Option<bool>,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded = urlencoding::encode(channel);
        let mut path = format!("channels/{encoded}/threads");
        if let Some(active) = active_filter {
            path.push_str(if active {
                "?active=true"
            } else {
                "?active=false"
            });
        }
        self.authed_json(credential, Method::GET, &path, None).await
    }
}
