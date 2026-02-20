use super::traits::{Channel, ChannelMessage, SendMessage};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const DINGTALK_BOT_CALLBACK_TOPIC: &str = "/v1.0/im/bot/messages/get";

/// DingTalk channel — connects via Stream Mode WebSocket for real-time messages.
/// Replies are sent through per-message session webhook URLs.
pub struct DingTalkChannel {
    client_id: String,
    client_secret: String,
    allowed_users: Vec<String>,
    /// Per-chat session webhooks for sending replies (chatID -> webhook URL).
    /// DingTalk provides a unique webhook URL with each incoming message.
    session_webhooks: Arc<RwLock<HashMap<String, String>>>,
}

/// Response from DingTalk gateway connection registration.
#[derive(serde::Deserialize)]
struct GatewayResponse {
    endpoint: String,
    ticket: String,
}

impl DingTalkChannel {
    pub fn new(client_id: String, client_secret: String, allowed_users: Vec<String>) -> Self {
        Self {
            client_id,
            client_secret,
            allowed_users,
            session_webhooks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn http_client(&self) -> reqwest::Client {
        crate::alphahuman::config::build_runtime_proxy_client("channel.dingtalk")
    }

    fn is_user_allowed(&self, user_id: &str) -> bool {
        self.allowed_users.iter().any(|u| u == "*" || u == user_id)
    }

    fn parse_stream_data(frame: &serde_json::Value) -> Option<serde_json::Value> {
        match frame.get("data") {
            Some(serde_json::Value::String(raw)) => serde_json::from_str(raw).ok(),
            Some(serde_json::Value::Object(_)) => frame.get("data").cloned(),
            _ => None,
        }
    }

    fn resolve_chat_id(data: &serde_json::Value, sender_id: &str) -> String {
        let is_private_chat = data
            .get("conversationType")
            .and_then(|value| {
                value
                    .as_str()
                    .map(|v| v == "1")
                    .or_else(|| value.as_i64().map(|v| v == 1))
            })
            .unwrap_or(true);

        if is_private_chat {
            sender_id.to_string()
        } else {
            data.get("conversationId")
                .and_then(|c| c.as_str())
                .unwrap_or(sender_id)
                .to_string()
        }
    }

    /// Register a connection with DingTalk's gateway to get a WebSocket endpoint.
    async fn register_connection(&self) -> anyhow::Result<GatewayResponse> {
        let body = serde_json::json!({
            "clientId": self.client_id,
            "clientSecret": self.client_secret,
            "subscriptions": [
                {
                    "type": "CALLBACK",
                    "topic": DINGTALK_BOT_CALLBACK_TOPIC,
                }
            ],
        });

        let resp = self
            .http_client()
            .post("https://api.dingtalk.com/v1.0/gateway/connections/open")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            anyhow::bail!("DingTalk gateway registration failed ({status}): {err}");
        }

        let gw: GatewayResponse = resp.json().await?;
        Ok(gw)
    }
}

#[async_trait]
impl Channel for DingTalkChannel {
    fn name(&self) -> &str {
        "dingtalk"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        let webhooks = self.session_webhooks.read().await;
        let webhook_url = webhooks.get(&message.recipient).ok_or_else(|| {
            anyhow::anyhow!(
                "No session webhook found for chat {}. \
                 The user must send a message first to establish a session.",
                message.recipient
            )
        })?;

        let title = message.subject.as_deref().unwrap_or("Alphahuman");
        let body = serde_json::json!({
            "msgtype": "markdown",
            "markdown": {
                "title": title,
                "text": message.content,
            }
        });

        let resp = self
            .http_client()
            .post(webhook_url)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            anyhow::bail!("DingTalk webhook reply failed ({status}): {err}");
        }

        Ok(())
    }

    async fn listen(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        tracing::info!("DingTalk: registering gateway connection...");

        let gw = self.register_connection().await?;
        let ws_url = format!("{}?ticket={}", gw.endpoint, gw.ticket);

        tracing::info!("DingTalk: connecting to stream WebSocket...");
        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url).await?;
        let (mut write, mut read) = ws_stream.split();

        tracing::info!("DingTalk: connected and listening for messages...");

        while let Some(msg) = read.next().await {
            let msg = match msg {
                Ok(Message::Text(t)) => t,
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    tracing::warn!("DingTalk WebSocket error: {e}");
                    break;
                }
                _ => continue,
            };

