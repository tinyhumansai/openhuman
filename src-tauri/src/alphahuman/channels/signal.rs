use crate::alphahuman::channels::traits::{Channel, ChannelMessage, SendMessage};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

const GROUP_TARGET_PREFIX: &str = "group:";

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecipientTarget {
    Direct(String),
    Group(String),
}

/// Signal channel using signal-cli daemon's native JSON-RPC + SSE API.
///
/// Connects to a running `signal-cli daemon --http <host:port>`.
/// Listens via SSE at `/api/v1/events` and sends via JSON-RPC at
/// `/api/v1/rpc`.
#[derive(Clone)]
pub struct SignalChannel {
    http_url: String,
    account: String,
    group_id: Option<String>,
    allowed_from: Vec<String>,
    ignore_attachments: bool,
    ignore_stories: bool,
}

// ── signal-cli SSE event JSON shapes ────────────────────────────

#[derive(Debug, Deserialize)]
struct SseEnvelope {
    #[serde(default)]
    envelope: Option<Envelope>,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    source: Option<String>,
    #[serde(rename = "sourceNumber", default)]
    source_number: Option<String>,
    #[serde(rename = "dataMessage", default)]
    data_message: Option<DataMessage>,
    #[serde(rename = "storyMessage", default)]
    story_message: Option<serde_json::Value>,
    #[serde(default)]
    timestamp: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DataMessage {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    timestamp: Option<u64>,
    #[serde(rename = "groupInfo", default)]
    group_info: Option<GroupInfo>,
    #[serde(default)]
    attachments: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
struct GroupInfo {
    #[serde(rename = "groupId", default)]
    group_id: Option<String>,
}

impl SignalChannel {
    pub fn new(
        http_url: String,
        account: String,
        group_id: Option<String>,
        allowed_from: Vec<String>,
        ignore_attachments: bool,
        ignore_stories: bool,
    ) -> Self {
        let http_url = http_url.trim_end_matches('/').to_string();
        Self {
            http_url,
            account,
            group_id,
            allowed_from,
            ignore_attachments,
            ignore_stories,
        }
    }

    fn http_client(&self) -> Client {
        let builder = Client::builder().connect_timeout(Duration::from_secs(10));
        let builder = crate::alphahuman::config::apply_runtime_proxy_to_builder(builder, "channel.signal");
        builder.build().expect("Signal HTTP client should build")
    }

    /// Effective sender: prefer `sourceNumber` (E.164), fall back to `source`.
    fn sender(envelope: &Envelope) -> Option<String> {
        envelope
            .source_number
            .as_deref()
            .or(envelope.source.as_deref())
            .map(String::from)
    }

    fn is_sender_allowed(&self, sender: &str) -> bool {
        if self.allowed_from.iter().any(|u| u == "*") {
            return true;
        }
        self.allowed_from.iter().any(|u| u == sender)
    }

    fn is_e164(recipient: &str) -> bool {
        let Some(number) = recipient.strip_prefix('+') else {
            return false;
        };
        (2..=15).contains(&number.len()) && number.chars().all(|c| c.is_ascii_digit())
    }

    /// Check whether a string is a valid UUID (signal-cli uses these for
    /// privacy-enabled users who have opted out of sharing their phone number).
    fn is_uuid(s: &str) -> bool {
        Uuid::parse_str(s).is_ok()
    }

    fn parse_recipient_target(recipient: &str) -> RecipientTarget {
        if let Some(group_id) = recipient.strip_prefix(GROUP_TARGET_PREFIX) {
            return RecipientTarget::Group(group_id.to_string());
        }

        if Self::is_e164(recipient) || Self::is_uuid(recipient) {
            RecipientTarget::Direct(recipient.to_string())
        } else {
            RecipientTarget::Group(recipient.to_string())
        }
    }

    /// Check whether the message targets the configured group.
    /// If no `group_id` is configured (None), all DMs and groups are accepted.
    /// Use "dm" to filter DMs only.
    fn matches_group(&self, data_msg: &DataMessage) -> bool {
        let Some(ref expected) = self.group_id else {
            return true;
        };
        match data_msg
            .group_info
            .as_ref()
            .and_then(|g| g.group_id.as_deref())
        {
            Some(gid) => gid == expected.as_str(),
            None => expected.eq_ignore_ascii_case("dm"),
        }
    }

    /// Determine the send target: group id or the sender's number.
    fn reply_target(&self, data_msg: &DataMessage, sender: &str) -> String {
        if let Some(group_id) = data_msg
            .group_info
            .as_ref()
            .and_then(|g| g.group_id.as_deref())
        {
            format!("{GROUP_TARGET_PREFIX}{group_id}")
        } else {
            sender.to_string()
        }
    }

    /// Send a JSON-RPC request to signal-cli daemon.
    async fn rpc_request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        let url = format!("{}/api/v1/rpc", self.http_url);
        let id = Uuid::new_v4().to_string();

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });

