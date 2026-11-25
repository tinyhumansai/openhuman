//! `channel_relay_inbound` parameters: what a relaying gateway sends, the
//! limits it is held to, and the thread and message it becomes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use tinychannels_bus::ChannelMessage;

/// Longest channel name: `telegram`, `imessage`, `discord`, ...
pub const MAX_CHANNEL_LEN: usize = 32;
/// Longest chat, sender, message or client id.
pub const MAX_ID_LEN: usize = 128;
/// Longest sender display name, in characters.
pub const MAX_SENDER_NAME_CHARS: usize = 128;
/// Largest message text, in bytes.
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
/// Channel names the core gives meaning of its own; a relay may not use them.
pub const RESERVED_CHANNELS: &[&str] = &["web", "cli", "relay"];
/// The `/events` client id outbound replies go to when the caller names none.
pub const DEFAULT_RELAY_CLIENT_ID: &str = "channel-relay";

/// One message a gateway relays from a hosted chat platform.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RelayInboundParams {
    /// The platform: lowercase `[a-z][a-z0-9_-]*` (`telegram`).
    pub channel: String,
    /// The chat the message arrived in; replies go back to it.
    pub chat_id: String,
    /// The platform's id for the sender.
    pub sender_id: String,
    /// The sender's display name, when the platform gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_name: Option<String>,
    /// The platform's id for the message. A message already recorded on its
    /// thread is not run twice, so a gateway may retry safely.
    pub message_id: String,
    /// The message text.
    pub text: String,
    /// The `/events` client id the gateway listens on for this user's
    /// replies. Defaults to [`DEFAULT_RELAY_CLIENT_ID`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Reserved: attachments are not relayed yet, and a non-empty list is
    /// refused rather than silently dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Value>>,
}

impl RelayInboundParams {
    /// Fixture-only identity matching the owner operation's canonical wire form.
    /// Production callers obtain this from `PrepareChannelIdentity`.
    #[cfg(test)]
    pub(crate) fn thread_id(&self) -> String {
        format!(
            "channel:{}/{}/{}",
            self.channel, self.sender_id, self.chat_id
        )
    }

    /// A message with the five required facts and nothing optional: no
    /// sender name, the default client id and no attachments.
    pub fn new(
        channel: impl Into<String>,
        chat_id: impl Into<String>,
        sender_id: impl Into<String>,
        message_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            channel: channel.into(),
            chat_id: chat_id.into(),
            sender_id: sender_id.into(),
            sender_name: None,
            message_id: message_id.into(),
            text: text.into(),
            client_id: None,
            attachments: None,
        }
    }

    /// Check every field against the relay's limits.
    pub fn validate(&self) -> Result<(), String> {
        validate_channel(&self.channel)?;
        validate_id("chat_id", &self.chat_id)?;
        validate_id("sender_id", &self.sender_id)?;
        validate_id("message_id", &self.message_id)?;
        if let Some(client_id) = &self.client_id {
            validate_id("client_id", client_id)?;
        }
        if let Some(name) = &self.sender_name {
            if name.chars().count() > MAX_SENDER_NAME_CHARS {
                return Err(format!(
                    "sender_name must be at most {MAX_SENDER_NAME_CHARS} characters"
                ));
            }
            if name.chars().any(char::is_control) {
                return Err("sender_name may not contain control characters".to_string());
            }
        }
        if self.text.trim().is_empty() {
            return Err("text must not be empty".to_string());
        }
        if self.text.len() > MAX_TEXT_BYTES {
            return Err(format!("text must be at most {MAX_TEXT_BYTES} bytes"));
        }
        if self
            .attachments
            .as_ref()
            .is_some_and(|list| !list.is_empty())
        {
            return Err("attachments are not supported by the relay yet".to_string());
        }
        Ok(())
    }

    /// The `/events` client id replies go to.
    pub fn client_id(&self) -> &str {
        self.client_id.as_deref().unwrap_or(DEFAULT_RELAY_CLIENT_ID)
    }

    /// A display title for a new thread.
    pub fn thread_title(&self) -> String {
        let sender = self
            .sender_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.sender_id);
        format!("{} · {sender} · {}", self.channel, self.chat_id)
    }

    /// The message as the channel dispatch pipeline takes it.
    pub(crate) fn to_channel_message(&self) -> ChannelMessage {
        ChannelMessage {
            reply_to_id: None,
            topic_id: None,
            id: self.message_id.clone(),
            sender: self.sender_id.clone(),
            reply_target: self.chat_id.clone(),
            content: self.text.clone(),
            channel: self.channel.clone(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default(),
            thread_ts: None,
            sender_name: self
                .sender_name
                .clone()
                .filter(|name| !name.trim().is_empty()),
        }
    }
}

/// A channel name is a lowercase token. It never carries the `:`, `/` or `#`
/// that structure a thread id, and never names a core channel.
pub fn validate_channel(channel: &str) -> Result<(), String> {
    let mut bytes = channel.bytes();
    let starts_with_letter = bytes.next().is_some_and(|b| b.is_ascii_lowercase());
    if !starts_with_letter
        || channel.len() > MAX_CHANNEL_LEN
        || !bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        return Err(format!(
            "channel must be 1 to {MAX_CHANNEL_LEN} characters of [a-z0-9_-], starting with a letter"
        ));
    }
    if RESERVED_CHANNELS.contains(&channel) {
        return Err(format!("channel `{channel}` is reserved"));
    }
    Ok(())
}

/// A platform id: printable ASCII without spaces, and without the `/`, `#`
/// and `\` that would let two different chats derive the same thread id.
pub fn validate_id(field: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_ID_LEN {
        return Err(format!("{field} must be 1 to {MAX_ID_LEN} characters"));
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_graphic() && !matches!(b, b'/' | b'#' | b'\\'))
    {
        return Err(format!(
            "{field} may contain only printable ASCII without spaces, '/', '#' or '\\'"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "params_tests.rs"]
mod tests;