            let frame: serde_json::Value = match serde_json::from_str(msg.as_ref()) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let frame_type = frame.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match frame_type {
                "SYSTEM" => {
                    // Respond to system pings to keep the connection alive
                    let message_id = frame
                        .get("headers")
                        .and_then(|h| h.get("messageId"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("");

                    let pong = serde_json::json!({
                        "code": 200,
                        "headers": {
                            "contentType": "application/json",
                            "messageId": message_id,
                        },
                        "message": "OK",
                        "data": "",
                    });

                    if let Err(e) = write.send(Message::Text(pong.to_string().into())).await {
                        tracing::warn!("DingTalk: failed to send pong: {e}");
                        break;
                    }
                }
                "EVENT" | "CALLBACK" => {
                    // Parse the chatbot callback data from the frame.
                    let data = match Self::parse_stream_data(&frame) {
                        Some(v) => v,
                        None => {
                            tracing::debug!("DingTalk: frame has no parseable data payload");
                            continue;
                        }
                    };

                    // Extract message content
                    let content = data
                        .get("text")
                        .and_then(|t| t.get("content"))
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .trim();

                    if content.is_empty() {
                        continue;
                    }

                    let sender_id = data
                        .get("senderStaffId")
                        .and_then(|s| s.as_str())
                        .unwrap_or("unknown");

                    if !self.is_user_allowed(sender_id) {
                        tracing::warn!(
                            "DingTalk: ignoring message from unauthorized user: {sender_id}"
                        );
                        continue;
                    }

                    // Private chat uses sender ID, group chat uses conversation ID.
                    let chat_id = Self::resolve_chat_id(&data, sender_id);

                    // Store session webhook for later replies
                    if let Some(webhook) = data.get("sessionWebhook").and_then(|w| w.as_str()) {
                        let webhook = webhook.to_string();
                        let mut webhooks = self.session_webhooks.write().await;
                        // Use both keys so reply routing works for both group and private flows.
                        webhooks.insert(chat_id.clone(), webhook.clone());
                        webhooks.insert(sender_id.to_string(), webhook);
                    }

                    // Acknowledge the event
                    let message_id = frame
                        .get("headers")
                        .and_then(|h| h.get("messageId"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("");

                    let ack = serde_json::json!({
                        "code": 200,
                        "headers": {
                            "contentType": "application/json",
                            "messageId": message_id,
                        },
                        "message": "OK",
                        "data": "",
                    });
                    let _ = write.send(Message::Text(ack.to_string().into())).await;

                    let channel_msg = ChannelMessage {
                        id: Uuid::new_v4().to_string(),
                        sender: sender_id.to_string(),
                        reply_target: chat_id,
                        content: content.to_string(),
                        channel: "dingtalk".to_string(),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        thread_ts: None,
                    };

                    if tx.send(channel_msg).await.is_err() {
                        tracing::warn!("DingTalk: message channel closed");
                        break;
                    }
                }
                _ => {}
            }
        }

        anyhow::bail!("DingTalk WebSocket stream ended")
    }

    async fn health_check(&self) -> bool {
        self.register_connection().await.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        let ch = DingTalkChannel::new("id".into(), "secret".into(), vec![]);
        assert_eq!(ch.name(), "dingtalk");
    }

    #[test]
    fn test_user_allowed_wildcard() {
        let ch = DingTalkChannel::new("id".into(), "secret".into(), vec!["*".into()]);
        assert!(ch.is_user_allowed("anyone"));
    }

    #[test]
    fn test_user_allowed_specific() {
        let ch = DingTalkChannel::new("id".into(), "secret".into(), vec!["user123".into()]);
        assert!(ch.is_user_allowed("user123"));
        assert!(!ch.is_user_allowed("other"));
    }

    #[test]
    fn test_user_denied_empty() {
        let ch = DingTalkChannel::new("id".into(), "secret".into(), vec![]);
        assert!(!ch.is_user_allowed("anyone"));
    }

    #[test]
    fn test_config_serde() {
        let toml_str = r#"
client_id = "app_id_123"
client_secret = "secret_456"
allowed_users = ["user1", "*"]
"#;
        let config: crate::alphahuman::config::schema::DingTalkConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.client_id, "app_id_123");
        assert_eq!(config.client_secret, "secret_456");
        assert_eq!(config.allowed_users, vec!["user1", "*"]);
    }

    #[test]
    fn test_config_serde_defaults() {
        let toml_str = r#"
client_id = "id"
client_secret = "secret"
"#;
        let config: crate::alphahuman::config::schema::DingTalkConfig = toml::from_str(toml_str).unwrap();
        assert!(config.allowed_users.is_empty());
    }

    #[test]
    fn parse_stream_data_supports_string_payload() {
        let frame = serde_json::json!({
            "data": "{\"text\":{\"content\":\"hello\"}}"
        });
        let parsed = DingTalkChannel::parse_stream_data(&frame).unwrap();
        assert_eq!(
            parsed.get("text").and_then(|v| v.get("content")),
            Some(&serde_json::json!("hello"))
        );
    }

    #[test]
    fn parse_stream_data_supports_object_payload() {
        let frame = serde_json::json!({
            "data": {"text": {"content": "hello"}}
        });
        let parsed = DingTalkChannel::parse_stream_data(&frame).unwrap();
        assert_eq!(
            parsed.get("text").and_then(|v| v.get("content")),
            Some(&serde_json::json!("hello"))
        );
    }

    #[test]
    fn resolve_chat_id_handles_numeric_group_conversation_type() {
        let data = serde_json::json!({
            "conversationType": 2,
            "conversationId": "cid-group",
        });
        let chat_id = DingTalkChannel::resolve_chat_id(&data, "staff-1");
        assert_eq!(chat_id, "cid-group");
    }
}