        let resp = self
            .http_client()
            .post(&url)
            .timeout(Duration::from_secs(30))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        // 201 = success with no body (e.g. typing indicators)
        if resp.status().as_u16() == 201 {
            return Ok(None);
        }

        let text = resp.text().await?;
        if text.is_empty() {
            return Ok(None);
        }

        let parsed: serde_json::Value = serde_json::from_str(&text)?;
        if let Some(err) = parsed.get("error") {
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            anyhow::bail!("Signal RPC error {code}: {msg}");
        }

        Ok(parsed.get("result").cloned())
    }

    /// Process a single SSE envelope, returning a ChannelMessage if valid.
    fn process_envelope(&self, envelope: &Envelope) -> Option<ChannelMessage> {
        // Skip story messages when configured
        if self.ignore_stories && envelope.story_message.is_some() {
            return None;
        }

        let data_msg = envelope.data_message.as_ref()?;

        // Skip attachment-only messages when configured
        if self.ignore_attachments {
            let has_attachments = data_msg.attachments.as_ref().is_some_and(|a| !a.is_empty());
            if has_attachments && data_msg.message.is_none() {
                return None;
            }
        }

        let text = data_msg.message.as_deref().filter(|t| !t.is_empty())?;
        let sender = Self::sender(envelope)?;

        if !self.is_sender_allowed(&sender) {
            return None;
        }

        if !self.matches_group(data_msg) {
            return None;
        }

        let target = self.reply_target(data_msg, &sender);

        let timestamp = data_msg
            .timestamp
            .or(envelope.timestamp)
            .unwrap_or_else(|| {
                u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis(),
                )
                .unwrap_or(u64::MAX)
            });

        Some(ChannelMessage {
            id: format!("sig_{timestamp}"),
            sender: sender.clone(),
            reply_target: target,
            content: text.to_string(),
            channel: "signal".to_string(),
            timestamp: timestamp / 1000, // millis → secs
            thread_ts: None,
        })
    }
}

#[async_trait]
impl Channel for SignalChannel {
    fn name(&self) -> &str {
        "signal"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        let params = match Self::parse_recipient_target(&message.recipient) {
            RecipientTarget::Direct(number) => serde_json::json!({
                "recipient": [number],
                "message": &message.content,
                "account": &self.account,
            }),
            RecipientTarget::Group(group_id) => serde_json::json!({
                "groupId": group_id,
                "message": &message.content,
                "account": &self.account,
            }),
        };

        self.rpc_request("send", params).await?;
        Ok(())
    }

    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        let mut url = reqwest::Url::parse(&format!("{}/api/v1/events", self.http_url))?;
        url.query_pairs_mut().append_pair("account", &self.account);

        tracing::info!("Signal channel listening via SSE on {}...", self.http_url);

        let mut retry_delay_secs = 2u64;
        let max_delay_secs = 60u64;

        loop {
            let resp = self
                .http_client()
                .get(url.clone())
                .header("Accept", "text/event-stream")
                .send()
                .await;

            let resp = match resp {
                Ok(r) if r.status().is_success() => r,
                Ok(r) => {
                    let status = r.status();
                    let body = r.text().await.unwrap_or_default();
                    tracing::warn!("Signal SSE returned {status}: {body}");
                    tokio::time::sleep(tokio::time::Duration::from_secs(retry_delay_secs)).await;
                    retry_delay_secs = (retry_delay_secs * 2).min(max_delay_secs);
                    continue;
                }
                Err(e) => {
                    tracing::warn!("Signal SSE connect error: {e}, retrying...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(retry_delay_secs)).await;
                    retry_delay_secs = (retry_delay_secs * 2).min(max_delay_secs);
                    continue;
                }
            };

            retry_delay_secs = 2;

            let mut bytes_stream = resp.bytes_stream();
            let mut buffer = String::new();
            let mut current_data = String::new();

            while let Some(chunk) = bytes_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::debug!("Signal SSE chunk error, reconnecting: {e}");
                        break;
                    }
                };

                let text = match String::from_utf8(chunk.to_vec()) {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::debug!("Signal SSE invalid UTF-8, skipping chunk: {}", e);
                        continue;
                    }
                };

                buffer.push_str(&text);

                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                    buffer = buffer[newline_pos + 1..].to_string();

                    // Skip SSE comments (keepalive)
                    if line.starts_with(':') {
                        continue;
                    }

                    if line.is_empty() {
                        // Empty line = event boundary, dispatch accumulated data
                        if !current_data.is_empty() {
                            match serde_json::from_str::<SseEnvelope>(&current_data) {
                                Ok(sse) => {
                                    if let Some(ref envelope) = sse.envelope {
                                        if let Some(msg) = self.process_envelope(envelope) {
                                            if tx.send(msg).await.is_err() {
                                                return Ok(());
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!("Signal SSE parse skip: {e}");
                                }
                            }
                            current_data.clear();
                        }
                    } else if let Some(data) = line.strip_prefix("data:") {
                        if !current_data.is_empty() {
                            current_data.push('\n');
                        }
                        current_data.push_str(data.trim_start());
                    }
                    // Ignore "event:", "id:", "retry:" lines
                }
            }

            if !current_data.is_empty() {
                match serde_json::from_str::<SseEnvelope>(&current_data) {
                    Ok(sse) => {
                        if let Some(ref envelope) = sse.envelope {
                            if let Some(msg) = self.process_envelope(envelope) {
                                let _ = tx.send(msg).await;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Signal SSE trailing parse skip: {e}");
                    }
                }
            }

            tracing::debug!("Signal SSE stream ended, reconnecting...");
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }
    }

    async fn health_check(&self) -> bool {
        let url = format!("{}/api/v1/check", self.http_url);
        let Ok(resp) = self
            .http_client()
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        else {
            return false;
        };
        resp.status().is_success()
    }

    async fn start_typing(&self, recipient: &str) -> anyhow::Result<()> {
        let params = match Self::parse_recipient_target(recipient) {
            RecipientTarget::Direct(number) => serde_json::json!({
                "recipient": [number],
                "account": &self.account,
            }),
            RecipientTarget::Group(group_id) => serde_json::json!({
                "groupId": group_id,
                "account": &self.account,
            }),
        };
        self.rpc_request("sendTyping", params).await?;
        Ok(())
    }

    async fn stop_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        // signal-cli doesn't have a stop-typing RPC; typing indicators
        // auto-expire after ~15s on the client side.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_channel() -> SignalChannel {
        SignalChannel::new(
            "http://127.0.0.1:8686".to_string(),
            "+1234567890".to_string(),
            None,
            vec!["+1111111111".to_string()],
            false,
            false,
        )
    }

    fn make_channel_with_group(group_id: &str) -> SignalChannel {
        SignalChannel::new(
            "http://127.0.0.1:8686".to_string(),
            "+1234567890".to_string(),
            Some(group_id.to_string()),
            vec!["*".to_string()],
            true,
            true,
        )
    }

    fn make_envelope(source_number: Option<&str>, message: Option<&str>) -> Envelope {
        Envelope {
            source: source_number.map(String::from),
            source_number: source_number.map(String::from),
            data_message: message.map(|m| DataMessage {
                message: Some(m.to_string()),
                timestamp: Some(1_700_000_000_000),
                group_info: None,
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        }
    }

    #[test]
    fn creates_with_correct_fields() {
        let ch = make_channel();
        assert_eq!(ch.http_url, "http://127.0.0.1:8686");
        assert_eq!(ch.account, "+1234567890");
        assert!(ch.group_id.is_none());
        assert_eq!(ch.allowed_from.len(), 1);
        assert!(!ch.ignore_attachments);
        assert!(!ch.ignore_stories);
    }

    #[test]
    fn strips_trailing_slash() {
        let ch = SignalChannel::new(
            "http://127.0.0.1:8686/".to_string(),
            "+1234567890".to_string(),
            None,
            vec![],
            false,
            false,
        );
        assert_eq!(ch.http_url, "http://127.0.0.1:8686");
    }

    #[test]
    fn wildcard_allows_anyone() {
        let ch = make_channel_with_group("dm");
        assert!(ch.is_sender_allowed("+9999999999"));
    }

    #[test]
    fn specific_sender_allowed() {
        let ch = make_channel();
        assert!(ch.is_sender_allowed("+1111111111"));
    }

    #[test]
    fn unknown_sender_denied() {
        let ch = make_channel();
        assert!(!ch.is_sender_allowed("+9999999999"));
    }

    #[test]
    fn empty_allowlist_denies_all() {
        let ch = SignalChannel::new(
            "http://127.0.0.1:8686".to_string(),
            "+1234567890".to_string(),
            None,
            vec![],
            false,
            false,
        );
        assert!(!ch.is_sender_allowed("+1111111111"));
    }

    #[test]
    fn name_returns_signal() {
        let ch = make_channel();
        assert_eq!(ch.name(), "signal");
    }

    #[test]
    fn matches_group_no_group_id_accepts_all() {
        let ch = make_channel();
        let dm = DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(1000),
            group_info: None,
            attachments: None,
        };
        assert!(ch.matches_group(&dm));

        let group = DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(1000),
            group_info: Some(GroupInfo {
                group_id: Some("group123".to_string()),
            }),
            attachments: None,
        };
        assert!(ch.matches_group(&group));
    }

    #[test]
    fn matches_group_filters_group() {
        let ch = make_channel_with_group("group123");
        let matching = DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(1000),
            group_info: Some(GroupInfo {
                group_id: Some("group123".to_string()),
            }),
            attachments: None,
        };
        assert!(ch.matches_group(&matching));

        let non_matching = DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(1000),
            group_info: Some(GroupInfo {
                group_id: Some("other_group".to_string()),
            }),
            attachments: None,
        };
        assert!(!ch.matches_group(&non_matching));
    }

    #[test]
    fn matches_group_dm_keyword() {
        let ch = make_channel_with_group("dm");
        let dm = DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(1000),
            group_info: None,
            attachments: None,
        };
        assert!(ch.matches_group(&dm));

        let group = DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(1000),
            group_info: Some(GroupInfo {
                group_id: Some("group123".to_string()),
            }),
            attachments: None,
        };
        assert!(!ch.matches_group(&group));
    }

    #[test]
    fn reply_target_dm() {
        let ch = make_channel();
        let dm = DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(1000),
            group_info: None,
            attachments: None,
        };
        assert_eq!(ch.reply_target(&dm, "+1111111111"), "+1111111111");
    }

    #[test]
    fn reply_target_group() {
        let ch = make_channel();
        let group = DataMessage {
            message: Some("hi".to_string()),
            timestamp: Some(1000),
            group_info: Some(GroupInfo {
                group_id: Some("group123".to_string()),
            }),
            attachments: None,
        };
        assert_eq!(ch.reply_target(&group, "+1111111111"), "group:group123");
    }

    #[test]
    fn parse_recipient_target_e164_is_direct() {
        assert_eq!(
            SignalChannel::parse_recipient_target("+1234567890"),
            RecipientTarget::Direct("+1234567890".to_string())
        );
    }

    #[test]
    fn parse_recipient_target_prefixed_group_is_group() {
        assert_eq!(
            SignalChannel::parse_recipient_target("group:abc123"),
            RecipientTarget::Group("abc123".to_string())
        );
    }

    #[test]
    fn parse_recipient_target_uuid_is_direct() {
        let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        assert_eq!(
            SignalChannel::parse_recipient_target(uuid),
            RecipientTarget::Direct(uuid.to_string())
        );
    }

    #[test]
    fn parse_recipient_target_non_e164_plus_is_group() {
        assert_eq!(
            SignalChannel::parse_recipient_target("+abc123"),
            RecipientTarget::Group("+abc123".to_string())
        );
    }

    #[test]
    fn is_uuid_valid() {
        assert!(SignalChannel::is_uuid(
            "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
        ));
        assert!(SignalChannel::is_uuid(
            "00000000-0000-0000-0000-000000000000"
        ));
    }

    #[test]
    fn is_uuid_invalid() {
        assert!(!SignalChannel::is_uuid("+1234567890"));
        assert!(!SignalChannel::is_uuid("not-a-uuid"));
        assert!(!SignalChannel::is_uuid("group:abc123"));
        assert!(!SignalChannel::is_uuid(""));
    }

    #[test]
    fn sender_prefers_source_number() {
        let env = Envelope {
            source: Some("uuid-123".to_string()),
            source_number: Some("+1111111111".to_string()),
            data_message: None,
            story_message: None,
            timestamp: Some(1000),
        };
        assert_eq!(SignalChannel::sender(&env), Some("+1111111111".to_string()));
    }

    #[test]
    fn sender_falls_back_to_source() {
        let env = Envelope {
            source: Some("uuid-123".to_string()),
            source_number: None,
            data_message: None,
            story_message: None,
            timestamp: Some(1000),
        };
        assert_eq!(SignalChannel::sender(&env), Some("uuid-123".to_string()));
    }

    #[test]
    fn process_envelope_uuid_sender_dm() {
        let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let ch = SignalChannel::new(
            "http://127.0.0.1:8686".to_string(),
            "+1234567890".to_string(),
            None,
            vec!["*".to_string()],
            false,
            false,
        );
        let env = Envelope {
            source: Some(uuid.to_string()),
            source_number: None,
            data_message: Some(DataMessage {
                message: Some("Hello from privacy user".to_string()),
                timestamp: Some(1_700_000_000_000),
                group_info: None,
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        };
        let msg = ch.process_envelope(&env).unwrap();
        assert_eq!(msg.sender, uuid);
        assert_eq!(msg.reply_target, uuid);
        assert_eq!(msg.content, "Hello from privacy user");

        // Verify reply routing: UUID sender in DM should route as Direct
        let target = SignalChannel::parse_recipient_target(&msg.reply_target);
        assert_eq!(target, RecipientTarget::Direct(uuid.to_string()));
    }

    #[test]
    fn process_envelope_uuid_sender_in_group() {
        let uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let ch = SignalChannel::new(
            "http://127.0.0.1:8686".to_string(),
            "+1234567890".to_string(),
            Some("testgroup".to_string()),
            vec!["*".to_string()],
            false,
            false,
        );
        let env = Envelope {
            source: Some(uuid.to_string()),
            source_number: None,
            data_message: Some(DataMessage {
                message: Some("Group msg from privacy user".to_string()),
                timestamp: Some(1_700_000_000_000),
                group_info: Some(GroupInfo {
                    group_id: Some("testgroup".to_string()),
                }),
                attachments: None,
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        };
        let msg = ch.process_envelope(&env).unwrap();
        assert_eq!(msg.sender, uuid);
        assert_eq!(msg.reply_target, "group:testgroup");

        // Verify reply routing: group message should still route as Group
        let target = SignalChannel::parse_recipient_target(&msg.reply_target);
        assert_eq!(target, RecipientTarget::Group("testgroup".to_string()));
    }

    #[test]
    fn sender_none_when_both_missing() {
        let env = Envelope {
            source: None,
            source_number: None,
            data_message: None,
            story_message: None,
            timestamp: None,
        };
        assert_eq!(SignalChannel::sender(&env), None);
    }

    #[test]
    fn process_envelope_valid_dm() {
        let ch = make_channel();
        let env = make_envelope(Some("+1111111111"), Some("Hello!"));
        let msg = ch.process_envelope(&env).unwrap();
        assert_eq!(msg.content, "Hello!");
        assert_eq!(msg.sender, "+1111111111");
        assert_eq!(msg.channel, "signal");
    }

    #[test]
    fn process_envelope_denied_sender() {
        let ch = make_channel();
        let env = make_envelope(Some("+9999999999"), Some("Hello!"));
        assert!(ch.process_envelope(&env).is_none());
    }

    #[test]
    fn process_envelope_empty_message() {
        let ch = make_channel();
        let env = make_envelope(Some("+1111111111"), Some(""));
        assert!(ch.process_envelope(&env).is_none());
    }

    #[test]
    fn process_envelope_no_data_message() {
        let ch = make_channel();
        let env = make_envelope(Some("+1111111111"), None);
        assert!(ch.process_envelope(&env).is_none());
    }

    #[test]
    fn process_envelope_skips_stories() {
        let ch = make_channel_with_group("dm");
        let mut env = make_envelope(Some("+1111111111"), Some("story text"));
        env.story_message = Some(serde_json::json!({}));
        assert!(ch.process_envelope(&env).is_none());
    }

    #[test]
    fn process_envelope_skips_attachment_only() {
        let ch = make_channel_with_group("dm");
        let env = Envelope {
            source: Some("+1111111111".to_string()),
            source_number: Some("+1111111111".to_string()),
            data_message: Some(DataMessage {
                message: None,
                timestamp: Some(1_700_000_000_000),
                group_info: None,
                attachments: Some(vec![serde_json::json!({"contentType": "image/png"})]),
            }),
            story_message: None,
            timestamp: Some(1_700_000_000_000),
        };
        assert!(ch.process_envelope(&env).is_none());
    }

    #[test]
    fn sse_envelope_deserializes() {
        let json = r#"{
            "envelope": {
                "source": "+1111111111",
                "sourceNumber": "+1111111111",
                "timestamp": 1700000000000,
                "dataMessage": {
                    "message": "Hello Signal!",
                    "timestamp": 1700000000000
                }
            }
        }"#;
        let sse: SseEnvelope = serde_json::from_str(json).unwrap();
        let env = sse.envelope.unwrap();
        assert_eq!(env.source_number.as_deref(), Some("+1111111111"));
        let dm = env.data_message.unwrap();
        assert_eq!(dm.message.as_deref(), Some("Hello Signal!"));
    }

    #[test]
    fn sse_envelope_deserializes_group() {
        let json = r#"{
            "envelope": {
                "sourceNumber": "+2222222222",
                "dataMessage": {
                    "message": "Group msg",
                    "groupInfo": {
                        "groupId": "abc123"
                    }
                }
            }
        }"#;
        let sse: SseEnvelope = serde_json::from_str(json).unwrap();
        let env = sse.envelope.unwrap();
        let dm = env.data_message.unwrap();
        assert_eq!(
            dm.group_info.as_ref().unwrap().group_id.as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn envelope_defaults() {
        let json = r#"{}"#;
        let env: Envelope = serde_json::from_str(json).unwrap();
        assert!(env.source.is_none());
        assert!(env.source_number.is_none());
        assert!(env.data_message.is_none());
        assert!(env.story_message.is_none());
        assert!(env.timestamp.is_none());
    }
}
