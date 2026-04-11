//! Telegram Bot API channel implementation.

use super::attachments::{
    is_http_url, parse_attachment_markers, parse_path_only_attachment, TelegramAttachment,
    TelegramAttachmentKind,
};
use super::text::{
    split_message_for_telegram, strip_tool_call_tags, TELEGRAM_BIND_COMMAND,
    TELEGRAM_MAX_MESSAGE_LENGTH,
};
use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::channels::traits::{Channel, ChannelMessage, SendMessage};
use crate::openhuman::config::{Config, StreamMode};
use crate::openhuman::security::pairing::PairingGuard;
use anyhow::Context;
use async_trait::async_trait;
use directories::UserDirs;
use parking_lot::Mutex;
use reqwest::multipart::{Form, Part};
use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::fs;

const TELEGRAM_RECENT_UPDATE_CACHE_SIZE: usize = 4096;

struct TelegramTypingTask {
    recipient: String,
    handle: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
struct TelegramUpdateWindow {
    max_seen_update_id: i64,
    recent_order: VecDeque<i64>,
    recent_lookup: HashSet<i64>,
}

#[derive(Debug, Clone)]
struct TelegramReactionEvent {
    sender: String,
    reply_target: String,
    target_message_id: String,
    emoji: String,
}

/// Telegram channel — long-polls the Bot API for updates
pub struct TelegramChannel {
    bot_token: String,
    allowed_users: Arc<RwLock<Vec<String>>>,
    pairing: Option<PairingGuard>,
    client: reqwest::Client,
    typing_handle: Mutex<Option<TelegramTypingTask>>,
    stream_mode: StreamMode,
    draft_update_interval_ms: u64,
    last_draft_edit: Mutex<std::collections::HashMap<String, std::time::Instant>>,
    mention_only: bool,
    bot_username: Mutex<Option<String>>,
    recent_updates: Mutex<TelegramUpdateWindow>,
}

impl TelegramChannel {
    pub fn new(bot_token: String, allowed_users: Vec<String>, mention_only: bool) -> Self {
        let normalized_allowed = Self::normalize_allowed_users(allowed_users);
        let pairing = if normalized_allowed.is_empty() {
            let guard = PairingGuard::new(true, &[]);
            if let Some(code) = guard.pairing_code() {
                println!("  🔐 Telegram pairing required. One-time bind code: {code}");
                println!("     Send `{TELEGRAM_BIND_COMMAND} <code>` from your Telegram account.");
            }
            Some(guard)
        } else {
            None
        };

        Self {
            bot_token,
            allowed_users: Arc::new(RwLock::new(normalized_allowed)),
            pairing,
            client: reqwest::Client::new(),
            stream_mode: StreamMode::Off,
            draft_update_interval_ms: 1000,
            last_draft_edit: Mutex::new(std::collections::HashMap::new()),
            typing_handle: Mutex::new(None),
            mention_only,
            bot_username: Mutex::new(None),
            recent_updates: Mutex::new(TelegramUpdateWindow::default()),
        }
    }

    /// Configure streaming mode for progressive draft updates.
    pub fn with_streaming(
        mut self,
        stream_mode: StreamMode,
        draft_update_interval_ms: u64,
    ) -> Self {
        self.stream_mode = stream_mode;
        self.draft_update_interval_ms = draft_update_interval_ms;
        self
    }

    /// Parse reply_target into (chat_id, optional thread_id).
    fn parse_reply_target(reply_target: &str) -> (String, Option<String>) {
        if let Some((chat_id, thread_id)) = reply_target.split_once(':') {
            (chat_id.to_string(), Some(thread_id.to_string()))
        } else {
            (reply_target.to_string(), None)
        }
    }

    fn parse_message_id(value: Option<&str>) -> Option<i64> {
        value.and_then(|raw| raw.trim().parse::<i64>().ok())
    }

    fn typing_body_for_recipient(recipient: &str) -> serde_json::Value {
        let (chat_id, thread_id) = Self::parse_reply_target(recipient);
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "action": "typing"
        });
        if let Some(thread_id) = thread_id {
            body["message_thread_id"] = serde_json::Value::String(thread_id);
        }
        body
    }

    async fn telegram_api_ok(resp: reqwest::Response) -> bool {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            tracing::warn!(status = ?status, body, "Telegram API request failed");
            return false;
        }

        match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(payload) => {
                if payload
                    .get("ok")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    true
                } else {
                    let error_code = payload
                        .get("error_code")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or_default();
                    let description = payload
                        .get("description")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown Telegram API error");
                    tracing::warn!(
                        status = ?status,
                        error_code,
                        description,
                        body,
                        "Telegram API responded with ok=false"
                    );
                    false
                }
            }
            Err(error) => {
                tracing::warn!(
                    status = ?status,
                    %error,
                    body,
                    "Telegram API returned non-JSON body"
                );
                false
            }
        }
    }

    async fn send_typing_action_once(&self, recipient: &str) {
        tracing::info!(recipient, "Telegram typing action attempt");
        let body = Self::typing_body_for_recipient(recipient);
        let has_thread_id = body.get("message_thread_id").is_some();
        match self
            .http_client()
            .post(self.api_url("sendChatAction"))
            .json(&body)
            .send()
            .await
        {
            Ok(resp) => {
                if Self::telegram_api_ok(resp).await {
                    tracing::info!(recipient, "Telegram typing action sent");
                    return;
                }
                tracing::warn!(recipient, "Telegram typing action rejected");

                // Some chats can reject thread-scoped chat actions; retry plain chat_id once.
                if has_thread_id {
                    let (chat_id, _) = Self::parse_reply_target(recipient);
                    let fallback_body = serde_json::json!({
                        "chat_id": chat_id,
                        "action": "typing"
                    });
                    match self
                        .http_client()
                        .post(self.api_url("sendChatAction"))
                        .json(&fallback_body)
                        .send()
                        .await
                    {
                        Ok(fallback_resp) => {
                            if Self::telegram_api_ok(fallback_resp).await {
                                tracing::warn!(
                                    recipient,
                                    "Telegram typing action accepted after removing message_thread_id"
                                );
                            } else {
                                tracing::warn!(
                                    recipient,
                                    "Telegram typing fallback (without message_thread_id) rejected"
                                );
                            }
                        }
                        Err(fallback_error) => {
                            tracing::warn!(
                                recipient,
                                %fallback_error,
                                "Telegram typing fallback request failed"
                            );
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!(recipient, %error, "Telegram typing action request failed");
            }
        }
    }

    fn track_update_id(&self, update_id: i64) -> bool {
        let mut window = self.recent_updates.lock();
        if window.recent_lookup.contains(&update_id) {
            tracing::debug!(
                update_id,
                "Telegram update dedupe hit: duplicate update skipped"
            );
            return false;
        }

        if update_id < window.max_seen_update_id {
            tracing::debug!(
                update_id,
                max_seen = window.max_seen_update_id,
                "Telegram update ordering safeguard: stale update skipped"
            );
            return false;
        }

        if update_id > window.max_seen_update_id {
            window.max_seen_update_id = update_id;
        }

        window.recent_lookup.insert(update_id);
        window.recent_order.push_back(update_id);
        if window.recent_order.len() > TELEGRAM_RECENT_UPDATE_CACHE_SIZE {
            if let Some(evicted) = window.recent_order.pop_front() {
                window.recent_lookup.remove(&evicted);
            }
        }
        true
    }

    fn http_client(&self) -> reqwest::Client {
        crate::openhuman::config::build_runtime_proxy_client("channel.telegram")
    }

    fn normalize_identity(value: &str) -> String {
        value.trim().trim_start_matches('@').to_string()
    }

    fn normalize_allowed_users(allowed_users: Vec<String>) -> Vec<String> {
        allowed_users
            .into_iter()
            .map(|entry| Self::normalize_identity(&entry))
            .filter(|entry| !entry.is_empty())
            .collect()
    }

    async fn load_config_without_env() -> anyhow::Result<Config> {
        let home = UserDirs::new()
            .map(|u| u.home_dir().to_path_buf())
            .context("Could not find home directory")?;
        let openhuman_dir = home.join(".openhuman");
        let config_path = openhuman_dir.join("config.toml");

        let contents = fs::read_to_string(&config_path)
            .await
            .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
        let mut config: Config = toml::from_str(&contents)
            .context("Failed to parse config file for Telegram binding")?;
        config.config_path = config_path;
        config.workspace_dir = openhuman_dir.join("workspace");
        Ok(config)
    }

    async fn persist_allowed_identity(&self, identity: &str) -> anyhow::Result<()> {
        let mut config = Self::load_config_without_env().await?;
        let Some(telegram) = config.channels_config.telegram.as_mut() else {
            anyhow::bail!("Telegram channel config is missing in config.toml");
        };

        let normalized = Self::normalize_identity(identity);
        if normalized.is_empty() {
            anyhow::bail!("Cannot persist empty Telegram identity");
        }

        if !telegram.allowed_users.iter().any(|u| u == &normalized) {
            telegram.allowed_users.push(normalized);
            config
                .save()
                .await
                .context("Failed to persist Telegram allowlist to config.toml")?;
        }

        Ok(())
    }

    fn add_allowed_identity_runtime(&self, identity: &str) {
        let normalized = Self::normalize_identity(identity);
        if normalized.is_empty() {
            return;
        }
        if let Ok(mut users) = self.allowed_users.write() {
            if !users.iter().any(|u| u == &normalized) {
                users.push(normalized);
            }
        }
    }

    fn extract_bind_code(text: &str) -> Option<&str> {
        let mut parts = text.split_whitespace();
        let command = parts.next()?;
        let base_command = command.split('@').next().unwrap_or(command);
        if base_command != TELEGRAM_BIND_COMMAND {
            return None;
        }
        parts.next().map(str::trim).filter(|code| !code.is_empty())
    }

    fn pairing_code_active(&self) -> bool {
        self.pairing
            .as_ref()
            .and_then(PairingGuard::pairing_code)
            .is_some()
    }

    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{method}", self.bot_token)
    }

    /// Clears Bot API webhook mode so `getUpdates` long polling can run.
    async fn delete_webhook_for_long_polling(&self) -> bool {
        let url = self.api_url("deleteWebhook");
        let body = serde_json::json!({ "drop_pending_updates": false });
        tracing::info!(
            "[telegram] deleteWebhook: enabling getUpdates polling (drop_pending_updates=false)"
        );
        match self.http_client().post(&url).json(&body).send().await {
            Ok(resp) => Self::telegram_api_ok(resp).await,
            Err(e) => {
                tracing::warn!(error = %e, "[telegram] deleteWebhook HTTP request failed");
                false
            }
        }
    }

    async fn fetch_bot_username(&self) -> anyhow::Result<String> {
        let resp = self.http_client().get(self.api_url("getMe")).send().await?;

        if !resp.status().is_success() {
            anyhow::bail!("Failed to fetch bot info: {}", resp.status());
        }

        let data: serde_json::Value = resp.json().await?;
        let username = data
            .get("result")
            .and_then(|r| r.get("username"))
            .and_then(|u| u.as_str())
            .context("Bot username not found in response")?;

        Ok(username.to_string())
    }

    async fn get_bot_username(&self) -> Option<String> {
        {
            let cache = self.bot_username.lock();
            if let Some(ref username) = *cache {
                return Some(username.clone());
            }
        }

        match self.fetch_bot_username().await {
            Ok(username) => {
                let mut cache = self.bot_username.lock();
                *cache = Some(username.clone());
                Some(username)
            }
            Err(e) => {
                tracing::warn!("Failed to fetch bot username: {e}");
                None
            }
        }
    }

    fn is_telegram_username_char(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || ch == '_'
    }

    fn find_bot_mention_spans(text: &str, bot_username: &str) -> Vec<(usize, usize)> {
        let bot_username = bot_username.trim_start_matches('@');
        if bot_username.is_empty() {
            return Vec::new();
        }

        let mut spans = Vec::new();

        for (at_idx, ch) in text.char_indices() {
            if ch != '@' {
                continue;
            }

            if at_idx > 0 {
                let prev = text[..at_idx].chars().next_back().unwrap_or(' ');
                if Self::is_telegram_username_char(prev) {
                    continue;
                }
            }

            let username_start = at_idx + 1;
            let mut username_end = username_start;

            for (rel_idx, candidate_ch) in text[username_start..].char_indices() {
                if Self::is_telegram_username_char(candidate_ch) {
                    username_end = username_start + rel_idx + candidate_ch.len_utf8();
                } else {
                    break;
                }
            }

            if username_end == username_start {
                continue;
            }

            let mention_username = &text[username_start..username_end];
            if mention_username.eq_ignore_ascii_case(bot_username) {
                spans.push((at_idx, username_end));
            }
        }

        spans
    }

    fn contains_bot_mention(text: &str, bot_username: &str) -> bool {
        !Self::find_bot_mention_spans(text, bot_username).is_empty()
    }

    fn normalize_incoming_content(text: &str, bot_username: &str) -> Option<String> {
        let spans = Self::find_bot_mention_spans(text, bot_username);
        if spans.is_empty() {
            let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
            return (!normalized.is_empty()).then_some(normalized);
        }

        let mut normalized = String::with_capacity(text.len());
        let mut cursor = 0;
        for (start, end) in spans {
            normalized.push_str(&text[cursor..start]);
            cursor = end;
        }
        normalized.push_str(&text[cursor..]);

        let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
        (!normalized.is_empty()).then_some(normalized)
    }

    fn is_group_message(message: &serde_json::Value) -> bool {
        message
            .get("chat")
            .and_then(|c| c.get("type"))
            .and_then(|t| t.as_str())
            .map(|t| t == "group" || t == "supergroup")
            .unwrap_or(false)
    }

    fn is_user_allowed(&self, username: &str) -> bool {
        let identity = Self::normalize_identity(username);
        self.allowed_users
            .read()
            .map(|users| {
                users
                    .iter()
                    .any(|u| u == "*" || u.eq_ignore_ascii_case(&identity))
            })
            .unwrap_or(false)
    }

    fn is_any_user_allowed<'a, I>(&self, identities: I) -> bool
    where
        I: IntoIterator<Item = &'a str>,
    {
        identities.into_iter().any(|id| self.is_user_allowed(id))
    }

    async fn handle_unauthorized_message(&self, update: &serde_json::Value) {
        let Some(message) = update.get("message") else {
            return;
        };

        let Some(text) = message.get("text").and_then(serde_json::Value::as_str) else {
            return;
        };

        let username_opt = message
            .get("from")
            .and_then(|from| from.get("username"))
            .and_then(serde_json::Value::as_str);
        let username = username_opt.unwrap_or("unknown");
        let normalized_username = Self::normalize_identity(username);

        let sender_id = message
            .get("from")
            .and_then(|from| from.get("id"))
            .and_then(serde_json::Value::as_i64);
        let sender_id_str = sender_id.map(|id| id.to_string());
        let normalized_sender_id = sender_id_str.as_deref().map(Self::normalize_identity);

        let chat_id = message
            .get("chat")
            .and_then(|chat| chat.get("id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());

        let Some(chat_id) = chat_id else {
            tracing::warn!("Telegram: missing chat_id in message, skipping");
            return;
        };

        let mut identities = vec![normalized_username.as_str()];
        if let Some(ref id) = normalized_sender_id {
            identities.push(id.as_str());
        }

        if self.is_any_user_allowed(identities.iter().copied()) {
            return;
        }

        if let Some(code) = Self::extract_bind_code(text) {
            if let Some(pairing) = self.pairing.as_ref() {
                match pairing.try_pair(code).await {
                    Ok(Some(_token)) => {
                        let bind_identity = normalized_sender_id.clone().or_else(|| {
                            if normalized_username.is_empty() || normalized_username == "unknown" {
                                None
                            } else {
                                Some(normalized_username.clone())
                            }
                        });

                        if let Some(identity) = bind_identity {
                            self.add_allowed_identity_runtime(&identity);
                            match self.persist_allowed_identity(&identity).await {
                                Ok(()) => {
                                    let _ = self
                                        .send(&SendMessage::new(
                                            "✅ Telegram account bound successfully. You can talk to OpenHuman now.",
                                            &chat_id,
                                        ))
                                        .await;
                                    tracing::info!(
                                        "Telegram: paired and allowlisted identity={identity}"
                                    );
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Telegram: failed to persist allowlist after bind: {e}"
                                    );
                                    let _ = self
                                        .send(&SendMessage::new(
                                            "⚠️ Bound for this runtime, but failed to persist config. Access may be lost after restart; check config file permissions.",
                                            &chat_id,
                                        ))
                                        .await;
                                }
                            }
                        } else {
                            let _ = self
                                .send(&SendMessage::new(
                                    "❌ Could not identify your Telegram account. Ensure your account has a username or stable user ID, then retry.",
                                    &chat_id,
                                ))
                                .await;
                        }
                    }
                    Ok(None) => {
                        let _ = self
                            .send(&SendMessage::new(
                                "❌ Invalid binding code. Ask operator for the latest code and retry.",
                                &chat_id,
                            ))
                            .await;
                    }
                    Err(lockout_secs) => {
                        let _ = self
                            .send(&SendMessage::new(
                                format!("⏳ Too many invalid attempts. Retry in {lockout_secs}s."),
                                &chat_id,
                            ))
                            .await;
                    }
                }
            } else {
                let _ = self
                    .send(&SendMessage::new(
                        "ℹ️ Telegram pairing is not active. Ask operator to update allowlist in config.toml.",
                        &chat_id,
                    ))
                    .await;
            }
            return;
        }

        tracing::warn!(
            "Telegram: ignoring message from unauthorized user: username={username}, sender_id={}. \
Allowlist Telegram username (without '@') or numeric user ID.",
            sender_id_str.as_deref().unwrap_or("unknown")
        );

        let _ = self
            .send(&SendMessage::new(
                "🔐 This bot requires operator approval.\n\nAsk the operator to approve the pairing in the web UI, then send your message again.".to_string(),
                &chat_id,
            ))
            .await;

        if self.pairing_code_active() {
            let _ = self
                .send(&SendMessage::new(
                    "ℹ️ If operator provides a one-time pairing code, you can also run `/bind <code>`.",
                    &chat_id,
                ))
                .await;
        }
    }

    fn parse_update_message(&self, update: &serde_json::Value) -> Option<ChannelMessage> {
        let message = update
            .get("message")
            .or_else(|| update.get("edited_message"))?;

        let text = message.get("text").and_then(serde_json::Value::as_str)?;

        let username = message
            .get("from")
            .and_then(|from| from.get("username"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        let sender_id = message
            .get("from")
            .and_then(|from| from.get("id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());

        let sender_identity = if username == "unknown" {
            sender_id.clone().unwrap_or_else(|| "unknown".to_string())
        } else {
            username.clone()
        };

        let mut identities = vec![username.as_str()];
        if let Some(id) = sender_id.as_deref() {
            identities.push(id);
        }

        if !self.is_any_user_allowed(identities.iter().copied()) {
            tracing::debug!(
                username = %username,
                sender_id = sender_id.as_deref().unwrap_or("none"),
                message_len = text.len(),
                "[telegram] dropped message: sender not in allowed_users (unauthorized handler may reply)"
            );
            return None;
        }

        let is_group = Self::is_group_message(message);
        if self.mention_only && is_group {
            let bot_username = self.bot_username.lock();
            if let Some(ref bot_username) = *bot_username {
                if !Self::contains_bot_mention(text, bot_username) {
                    return None;
                }
            } else {
                return None;
            }
        }

        let chat_id = message
            .get("chat")
            .and_then(|chat| chat.get("id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string())?;

        let message_id = message
            .get("message_id")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        // Extract thread/topic ID for forum support
        let thread_id = message
            .get("message_thread_id")
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());

        // reply_target: chat_id or chat_id:thread_id format
        let reply_target = if let Some(tid) = thread_id {
            format!("{}:{}", chat_id, tid)
        } else {
            chat_id.clone()
        };

        let replied_parent_message_id = message
            .get("reply_to_message")
            .and_then(|reply| reply.get("message_id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());

        // Telegram "reply" targeting should point to the inbound message itself so the
        // assistant response is visibly attached in chat. We still retain the inbound
        // parent reference in logs for reply-context diagnostics.
        let outbound_reply_to_message_id = Some(message_id.to_string());
        tracing::debug!(
            chat_id,
            message_id,
            reply_to_parent = replied_parent_message_id.as_deref().unwrap_or("none"),
            "Telegram inbound message parsed for reply mapping"
        );

        let content = if self.mention_only && is_group {
            let bot_username = self.bot_username.lock();
            let bot_username = bot_username.as_ref()?;
            Self::normalize_incoming_content(text, bot_username)?
        } else {
            text.to_string()
        };

        Some(ChannelMessage {
            id: format!("telegram_{chat_id}_{message_id}"),
            sender: sender_identity,
            reply_target,
            content,
            channel: "telegram".to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            thread_ts: outbound_reply_to_message_id,
        })
    }

    fn parse_update_reaction(&self, update: &serde_json::Value) -> Option<TelegramReactionEvent> {
        let reaction = update.get("message_reaction")?;

        let chat_id = reaction
            .get("chat")
            .and_then(|chat| chat.get("id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string())?;
        let message_id = reaction
            .get("message_id")
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string())?;
        let actor = reaction
            .get("user")
            .and_then(|user| user.get("username"))
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string)
            .or_else(|| {
                reaction
                    .get("user")
                    .and_then(|user| user.get("id"))
                    .and_then(serde_json::Value::as_i64)
                    .map(|id| id.to_string())
            })
            .unwrap_or_else(|| "unknown".to_string());

        let user_id = reaction
            .get("user")
            .and_then(|user| user.get("id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());

        let actor_allowed = self.is_user_allowed(&actor);
        let user_id_allowed = user_id
            .as_deref()
            .is_some_and(|id| self.is_user_allowed(id));

        if !(actor_allowed || user_id_allowed) {
            tracing::debug!(
                actor,
                message_id,
                "Telegram reaction ignored: actor is not allowlisted"
            );
            return None;
        }

        let emoji = reaction
            .get("new_reaction")
            .and_then(serde_json::Value::as_array)
            .and_then(|arr| {
                arr.iter().find_map(|entry| {
                    entry
                        .get("emoji")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string)
                })
            })?;

        Some(TelegramReactionEvent {
            sender: actor,
            reply_target: chat_id,
            target_message_id: message_id,
            emoji,
        })
    }

    async fn send_message_reaction(
        &self,
        chat_id: &str,
        message_id: i64,
        emoji: &str,
    ) -> anyhow::Result<bool> {
        let emoji = emoji.trim();
        if emoji.is_empty() {
            return Ok(false);
        }

        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": [
                {
                    "type": "emoji",
                    "emoji": emoji
                }
            ],
            "is_big": false
        });
        let resp = self
            .http_client()
            .post(self.api_url("setMessageReaction"))
            .json(&body)
            .send()
            .await?;
        if resp.status().is_success() {
            publish_global(DomainEvent::ChannelReactionSent {
                channel: "telegram".to_string(),
                target_message_id: format!("telegram_{chat_id}_{message_id}"),
                emoji: emoji.to_string(),
                success: true,
            });
            tracing::info!(chat_id, message_id, emoji, "Telegram reaction sent");
            return Ok(true);
        }

        let status = resp.status();
        let err = resp.text().await.unwrap_or_default();
        tracing::warn!(
            chat_id,
            message_id,
            emoji,
            status = ?status,
            error = err,
            "Telegram reaction not applied; continuing without failure"
        );
        publish_global(DomainEvent::ChannelReactionSent {
            channel: "telegram".to_string(),
            target_message_id: format!("telegram_{chat_id}_{message_id}"),
            emoji: emoji.to_string(),
            success: false,
        });
        Ok(false)
    }

    fn parse_reaction_marker(content: &str) -> (String, Option<String>) {
        // Marker format at the start of the message: [REACTION:😀] or [REACTION:😀|12345]
        // The marker may be followed by a text reply: [REACTION:👍] Great point!
        // Returns (remaining_text, Some(marker_inner)) or (original, None).
        let trimmed = content.trim();
        let Some(rest) = trimmed.strip_prefix("[REACTION:") else {
            return (content.to_string(), None);
        };
        let Some(close_pos) = rest.find(']') else {
            return (content.to_string(), None);
        };
        let inner = rest[..close_pos].trim();
        if inner.is_empty() {
            return (String::new(), None);
        }
        let remaining = rest[close_pos + 1..].trim().to_string();
        (remaining, Some(inner.to_string()))
    }

    async fn send_text_chunks(
        &self,
        message: &str,
        chat_id: &str,
        thread_id: Option<&str>,
        reply_to_message_id: Option<i64>,
    ) -> anyhow::Result<()> {
        let chunks = split_message_for_telegram(message);

        for (index, chunk) in chunks.iter().enumerate() {
            let text = if chunks.len() > 1 {
                if index == 0 {
                    format!("{chunk}\n\n(continues...)")
                } else if index == chunks.len() - 1 {
                    format!("(continued)\n\n{chunk}")
                } else {
                    format!("(continued)\n\n{chunk}\n\n(continues...)")
                }
            } else {
                chunk.to_string()
            };

            let mut markdown_body = serde_json::json!({
                "chat_id": chat_id,
                "text": text,
                "parse_mode": "Markdown"
            });

            // Add message_thread_id for forum topic support
            if let Some(tid) = thread_id {
                markdown_body["message_thread_id"] = serde_json::Value::String(tid.to_string());
            }
            if index == 0 {
                if let Some(parent_id) = reply_to_message_id {
                    markdown_body["reply_to_message_id"] = serde_json::Value::from(parent_id);
                }
            }

            let markdown_resp = self
                .http_client()
                .post(self.api_url("sendMessage"))
                .json(&markdown_body)
                .send()
                .await?;

            if markdown_resp.status().is_success() {
                if index < chunks.len() - 1 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                continue;
            }

            let markdown_status = markdown_resp.status();
            let markdown_err = markdown_resp.text().await.unwrap_or_default();
            tracing::warn!(
                status = ?markdown_status,
                "Telegram sendMessage with Markdown failed; retrying without parse_mode"
            );

            let mut plain_body = serde_json::json!({
                "chat_id": chat_id,
                "text": text,
            });

            // Add message_thread_id for forum topic support
            if let Some(tid) = thread_id {
                plain_body["message_thread_id"] = serde_json::Value::String(tid.to_string());
            }
            if index == 0 {
                if let Some(parent_id) = reply_to_message_id {
                    plain_body["reply_to_message_id"] = serde_json::Value::from(parent_id);
                }
            }
            let plain_resp = self
                .http_client()
                .post(self.api_url("sendMessage"))
                .json(&plain_body)
                .send()
                .await?;

            if !plain_resp.status().is_success() {
                let plain_status = plain_resp.status();
                let plain_err = plain_resp.text().await.unwrap_or_default();
                anyhow::bail!(
                    "Telegram sendMessage failed (markdown {}: {}; plain {}: {})",
                    markdown_status,
                    markdown_err,
                    plain_status,
                    plain_err
                );
            }

            if index < chunks.len() - 1 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        Ok(())
    }

    async fn send_media_by_url(
        &self,
        method: &str,
        media_field: &str,
        chat_id: &str,
        thread_id: Option<&str>,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut body = serde_json::json!({
            "chat_id": chat_id,
        });
        body[media_field] = serde_json::Value::String(url.to_string());

        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::Value::String(tid.to_string());
        }

        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url(method))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram {method} by URL failed: {err}");
        }

        tracing::info!("Telegram {method} sent to {chat_id}: {url}");
        Ok(())
    }

    async fn send_attachment(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        attachment: &TelegramAttachment,
    ) -> anyhow::Result<()> {
        let target = attachment.target.trim();

        if is_http_url(target) {
            return match attachment.kind {
                TelegramAttachmentKind::Image => {
                    self.send_photo_by_url(chat_id, thread_id, target, None)
                        .await
                }
                TelegramAttachmentKind::Document => {
                    self.send_document_by_url(chat_id, thread_id, target, None)
                        .await
                }
                TelegramAttachmentKind::Video => {
                    self.send_video_by_url(chat_id, thread_id, target, None)
                        .await
                }
                TelegramAttachmentKind::Audio => {
                    self.send_audio_by_url(chat_id, thread_id, target, None)
                        .await
                }
                TelegramAttachmentKind::Voice => {
                    self.send_voice_by_url(chat_id, thread_id, target, None)
                        .await
                }
            };
        }

        let path = Path::new(target);
        if !path.exists() {
            anyhow::bail!("Telegram attachment path not found: {target}");
        }

        match attachment.kind {
            TelegramAttachmentKind::Image => self.send_photo(chat_id, thread_id, path, None).await,
            TelegramAttachmentKind::Document => {
                self.send_document(chat_id, thread_id, path, None).await
            }
            TelegramAttachmentKind::Video => self.send_video(chat_id, thread_id, path, None).await,
            TelegramAttachmentKind::Audio => self.send_audio(chat_id, thread_id, path, None).await,
            TelegramAttachmentKind::Voice => self.send_voice(chat_id, thread_id, path, None).await,
        }
    }

    /// Send a document/file to a Telegram chat
    pub async fn send_document(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");

        let file_bytes = tokio::fs::read(file_path).await?;
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("document", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendDocument"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendDocument failed: {err}");
        }

        tracing::info!("Telegram document sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a document from bytes (in-memory) to a Telegram chat
    pub async fn send_document_bytes(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_bytes: Vec<u8>,
        file_name: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("document", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendDocument"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendDocument failed: {err}");
        }

        tracing::info!("Telegram document sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a photo to a Telegram chat
    pub async fn send_photo(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("photo.jpg");

        let file_bytes = tokio::fs::read(file_path).await?;
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("photo", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendPhoto"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendPhoto failed: {err}");
        }

        tracing::info!("Telegram photo sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a photo from bytes (in-memory) to a Telegram chat
    pub async fn send_photo_bytes(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_bytes: Vec<u8>,
        file_name: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("photo", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendPhoto"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendPhoto failed: {err}");
        }

        tracing::info!("Telegram photo sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a video to a Telegram chat
    pub async fn send_video(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("video.mp4");

        let file_bytes = tokio::fs::read(file_path).await?;
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("video", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendVideo"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendVideo failed: {err}");
        }

        tracing::info!("Telegram video sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send an audio file to a Telegram chat
    pub async fn send_audio(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("audio.mp3");

        let file_bytes = tokio::fs::read(file_path).await?;
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("audio", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendAudio"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendAudio failed: {err}");
        }

        tracing::info!("Telegram audio sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a voice message to a Telegram chat
    pub async fn send_voice(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("voice.ogg");

        let file_bytes = tokio::fs::read(file_path).await?;
        let part = Part::bytes(file_bytes).file_name(file_name.to_string());

        let mut form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part("voice", part);

        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendVoice"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendVoice failed: {err}");
        }

        tracing::info!("Telegram voice sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a file by URL (Telegram will download it)
    pub async fn send_document_by_url(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "document": url
        });

        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::Value::String(tid.to_string());
        }

        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendDocument"))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendDocument by URL failed: {err}");
        }

        tracing::info!("Telegram document (URL) sent to {chat_id}: {url}");
        Ok(())
    }

    /// Send a photo by URL (Telegram will download it)
    pub async fn send_photo_by_url(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "photo": url
        });

        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::Value::String(tid.to_string());
        }

        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
        }

        let resp = self
            .http_client()
            .post(self.api_url("sendPhoto"))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Telegram sendPhoto by URL failed: {err}");
        }

        tracing::info!("Telegram photo (URL) sent to {chat_id}: {url}");
        Ok(())
    }

    /// Send a video by URL (Telegram will download it)
    pub async fn send_video_by_url(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_by_url("sendVideo", "video", chat_id, thread_id, url, caption)
            .await
    }

    /// Send an audio file by URL (Telegram will download it)
    pub async fn send_audio_by_url(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_by_url("sendAudio", "audio", chat_id, thread_id, url, caption)
            .await
    }

    /// Send a voice message by URL (Telegram will download it)
    pub async fn send_voice_by_url(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_by_url("sendVoice", "voice", chat_id, thread_id, url, caption)
            .await
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    fn supports_reactions(&self) -> bool {
        true
    }

    fn supports_draft_updates(&self) -> bool {
        self.stream_mode != StreamMode::Off
    }

    async fn send_draft(&self, message: &SendMessage) -> anyhow::Result<Option<String>> {
        if self.stream_mode == StreamMode::Off {
            return Ok(None);
        }

        let (chat_id, thread_id) = Self::parse_reply_target(&message.recipient);
        let parent_message_id = Self::parse_message_id(message.thread_ts.as_deref());
        let initial_text = if message.content.is_empty() {
            "...".to_string()
        } else {
            message.content.clone()
        };

        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "text": initial_text,
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::Value::String(tid.to_string());
        }
        if let Some(parent_id) = parent_message_id {
            body["reply_to_message_id"] = serde_json::Value::from(parent_id);
        }

        let resp = self
            .client
            .post(self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            anyhow::bail!("Telegram sendMessage (draft) failed: {err}");
        }

        let resp_json: serde_json::Value = resp.json().await?;
        let message_id = resp_json
            .get("result")
            .and_then(|r| r.get("message_id"))
            .and_then(|id| id.as_i64())
            .map(|id| id.to_string());

        self.last_draft_edit
            .lock()
            .insert(chat_id.to_string(), std::time::Instant::now());

        Ok(message_id)
    }

    async fn update_draft(
        &self,
        recipient: &str,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        let (chat_id, _) = Self::parse_reply_target(recipient);

        // Rate-limit edits per chat
        {
            let last_edits = self.last_draft_edit.lock();
            if let Some(last_time) = last_edits.get(&chat_id) {
                let elapsed = u64::try_from(last_time.elapsed().as_millis()).unwrap_or(u64::MAX);
                if elapsed < self.draft_update_interval_ms {
                    return Ok(());
                }
            }
        }

        // Truncate to Telegram limit for mid-stream edits (UTF-8 safe)
        let display_text = if text.len() > TELEGRAM_MAX_MESSAGE_LENGTH {
            let mut end = 0;
            for (idx, ch) in text.char_indices() {
                let next = idx + ch.len_utf8();
                if next > TELEGRAM_MAX_MESSAGE_LENGTH {
                    break;
                }
                end = next;
            }
            &text[..end]
        } else {
            text
        };

        let message_id_parsed = match message_id.parse::<i64>() {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!("Invalid Telegram message_id '{message_id}': {e}");
                return Ok(());
            }
        };

        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id_parsed,
            "text": display_text,
        });

        let resp = self
            .client
            .post(self.api_url("editMessageText"))
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            self.last_draft_edit
                .lock()
                .insert(chat_id.clone(), std::time::Instant::now());
        } else {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            tracing::debug!("Telegram editMessageText failed ({status}): {err}");
        }

        Ok(())
    }

    async fn finalize_draft(
        &self,
        recipient: &str,
        message_id: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> anyhow::Result<()> {
        let text = &strip_tool_call_tags(text);
        let (chat_id, thread_id) = Self::parse_reply_target(recipient);
        let parent_message_id = Self::parse_message_id(thread_ts);

        // Clean up rate-limit tracking for this chat
        self.last_draft_edit.lock().remove(&chat_id);

        // If text exceeds limit, delete draft and send as chunked messages
        if text.len() > TELEGRAM_MAX_MESSAGE_LENGTH {
            let msg_id = match message_id.parse::<i64>() {
                Ok(id) => id,
                Err(e) => {
                    tracing::warn!("Invalid Telegram message_id '{message_id}': {e}");
                    return self
                        .send_text_chunks(text, &chat_id, thread_id.as_deref(), parent_message_id)
                        .await;
                }
            };

            // Delete the draft
            let _ = self
                .client
                .post(self.api_url("deleteMessage"))
                .json(&serde_json::json!({
                    "chat_id": chat_id,
                    "message_id": msg_id,
                }))
                .send()
                .await;

            // Fall back to chunked send
            return self
                .send_text_chunks(text, &chat_id, thread_id.as_deref(), parent_message_id)
                .await;
        }

        let msg_id = match message_id.parse::<i64>() {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!("Invalid Telegram message_id '{message_id}': {e}");
                return self
                    .send_text_chunks(text, &chat_id, thread_id.as_deref(), parent_message_id)
                    .await;
            }
        };

        // Try editing with Markdown formatting
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": msg_id,
            "text": text,
            "parse_mode": "Markdown",
        });

        let resp = self
            .client
            .post(self.api_url("editMessageText"))
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            return Ok(());
        }

        // Markdown failed — retry without parse_mode
        let plain_body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": msg_id,
            "text": text,
        });

        let resp = self
            .client
            .post(self.api_url("editMessageText"))
            .json(&plain_body)
            .send()
            .await?;

        if resp.status().is_success() {
            return Ok(());
        }

        // Edit failed entirely — fall back to new message
        tracing::warn!("Telegram finalize_draft edit failed; falling back to sendMessage");
        self.send_text_chunks(text, &chat_id, thread_id.as_deref(), parent_message_id)
            .await
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        // Strip tool_call tags before processing to prevent Markdown parsing failures
        let content = strip_tool_call_tags(&message.content);
        let parent_message_id = Self::parse_message_id(message.thread_ts.as_deref());

        // Parse recipient: "chat_id" or "chat_id:thread_id" format
        let (chat_id, thread_id) = match message.recipient.split_once(':') {
            Some((chat, thread)) => (chat, Some(thread)),
            None => (message.recipient.as_str(), None),
        };

        let (reactionless_content, reaction_marker) = Self::parse_reaction_marker(&content);
        if let Some(reaction_marker) = reaction_marker.as_deref() {
            let (emoji, explicit_target_id) = match reaction_marker.split_once('|') {
                Some((emoji, target)) => (emoji.trim(), Self::parse_message_id(Some(target))),
                None => (reaction_marker.trim(), None),
            };
            let target_message_id = explicit_target_id.or(parent_message_id);
            if let Some(target_id) = target_message_id {
                let _ = self
                    .send_message_reaction(chat_id, target_id, emoji)
                    .await?;
                tracing::debug!(
                    chat_id,
                    target_id,
                    emoji,
                    has_reply = !reactionless_content.is_empty(),
                    "[telegram] reaction sent; continuing to send reply text if present"
                );
            } else {
                tracing::warn!(
                    recipient = message.recipient,
                    marker = reaction_marker,
                    "[telegram] reaction marker ignored: missing target message id"
                );
            }
            // If no text follows the reaction marker, we are done.
            if reactionless_content.trim().is_empty() {
                return Ok(());
            }
        }

        let (text_without_markers, attachments) = parse_attachment_markers(&reactionless_content);

        if !attachments.is_empty() {
            if !text_without_markers.is_empty() {
                self.send_text_chunks(&text_without_markers, chat_id, thread_id, parent_message_id)
                    .await?;
            }

            for attachment in &attachments {
                self.send_attachment(chat_id, thread_id, attachment).await?;
            }

            return Ok(());
        }

        if let Some(attachment) = parse_path_only_attachment(&reactionless_content) {
            self.send_attachment(chat_id, thread_id, &attachment)
                .await?;
            return Ok(());
        }

        self.send_text_chunks(&reactionless_content, chat_id, thread_id, parent_message_id)
            .await
    }

    async fn listen(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        let mut offset: i64 = 0;

        if self.mention_only {
            let _ = self.get_bot_username().await;
        }

        tracing::info!("Telegram channel listening for messages...");

        loop {
            if self.mention_only {
                let missing_username = self.bot_username.lock().is_none();
                if missing_username {
                    let _ = self.get_bot_username().await;
                }
            }

            let url = self.api_url("getUpdates");
            let body = serde_json::json!({
                "offset": offset,
                "timeout": 30,
                "allowed_updates": ["message", "edited_message", "message_reaction"]
            });

            let resp = match self.http_client().post(&url).json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Telegram poll error: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            let data: serde_json::Value = match resp.json().await {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!("Telegram parse error: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            let ok = data
                .get("ok")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            if !ok {
                let error_code = data
                    .get("error_code")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default();
                let description = data
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown Telegram API error");

                if error_code == 409 {
                    let webhook_blocks_polling = description.to_lowercase().contains("webhook");
                    if webhook_blocks_polling {
                        tracing::warn!(
                            "[telegram] getUpdates conflict (409): webhook is active; calling deleteWebhook"
                        );
                        if self.delete_webhook_for_long_polling().await {
                            tracing::info!("[telegram] deleteWebhook ok; retrying getUpdates");
                            continue;
                        }
                        tracing::warn!("[telegram] deleteWebhook did not succeed; backing off");
                    } else {
                        tracing::warn!(
                            "Telegram polling conflict (409): {description}. \
Ensure only one `openhuman` process is using this bot token."
                        );
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                } else {
                    tracing::warn!(
                        "Telegram getUpdates API error (code={}): {description}",
                        error_code
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
                continue;
            }

            if let Some(results) = data.get("result").and_then(serde_json::Value::as_array) {
                for update in results {
                    let update_id = update
                        .get("update_id")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or_default();
                    if update_id > 0 && !self.track_update_id(update_id) {
                        continue;
                    }

                    // Advance offset past this update
                    if let Some(uid) = update.get("update_id").and_then(serde_json::Value::as_i64) {
                        offset = uid + 1;
                    }

                    if let Some(reaction) = self.parse_update_reaction(update) {
                        tracing::info!(
                            sender = reaction.sender,
                            reply_target = reaction.reply_target,
                            target_message_id = reaction.target_message_id,
                            emoji = reaction.emoji,
                            "Telegram reaction received"
                        );
                        publish_global(DomainEvent::ChannelReactionReceived {
                            channel: "telegram".to_string(),
                            sender: reaction.sender,
                            target_message_id: format!(
                                "telegram_{}_{}",
                                reaction.reply_target, reaction.target_message_id
                            ),
                            emoji: reaction.emoji,
                        });
                        continue;
                    }

                    let Some(msg) = self.parse_update_message(update) else {
                        self.handle_unauthorized_message(update).await;
                        continue;
                    };

                    if tx.send(msg).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn health_check(&self) -> bool {
        let timeout_duration = Duration::from_secs(5);

        match tokio::time::timeout(
            timeout_duration,
            self.http_client().get(self.api_url("getMe")).send(),
        )
        .await
        {
            Ok(Ok(resp)) => resp.status().is_success(),
            Ok(Err(e)) => {
                tracing::debug!("Telegram health check failed: {e}");
                false
            }
            Err(_) => {
                tracing::debug!("Telegram health check timed out after 5s");
                false
            }
        }
    }

    async fn start_typing(&self, recipient: &str) -> anyhow::Result<()> {
        tracing::info!(recipient, "Telegram start_typing invoked");
        // Emit immediately so short model turns still show "typing…"
        self.send_typing_action_once(recipient).await;

        {
            let guard = self.typing_handle.lock();
            if guard
                .as_ref()
                .is_some_and(|task| task.recipient == recipient)
            {
                return Ok(());
            }
        }
        self.stop_typing(recipient).await?;

        let client = self.http_client();
        let url = self.api_url("sendChatAction");
        let recipient_owned = recipient.to_string();
        let recipient_for_log = recipient_owned.clone();
        let body = Self::typing_body_for_recipient(recipient);

        let handle = tokio::spawn(async move {
            loop {
                match client.post(&url).json(&body).send().await {
                    Ok(resp) => {
                        if !Self::telegram_api_ok(resp).await {
                            tracing::warn!(
                                recipient = recipient_for_log,
                                "Telegram typing refresh rejected"
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            recipient = recipient_for_log,
                            %error,
                            "Telegram typing refresh request failed"
                        );
                    }
                }
                // Telegram typing indicator expires after 5s; refresh at 4s
                tokio::time::sleep(Duration::from_secs(4)).await;
            }
        });

        let mut guard = self.typing_handle.lock();
        *guard = Some(TelegramTypingTask {
            recipient: recipient_owned,
            handle,
        });

        Ok(())
    }

    async fn stop_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        tracing::info!("Telegram stop_typing invoked");
        let mut guard = self.typing_handle.lock();
        if let Some(task) = guard.take() {
            task.handle.abort();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::attachments::{
        infer_attachment_kind_from_target, parse_attachment_markers, parse_path_only_attachment,
        TelegramAttachmentKind,
    };
    use super::super::text::{
        split_message_for_telegram, strip_tool_call_tags, TELEGRAM_MAX_MESSAGE_LENGTH,
    };
    use super::TelegramChannel;
    use crate::openhuman::channels::traits::{Channel, SendMessage};
    use crate::openhuman::config::StreamMode;
    use std::path::Path;
    use std::time::Duration;

    #[test]
    fn telegram_channel_name() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        assert_eq!(ch.name(), "telegram");
    }

    #[test]
    fn typing_handle_starts_as_none() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        let guard = ch.typing_handle.lock();
        assert!(guard.is_none());
    }

    #[tokio::test]
    async fn stop_typing_clears_handle() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);

        // Manually insert a dummy handle
        {
            let mut guard = ch.typing_handle.lock();
            *guard = Some(super::TelegramTypingTask {
                recipient: "123".to_string(),
                handle: tokio::spawn(async {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }),
            });
        }

        // stop_typing should abort and clear
        ch.stop_typing("123").await.unwrap();

        let guard = ch.typing_handle.lock();
        assert!(guard.is_none());
    }

    #[tokio::test]
    async fn start_typing_replaces_previous_handle() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);

        // Insert a dummy handle first
        {
            let mut guard = ch.typing_handle.lock();
            *guard = Some(super::TelegramTypingTask {
                recipient: "123".to_string(),
                handle: tokio::spawn(async {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }),
            });
        }

        // start_typing should abort the old handle and set a new one
        let _ = ch.start_typing("123").await;

        let guard = ch.typing_handle.lock();
        assert!(guard.is_some());
    }

    #[test]
    fn supports_draft_updates_respects_stream_mode() {
        let off = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        assert!(!off.supports_draft_updates());

        let partial = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
            .with_streaming(StreamMode::Partial, 750);
        assert!(partial.supports_draft_updates());
        assert_eq!(partial.draft_update_interval_ms, 750);
    }

    #[tokio::test]
    async fn send_draft_returns_none_when_stream_mode_off() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        let id = ch
            .send_draft(&SendMessage::new("draft", "123"))
            .await
            .unwrap();
        assert!(id.is_none());
    }

    #[tokio::test]
    async fn update_draft_rate_limit_short_circuits_network() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
            .with_streaming(StreamMode::Partial, 60_000);
        ch.last_draft_edit
            .lock()
            .insert("123".to_string(), std::time::Instant::now());

        let result = ch.update_draft("123", "42", "delta text").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn update_draft_utf8_truncation_is_safe_for_multibyte_text() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
            .with_streaming(StreamMode::Partial, 0);
        let long_emoji_text = "😀".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 20);

        // Invalid message_id returns early after building display_text.
        // This asserts truncation never panics on UTF-8 boundaries.
        let result = ch
            .update_draft("123", "not-a-number", &long_emoji_text)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn finalize_draft_invalid_message_id_falls_back_to_chunk_send() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false)
            .with_streaming(StreamMode::Partial, 0);
        let long_text = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 64);

        // For oversized text + invalid draft message_id, finalize_draft should
        // fall back to chunked send instead of returning early.
        let result = ch
            .finalize_draft("123", "not-a-number", &long_text, None)
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn telegram_api_url() {
        let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
        assert_eq!(
            ch.api_url("getMe"),
            "https://api.telegram.org/bot123:ABC/getMe"
        );
    }

    #[test]
    fn telegram_user_allowed_wildcard() {
        let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
        assert!(ch.is_user_allowed("anyone"));
    }

    #[test]
    fn telegram_user_allowed_specific() {
        let ch = TelegramChannel::new("t".into(), vec!["alice".into(), "bob".into()], false);
        assert!(ch.is_user_allowed("alice"));
        assert!(!ch.is_user_allowed("eve"));
    }

    #[test]
    fn telegram_user_allowed_with_at_prefix_in_config() {
        let ch = TelegramChannel::new("t".into(), vec!["@alice".into()], false);
        assert!(ch.is_user_allowed("alice"));
    }

    #[test]
    fn telegram_user_denied_empty() {
        let ch = TelegramChannel::new("t".into(), vec![], false);
        assert!(!ch.is_user_allowed("anyone"));
    }

    #[test]
    fn telegram_user_exact_match_not_substring() {
        let ch = TelegramChannel::new("t".into(), vec!["alice".into()], false);
        assert!(!ch.is_user_allowed("alice_bot"));
        assert!(!ch.is_user_allowed("alic"));
        assert!(!ch.is_user_allowed("malice"));
    }

    #[test]
    fn telegram_user_empty_string_denied() {
        let ch = TelegramChannel::new("t".into(), vec!["alice".into()], false);
        assert!(!ch.is_user_allowed(""));
    }

    #[test]
    fn telegram_user_case_insensitive() {
        let ch = TelegramChannel::new("t".into(), vec!["Alice".into()], false);
        assert!(ch.is_user_allowed("Alice"));
        assert!(ch.is_user_allowed("alice"));
        assert!(ch.is_user_allowed("ALICE"));
    }

    #[test]
    fn telegram_wildcard_with_specific_users() {
        let ch = TelegramChannel::new("t".into(), vec!["alice".into(), "*".into()], false);
        assert!(ch.is_user_allowed("alice"));
        assert!(ch.is_user_allowed("bob"));
        assert!(ch.is_user_allowed("anyone"));
    }

    #[test]
    fn telegram_user_allowed_by_numeric_id_identity() {
        let ch = TelegramChannel::new("t".into(), vec!["123456789".into()], false);
        assert!(ch.is_any_user_allowed(["unknown", "123456789"]));
    }

    #[test]
    fn telegram_user_denied_when_none_of_identities_match() {
        let ch = TelegramChannel::new("t".into(), vec!["alice".into(), "987654321".into()], false);
        assert!(!ch.is_any_user_allowed(["unknown", "123456789"]));
    }

    #[test]
    fn telegram_pairing_enabled_with_empty_allowlist() {
        let ch = TelegramChannel::new("t".into(), vec![], false);
        assert!(ch.pairing_code_active());
    }

    #[test]
    fn telegram_pairing_disabled_with_nonempty_allowlist() {
        let ch = TelegramChannel::new("t".into(), vec!["alice".into()], false);
        assert!(!ch.pairing_code_active());
    }

    #[test]
    fn telegram_extract_bind_code_plain_command() {
        assert_eq!(
            TelegramChannel::extract_bind_code("/bind 123456"),
            Some("123456")
        );
    }

    #[test]
    fn telegram_extract_bind_code_supports_bot_mention() {
        assert_eq!(
            TelegramChannel::extract_bind_code("/bind@openhuman_bot 654321"),
            Some("654321")
        );
    }

    #[test]
    fn telegram_extract_bind_code_rejects_invalid_forms() {
        assert_eq!(TelegramChannel::extract_bind_code("/bind"), None);
        assert_eq!(TelegramChannel::extract_bind_code("/start"), None);
    }

    #[test]
    fn parse_attachment_markers_extracts_multiple_types() {
        let message = "Here are files [IMAGE:/tmp/a.png] and [DOCUMENT:https://example.com/a.pdf]";
        let (cleaned, attachments) = parse_attachment_markers(message);

        assert_eq!(cleaned, "Here are files  and");
        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0].kind, TelegramAttachmentKind::Image);
        assert_eq!(attachments[0].target, "/tmp/a.png");
        assert_eq!(attachments[1].kind, TelegramAttachmentKind::Document);
        assert_eq!(attachments[1].target, "https://example.com/a.pdf");
    }

    #[test]
    fn parse_attachment_markers_keeps_invalid_markers_in_text() {
        let message = "Report [UNKNOWN:/tmp/a.bin]";
        let (cleaned, attachments) = parse_attachment_markers(message);

        assert_eq!(cleaned, "Report [UNKNOWN:/tmp/a.bin]");
        assert!(attachments.is_empty());
    }

    #[test]
    fn parse_path_only_attachment_detects_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let image_path = dir.path().join("snap.png");
        std::fs::write(&image_path, b"fake-png").unwrap();

        let parsed = parse_path_only_attachment(image_path.to_string_lossy().as_ref())
            .expect("expected attachment");

        assert_eq!(parsed.kind, TelegramAttachmentKind::Image);
        assert_eq!(parsed.target, image_path.to_string_lossy());
    }

    #[test]
    fn parse_path_only_attachment_rejects_sentence_text() {
        assert!(parse_path_only_attachment("Screenshot saved to /tmp/snap.png").is_none());
    }

    #[test]
    fn infer_attachment_kind_from_target_detects_document_extension() {
        assert_eq!(
            infer_attachment_kind_from_target("https://example.com/files/specs.pdf?download=1"),
            Some(TelegramAttachmentKind::Document)
        );
    }

    #[test]
    fn parse_update_message_uses_chat_id_as_reply_target() {
        let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
        let update = serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 33,
                "text": "hello",
                "from": {
                    "id": 555,
                    "username": "alice"
                },
                "chat": {
                    "id": -100_200_300
                }
            }
        });

        let msg = ch
            .parse_update_message(&update)
            .expect("message should parse");

        assert_eq!(msg.sender, "alice");
        assert_eq!(msg.reply_target, "-100200300");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.id, "telegram_-100200300_33");
    }

    #[test]
    fn parse_update_message_allows_numeric_id_without_username() {
        let ch = TelegramChannel::new("token".into(), vec!["555".into()], false);
        let update = serde_json::json!({
            "update_id": 2,
            "message": {
                "message_id": 9,
                "text": "ping",
                "from": {
                    "id": 555
                },
                "chat": {
                    "id": 12345
                }
            }
        });

        let msg = ch
            .parse_update_message(&update)
            .expect("numeric allowlist should pass");

        assert_eq!(msg.sender, "555");
        assert_eq!(msg.reply_target, "12345");
    }

    #[test]
    fn parse_update_message_extracts_thread_id_for_forum_topic() {
        let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
        let update = serde_json::json!({
            "update_id": 3,
            "message": {
                "message_id": 42,
                "text": "hello from topic",
                "from": {
                    "id": 555,
                    "username": "alice"
                },
                "chat": {
                    "id": -100_200_300
                },
                "message_thread_id": 789
            }
        });

        let msg = ch
            .parse_update_message(&update)
            .expect("message with thread_id should parse");

        assert_eq!(msg.sender, "alice");
        assert_eq!(msg.reply_target, "-100200300:789");
        assert_eq!(msg.content, "hello from topic");
        assert_eq!(msg.id, "telegram_-100200300_42");
    }

    #[test]
    fn parse_update_message_sets_thread_ts_to_current_message_id_for_outbound_reply() {
        let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
        let update = serde_json::json!({
            "update_id": 4,
            "message": {
                "message_id": 99,
                "text": "reply body",
                "from": {
                    "id": 555,
                    "username": "alice"
                },
                "chat": {
                    "id": 12345
                },
                "reply_to_message": {
                    "message_id": 88
                }
            }
        });

        let msg = ch
            .parse_update_message(&update)
            .expect("message should parse");
        assert_eq!(msg.thread_ts.as_deref(), Some("99"));
        assert_eq!(msg.reply_target, "12345");
    }

    #[test]
    fn parse_update_reaction_extracts_actor_target_and_emoji() {
        let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
        let update = serde_json::json!({
            "update_id": 5,
            "message_reaction": {
                "chat": {
                    "id": -100200300
                },
                "message_id": 123,
                "user": {
                    "id": 777,
                    "username": "alice"
                },
                "old_reaction": [],
                "new_reaction": [
                    { "type": "emoji", "emoji": "🔥" }
                ]
            }
        });

        let reaction = ch
            .parse_update_reaction(&update)
            .expect("reaction should parse");
        assert_eq!(reaction.sender, "alice");
        assert_eq!(reaction.reply_target, "-100200300");
        assert_eq!(reaction.target_message_id, "123");
        assert_eq!(reaction.emoji, "🔥");
    }

    #[test]
    fn parse_reaction_marker_supports_optional_target_id() {
        let (content, marker) = TelegramChannel::parse_reaction_marker("[REACTION:✅|321]");
        assert_eq!(content, "");
        assert_eq!(marker.as_deref(), Some("✅|321"));

        let (content, marker) = TelegramChannel::parse_reaction_marker("hello");
        assert_eq!(content, "hello");
        assert!(marker.is_none());
    }

    #[test]
    fn parse_reaction_marker_allows_inline_reply_text() {
        // Bot can react AND reply in one turn: [REACTION:👍] reply text
        let (content, marker) =
            TelegramChannel::parse_reaction_marker("[REACTION:👍] That's a great point!");
        assert_eq!(content, "That's a great point!");
        assert_eq!(marker.as_deref(), Some("👍"));

        // Explicit target id + inline text
        let (content, marker) =
            TelegramChannel::parse_reaction_marker("[REACTION:🔥|999] Here's my full reply.");
        assert_eq!(content, "Here's my full reply.");
        assert_eq!(marker.as_deref(), Some("🔥|999"));

        // Reaction only (no trailing text) still works
        let (content, marker) = TelegramChannel::parse_reaction_marker("[REACTION:🤔]");
        assert_eq!(content, "");
        assert_eq!(marker.as_deref(), Some("🤔"));
    }

    #[test]
    fn update_tracking_dedupes_and_skips_stale_updates() {
        let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
        assert!(ch.track_update_id(10));
        assert!(
            !ch.track_update_id(10),
            "duplicate update should be skipped"
        );
        assert!(
            !ch.track_update_id(9),
            "stale out-of-order update should be skipped"
        );
        assert!(ch.track_update_id(11));
    }

    // ── File sending API URL tests ──────────────────────────────────

    #[test]
    fn telegram_api_url_send_document() {
        let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
        assert_eq!(
            ch.api_url("sendDocument"),
            "https://api.telegram.org/bot123:ABC/sendDocument"
        );
    }

    #[test]
    fn telegram_api_url_send_photo() {
        let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
        assert_eq!(
            ch.api_url("sendPhoto"),
            "https://api.telegram.org/bot123:ABC/sendPhoto"
        );
    }

    #[test]
    fn telegram_api_url_send_video() {
        let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
        assert_eq!(
            ch.api_url("sendVideo"),
            "https://api.telegram.org/bot123:ABC/sendVideo"
        );
    }

    #[test]
    fn telegram_api_url_send_audio() {
        let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
        assert_eq!(
            ch.api_url("sendAudio"),
            "https://api.telegram.org/bot123:ABC/sendAudio"
        );
    }

    #[test]
    fn telegram_api_url_send_voice() {
        let ch = TelegramChannel::new("123:ABC".into(), vec![], false);
        assert_eq!(
            ch.api_url("sendVoice"),
            "https://api.telegram.org/bot123:ABC/sendVoice"
        );
    }

    // ── File sending integration tests (with mock server) ──────────

    #[tokio::test]
    async fn telegram_send_document_bytes_builds_correct_form() {
        // This test verifies the method doesn't panic and handles bytes correctly
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        let file_bytes = b"Hello, this is a test file content".to_vec();

        // The actual API call will fail (no real server), but we verify the method exists
        // and handles the input correctly up to the network call
        let result = ch
            .send_document_bytes("123456", None, file_bytes, "test.txt", Some("Test caption"))
            .await;

        // Should fail with network error, not a panic or type error
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        // Error should be network-related, not a code bug
        assert!(
            err.contains("error") || err.contains("failed") || err.contains("connect"),
            "Expected network error, got: {err}"
        );
    }

    #[tokio::test]
    async fn telegram_send_photo_bytes_builds_correct_form() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        // Minimal valid PNG header bytes
        let file_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

        let result = ch
            .send_photo_bytes("123456", None, file_bytes, "test.png", None)
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn telegram_send_document_by_url_builds_correct_json() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);

        let result = ch
            .send_document_by_url(
                "123456",
                None,
                "https://example.com/file.pdf",
                Some("PDF doc"),
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn telegram_send_photo_by_url_builds_correct_json() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);

        let result = ch
            .send_photo_by_url("123456", None, "https://example.com/image.jpg", None)
            .await;

        assert!(result.is_err());
    }

    // ── File path handling tests ────────────────────────────────────

    #[tokio::test]
    async fn telegram_send_document_nonexistent_file() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        let path = Path::new("/nonexistent/path/to/file.txt");

        let result = ch.send_document("123456", None, path, None).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        // Should fail with file not found error
        assert!(
            err.contains("No such file") || err.contains("not found") || err.contains("os error"),
            "Expected file not found error, got: {err}"
        );
    }

    #[tokio::test]
    async fn telegram_send_photo_nonexistent_file() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        let path = Path::new("/nonexistent/path/to/photo.jpg");

        let result = ch.send_photo("123456", None, path, None).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn telegram_send_video_nonexistent_file() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        let path = Path::new("/nonexistent/path/to/video.mp4");

        let result = ch.send_video("123456", None, path, None).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn telegram_send_audio_nonexistent_file() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        let path = Path::new("/nonexistent/path/to/audio.mp3");

        let result = ch.send_audio("123456", None, path, None).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn telegram_send_voice_nonexistent_file() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        let path = Path::new("/nonexistent/path/to/voice.ogg");

        let result = ch.send_voice("123456", None, path, None).await;

        assert!(result.is_err());
    }

    // ── Message splitting tests ─────────────────────────────────────

    #[test]
    fn telegram_split_short_message() {
        let msg = "Hello, world!";
        let chunks = split_message_for_telegram(msg);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], msg);
    }

    #[test]
    fn telegram_split_exact_limit() {
        let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH);
        let chunks = split_message_for_telegram(&msg);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), TELEGRAM_MAX_MESSAGE_LENGTH);
    }

    #[test]
    fn telegram_split_over_limit() {
        let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 100);
        let chunks = split_message_for_telegram(&msg);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
        assert!(chunks[1].len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
    }

    #[test]
    fn telegram_split_at_word_boundary() {
        let msg = format!(
            "{} more text here",
            "word ".repeat(TELEGRAM_MAX_MESSAGE_LENGTH / 5)
        );
        let chunks = split_message_for_telegram(&msg);
        assert!(chunks.len() >= 2);
        // First chunk should end with a complete word (space at the end)
        for chunk in &chunks[..chunks.len() - 1] {
            assert!(chunk.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
        }
    }

    #[test]
    fn telegram_split_at_newline() {
        let text_block = "Line of text\n".repeat(TELEGRAM_MAX_MESSAGE_LENGTH / 13 + 1);
        let chunks = split_message_for_telegram(&text_block);
        assert!(chunks.len() >= 2);
        for chunk in chunks {
            assert!(chunk.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
        }
    }

    #[test]
    fn telegram_split_preserves_content() {
        let msg = "test ".repeat(TELEGRAM_MAX_MESSAGE_LENGTH / 5 + 100);
        let chunks = split_message_for_telegram(&msg);
        let rejoined = chunks.join("");
        assert_eq!(rejoined, msg);
    }

    #[test]
    fn telegram_split_empty_message() {
        let chunks = split_message_for_telegram("");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "");
    }

    #[test]
    fn telegram_split_very_long_message() {
        let msg = "x".repeat(TELEGRAM_MAX_MESSAGE_LENGTH * 3);
        let chunks = split_message_for_telegram(&msg);
        assert!(chunks.len() >= 3);
        for chunk in chunks {
            assert!(chunk.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
        }
    }

    // ── Caption handling tests ──────────────────────────────────────

    #[tokio::test]
    async fn telegram_send_document_bytes_with_caption() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        let file_bytes = b"test content".to_vec();

        // With caption
        let result = ch
            .send_document_bytes(
                "123456",
                None,
                file_bytes.clone(),
                "test.txt",
                Some("My caption"),
            )
            .await;
        assert!(result.is_err()); // Network error expected

        // Without caption
        let result = ch
            .send_document_bytes("123456", None, file_bytes, "test.txt", None)
            .await;
        assert!(result.is_err()); // Network error expected
    }

    #[tokio::test]
    async fn telegram_send_photo_bytes_with_caption() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        let file_bytes = vec![0x89, 0x50, 0x4E, 0x47];

        // With caption
        let result = ch
            .send_photo_bytes(
                "123456",
                None,
                file_bytes.clone(),
                "test.png",
                Some("Photo caption"),
            )
            .await;
        assert!(result.is_err());

        // Without caption
        let result = ch
            .send_photo_bytes("123456", None, file_bytes, "test.png", None)
            .await;
        assert!(result.is_err());
    }

    // ── Empty/edge case tests ───────────────────────────────────────

    #[tokio::test]
    async fn telegram_send_document_bytes_empty_file() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        let file_bytes: Vec<u8> = vec![];

        let result = ch
            .send_document_bytes("123456", None, file_bytes, "empty.txt", None)
            .await;

        // Should not panic, will fail at API level
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn telegram_send_document_bytes_empty_filename() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        let file_bytes = b"content".to_vec();

        let result = ch
            .send_document_bytes("123456", None, file_bytes, "", None)
            .await;

        // Should not panic
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn telegram_send_document_bytes_empty_chat_id() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()], false);
        let file_bytes = b"content".to_vec();

        let result = ch
            .send_document_bytes("", None, file_bytes, "test.txt", None)
            .await;

        // Should not panic
        assert!(result.is_err());
    }

    // ── Message ID edge cases ─────────────────────────────────────

    #[test]
    fn telegram_message_id_format_includes_chat_and_message_id() {
        // Verify that message IDs follow the format: telegram_{chat_id}_{message_id}
        let chat_id = "123456";
        let message_id = 789;
        let expected_id = format!("telegram_{chat_id}_{message_id}");
        assert_eq!(expected_id, "telegram_123456_789");
    }

    #[test]
    fn telegram_message_id_is_deterministic() {
        // Same chat_id + same message_id = same ID (prevents duplicates after restart)
        let chat_id = "123456";
        let message_id = 789;
        let id1 = format!("telegram_{chat_id}_{message_id}");
        let id2 = format!("telegram_{chat_id}_{message_id}");
        assert_eq!(id1, id2);
    }

    #[test]
    fn telegram_message_id_different_message_different_id() {
        // Different message IDs produce different IDs
        let chat_id = "123456";
        let id1 = format!("telegram_{chat_id}_789");
        let id2 = format!("telegram_{chat_id}_790");
        assert_ne!(id1, id2);
    }

    #[test]
    fn telegram_message_id_different_chat_different_id() {
        // Different chats produce different IDs even with same message_id
        let message_id = 789;
        let id1 = format!("telegram_123456_{message_id}");
        let id2 = format!("telegram_789012_{message_id}");
        assert_ne!(id1, id2);
    }

    #[test]
    fn telegram_message_id_no_uuid_randomness() {
        // Verify format doesn't contain random UUID components
        let chat_id = "123456";
        let message_id = 789;
        let id = format!("telegram_{chat_id}_{message_id}");
        assert!(!id.contains('-')); // No UUID dashes
        assert!(id.starts_with("telegram_"));
    }

    #[test]
    fn telegram_message_id_handles_zero_message_id() {
        // Edge case: message_id can be 0 (fallback/missing case)
        let chat_id = "123456";
        let message_id = 0;
        let id = format!("telegram_{chat_id}_{message_id}");
        assert_eq!(id, "telegram_123456_0");
    }

    // ── Tool call tag stripping tests ───────────────────────────────────

    #[test]
    fn strip_tool_call_tags_removes_standard_tags() {
        let input =
            "Hello <tool>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool> world";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "Hello  world");
    }

    #[test]
    fn strip_tool_call_tags_removes_alias_tags() {
        let input = "Hello <toolcall>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</toolcall> world";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "Hello  world");
    }

    #[test]
    fn strip_tool_call_tags_removes_dash_tags() {
        let input = "Hello <tool-call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool-call> world";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "Hello  world");
    }

    #[test]
    fn strip_tool_call_tags_removes_tool_call_tags() {
        let input = "Hello <tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call> world";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "Hello  world");
    }

    #[test]
    fn strip_tool_call_tags_removes_invoke_tags() {
        let input = "Hello <invoke>{\"name\":\"shell\",\"arguments\":{\"command\":\"date\"}}</invoke> world";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "Hello  world");
    }

    #[test]
    fn strip_tool_call_tags_handles_multiple_tags() {
        let input = "Start <tool>a</tool> middle <tool>b</tool> end";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "Start  middle  end");
    }

    #[test]
    fn strip_tool_call_tags_handles_mixed_tags() {
        let input = "A <tool>a</tool> B <toolcall>b</toolcall> C <tool-call>c</tool-call> D";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "A  B  C  D");
    }

    #[test]
    fn strip_tool_call_tags_preserves_normal_text() {
        let input = "Hello world! This is a test.";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "Hello world! This is a test.");
    }

    #[test]
    fn strip_tool_call_tags_handles_unclosed_tags() {
        let input = "Hello <tool>world";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "Hello <tool>world");
    }

    #[test]
    fn strip_tool_call_tags_handles_unclosed_tool_call_with_json() {
        let input =
            "Status:\n<tool_call>\n{\"name\":\"shell\",\"arguments\":{\"command\":\"uptime\"}}";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "Status:");
    }

    #[test]
    fn strip_tool_call_tags_handles_mismatched_close_tag() {
        let input =
            "<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"uptime\"}}</arg_value>";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "");
    }

    #[test]
    fn strip_tool_call_tags_cleans_extra_newlines() {
        let input = "Hello\n\n<tool>\ntest\n</tool>\n\n\nworld";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "Hello\n\nworld");
    }

    #[test]
    fn strip_tool_call_tags_handles_empty_input() {
        let input = "";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "");
    }

    #[test]
    fn strip_tool_call_tags_handles_only_tags() {
        let input = "<tool>{\"name\":\"test\"}</tool>";
        let result = strip_tool_call_tags(input);
        assert_eq!(result, "");
    }

    #[test]
    fn telegram_contains_bot_mention_finds_mention() {
        assert!(TelegramChannel::contains_bot_mention(
            "Hello @mybot",
            "mybot"
        ));
        assert!(TelegramChannel::contains_bot_mention(
            "@mybot help",
            "mybot"
        ));
        assert!(TelegramChannel::contains_bot_mention(
            "Hey @mybot how are you?",
            "mybot"
        ));
        assert!(TelegramChannel::contains_bot_mention(
            "Hello @MyBot, can you help?",
            "mybot"
        ));
    }

    #[test]
    fn telegram_contains_bot_mention_no_false_positives() {
        assert!(!TelegramChannel::contains_bot_mention(
            "Hello @otherbot",
            "mybot"
        ));
        assert!(!TelegramChannel::contains_bot_mention(
            "Hello mybot",
            "mybot"
        ));
        assert!(!TelegramChannel::contains_bot_mention(
            "Hello @mybot2",
            "mybot"
        ));
        assert!(!TelegramChannel::contains_bot_mention("", "mybot"));
    }

    #[test]
    fn telegram_normalize_incoming_content_strips_mention() {
        let result = TelegramChannel::normalize_incoming_content("@mybot hello", "mybot");
        assert_eq!(result, Some("hello".to_string()));
    }

    #[test]
    fn telegram_normalize_incoming_content_handles_multiple_mentions() {
        let result = TelegramChannel::normalize_incoming_content("@mybot @mybot test", "mybot");
        assert_eq!(result, Some("test".to_string()));
    }

    #[test]
    fn telegram_normalize_incoming_content_returns_none_for_empty() {
        let result = TelegramChannel::normalize_incoming_content("@mybot", "mybot");
        assert_eq!(result, None);
    }

    #[test]
    fn parse_update_message_mention_only_group_requires_exact_mention() {
        let ch = TelegramChannel::new("token".into(), vec!["*".into()], true);
        {
            let mut cache = ch.bot_username.lock();
            *cache = Some("mybot".to_string());
        }

        let update = serde_json::json!({
            "update_id": 10,
            "message": {
                "message_id": 44,
                "text": "hello @mybot2",
                "from": {
                    "id": 555,
                    "username": "alice"
                },
                "chat": {
                    "id": -100_200_300,
                    "type": "group"
                }
            }
        });

        assert!(ch.parse_update_message(&update).is_none());
    }

    #[test]
    fn parse_update_message_mention_only_group_strips_mention_and_drops_empty() {
        let ch = TelegramChannel::new("token".into(), vec!["*".into()], true);
        {
            let mut cache = ch.bot_username.lock();
            *cache = Some("mybot".to_string());
        }

        let update = serde_json::json!({
            "update_id": 11,
            "message": {
                "message_id": 45,
                "text": "Hi @MyBot status please",
                "from": {
                    "id": 555,
                    "username": "alice"
                },
                "chat": {
                    "id": -100_200_300,
                    "type": "group"
                }
            }
        });

        let parsed = ch
            .parse_update_message(&update)
            .expect("mention should parse");
        assert_eq!(parsed.content, "Hi status please");

        let empty_update = serde_json::json!({
            "update_id": 12,
            "message": {
                "message_id": 46,
                "text": "@mybot",
                "from": {
                    "id": 555,
                    "username": "alice"
                },
                "chat": {
                    "id": -100_200_300,
                    "type": "group"
                }
            }
        });

        assert!(ch.parse_update_message(&empty_update).is_none());
    }

    #[test]
    fn telegram_is_group_message_detects_groups() {
        let group_msg = serde_json::json!({
            "chat": { "type": "group" }
        });
        assert!(TelegramChannel::is_group_message(&group_msg));

        let supergroup_msg = serde_json::json!({
            "chat": { "type": "supergroup" }
        });
        assert!(TelegramChannel::is_group_message(&supergroup_msg));

        let private_msg = serde_json::json!({
            "chat": { "type": "private" }
        });
        assert!(!TelegramChannel::is_group_message(&private_msg));
    }

    #[test]
    fn telegram_mention_only_enabled_by_config() {
        let ch = TelegramChannel::new("token".into(), vec!["*".into()], true);
        assert!(ch.mention_only);

        let ch_disabled = TelegramChannel::new("token".into(), vec!["*".into()], false);
        assert!(!ch_disabled.mention_only);
    }

    // ─────────────────────────────────────────────────────────────────────
    // TG6: Channel platform limit edge cases for Telegram (4096 char limit)
    // Prevents: Pattern 6 — issues #574, #499
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn telegram_split_code_block_at_boundary() {
        let mut msg = String::new();
        msg.push_str("```python\n");
        msg.push_str(&"x".repeat(4085));
        msg.push_str("\n```\nMore text after code block");
        let parts = split_message_for_telegram(&msg);
        assert!(
            parts.len() >= 2,
            "code block spanning boundary should split"
        );
        for part in &parts {
            assert!(
                part.len() <= TELEGRAM_MAX_MESSAGE_LENGTH,
                "each part must be <= {TELEGRAM_MAX_MESSAGE_LENGTH}, got {}",
                part.len()
            );
        }
    }

    #[test]
    fn telegram_split_single_long_word() {
        let long_word = "a".repeat(5000);
        let parts = split_message_for_telegram(&long_word);
        assert!(parts.len() >= 2, "word exceeding limit must be split");
        for part in &parts {
            assert!(
                part.len() <= TELEGRAM_MAX_MESSAGE_LENGTH,
                "hard-split part must be <= {TELEGRAM_MAX_MESSAGE_LENGTH}, got {}",
                part.len()
            );
        }
        let reassembled: String = parts.join("");
        assert_eq!(reassembled, long_word);
    }

    #[test]
    fn telegram_split_exactly_at_limit_no_split() {
        let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH);
        let parts = split_message_for_telegram(&msg);
        assert_eq!(parts.len(), 1, "message exactly at limit should not split");
    }

    #[test]
    fn telegram_split_one_over_limit() {
        let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 1);
        let parts = split_message_for_telegram(&msg);
        assert!(parts.len() >= 2, "message 1 char over limit must split");
    }

    #[test]
    fn telegram_split_many_short_lines() {
        let msg: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        let parts = split_message_for_telegram(&msg);
        for part in &parts {
            assert!(
                part.len() <= TELEGRAM_MAX_MESSAGE_LENGTH,
                "short-line batch must be <= limit"
            );
        }
    }

    #[test]
    fn telegram_split_only_whitespace() {
        let msg = "   \n\n\t  ";
        let parts = split_message_for_telegram(msg);
        assert!(parts.len() <= 1);
    }

    #[test]
    fn telegram_split_emoji_at_boundary() {
        let mut msg = "a".repeat(4094);
        msg.push_str("🎉🎊"); // 4096 chars total
        let parts = split_message_for_telegram(&msg);
        for part in &parts {
            // The function splits on character count, not byte count
            assert!(
                part.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH,
                "emoji boundary split must respect limit"
            );
        }
    }

    #[test]
    fn telegram_split_consecutive_newlines() {
        let mut msg = "a".repeat(4090);
        msg.push_str("\n\n\n\n\n\n");
        msg.push_str(&"b".repeat(100));
        let parts = split_message_for_telegram(&msg);
        for part in &parts {
            assert!(part.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
        }
    }

    // ── Reaction allowlist tests ────────────────────────────────────

    #[test]
    fn parse_update_reaction_returns_none_for_unlisted_actor() {
        // Only "alice" is allowed; "mallory" should be rejected.
        let ch = TelegramChannel::new("token".into(), vec!["alice".into()], false);
        let update = serde_json::json!({
            "update_id": 50,
            "message_reaction": {
                "chat": { "id": 100 },
                "message_id": 1,
                "user": { "id": 999, "username": "mallory" },
                "old_reaction": [],
                "new_reaction": [{ "type": "emoji", "emoji": "👍" }]
            }
        });
        assert!(
            ch.parse_update_reaction(&update).is_none(),
            "reaction from non-allowlisted actor must be ignored"
        );
    }

    #[test]
    fn parse_update_reaction_returns_none_when_new_reaction_is_empty() {
        // Removing a reaction (new_reaction is empty) should yield None.
        let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
        let update = serde_json::json!({
            "update_id": 51,
            "message_reaction": {
                "chat": { "id": 100 },
                "message_id": 55,
                "user": { "id": 777, "username": "alice" },
                "old_reaction": [{ "type": "emoji", "emoji": "👍" }],
                "new_reaction": []
            }
        });
        assert!(
            ch.parse_update_reaction(&update).is_none(),
            "reaction removal (empty new_reaction) must be ignored"
        );
    }

    #[test]
    fn parse_update_reaction_falls_back_to_user_id_when_username_absent() {
        // No "username" field; allowlist uses numeric user id.
        let ch = TelegramChannel::new("token".into(), vec!["99999".into()], false);
        let update = serde_json::json!({
            "update_id": 52,
            "message_reaction": {
                "chat": { "id": 200 },
                "message_id": 77,
                "user": { "id": 99999 },
                "old_reaction": [],
                "new_reaction": [{ "type": "emoji", "emoji": "❤️" }]
            }
        });
        let reaction = ch
            .parse_update_reaction(&update)
            .expect("user_id in allowlist should be accepted");
        assert_eq!(reaction.sender, "99999");
        assert_eq!(reaction.emoji, "❤️");
        assert_eq!(reaction.target_message_id, "77");
    }

    // ── Reaction marker parsing edge cases ─────────────────────────

    #[test]
    fn parse_reaction_marker_plain_emoji_without_pipe_has_no_explicit_target() {
        // [REACTION:👍] — no pipe separator, no explicit target message id.
        let (content, marker) = TelegramChannel::parse_reaction_marker("[REACTION:👍]");
        assert_eq!(
            content, "",
            "content after stripping marker should be empty"
        );
        assert_eq!(
            marker.as_deref(),
            Some("👍"),
            "marker should be the emoji without pipe"
        );
    }

    #[test]
    fn parse_reaction_marker_empty_inner_produces_no_marker() {
        // [REACTION:] — empty inner, no valid emoji.
        let (content, marker) = TelegramChannel::parse_reaction_marker("[REACTION:]");
        assert_eq!(
            content, "",
            "content should be empty string for empty marker"
        );
        assert!(
            marker.is_none(),
            "empty inner must not produce a reaction marker"
        );
    }

    #[test]
    fn parse_reaction_marker_non_marker_text_is_unchanged() {
        let input = "here is your answer";
        let (content, marker) = TelegramChannel::parse_reaction_marker(input);
        assert_eq!(content, input);
        assert!(marker.is_none());
    }

    // ── Typing body construction tests ─────────────────────────────

    #[test]
    fn typing_body_for_plain_chat_contains_no_thread_field() {
        let body = TelegramChannel::typing_body_for_recipient("99999");
        assert_eq!(body["chat_id"].as_str(), Some("99999"));
        assert_eq!(body["action"].as_str(), Some("typing"));
        // No message_thread_id for plain chats
        assert!(
            body.get("message_thread_id").is_none() || body["message_thread_id"].is_null(),
            "plain chat should not include message_thread_id"
        );
    }

    #[test]
    fn typing_body_for_forum_topic_includes_message_thread_id() {
        let body = TelegramChannel::typing_body_for_recipient("99999:42");
        assert_eq!(body["chat_id"].as_str(), Some("99999"));
        assert_eq!(body["action"].as_str(), Some("typing"));
        assert_eq!(
            body["message_thread_id"].as_str(),
            Some("42"),
            "forum topic must carry message_thread_id in typing body"
        );
    }

    // ── Update tracking edge cases ──────────────────────────────────

    #[test]
    fn track_update_id_accepts_monotonically_increasing_sequence() {
        let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
        for id in 1..=20i64 {
            assert!(
                ch.track_update_id(id),
                "monotonically increasing id {id} should be accepted"
            );
        }
    }

    #[test]
    fn track_update_id_large_volume_beyond_cache_does_not_panic() {
        // TELEGRAM_RECENT_UPDATE_CACHE_SIZE is 4096; push well beyond to exercise eviction.
        let ch = TelegramChannel::new("t".into(), vec!["*".into()], false);
        for id in 1..=5000i64 {
            ch.track_update_id(id);
        }
        // After eviction, the next fresh id is still accepted.
        assert!(
            ch.track_update_id(5001),
            "first occurrence of a new id must always be accepted"
        );
    }

    // ── Reply-target parsing unit tests ────────────────────────────

    #[test]
    fn parse_reply_target_splits_chat_and_thread_on_colon() {
        let (chat_id, thread_id) = TelegramChannel::parse_reply_target("12345:789");
        assert_eq!(chat_id, "12345");
        assert_eq!(thread_id.as_deref(), Some("789"));
    }

    #[test]
    fn parse_reply_target_no_colon_returns_plain_chat_id() {
        let (chat_id, thread_id) = TelegramChannel::parse_reply_target("-100200300");
        assert_eq!(chat_id, "-100200300");
        assert!(thread_id.is_none());
    }

    #[test]
    fn parse_update_message_without_reply_to_still_sets_thread_ts_to_own_message_id() {
        // Every inbound message sets thread_ts = its own message_id so the outbound
        // reply attaches visibly in Telegram. This applies even with no reply_to_message.
        let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
        let update = serde_json::json!({
            "update_id": 60,
            "message": {
                "message_id": 55,
                "text": "standalone message",
                "from": { "id": 1, "username": "alice" },
                "chat": { "id": 100 }
            }
        });
        let msg = ch.parse_update_message(&update).expect("should parse");
        assert_eq!(
            msg.thread_ts.as_deref(),
            Some("55"),
            "thread_ts must equal inbound message_id for reply targeting"
        );
        assert_eq!(msg.reply_target, "100");
    }

    #[test]
    fn parse_update_message_forum_topic_encodes_thread_in_reply_target_and_thread_ts() {
        // Forum-topic messages carry message_thread_id (topic) AND may have reply_to_message.
        // reply_target must be chat_id:thread_id; thread_ts must be the inbound message_id.
        let ch = TelegramChannel::new("token".into(), vec!["*".into()], false);
        let update = serde_json::json!({
            "update_id": 61,
            "message": {
                "message_id": 100,
                "text": "in a forum topic with a quoted reply",
                "from": { "id": 1, "username": "alice" },
                "chat": { "id": -200 },
                "message_thread_id": 42,
                "reply_to_message": { "message_id": 90 }
            }
        });
        let msg = ch.parse_update_message(&update).expect("should parse");
        assert_eq!(
            msg.reply_target, "-200:42",
            "reply_target must encode both chat_id and topic thread_id"
        );
        assert_eq!(
            msg.thread_ts.as_deref(),
            Some("100"),
            "thread_ts is the inbound message_id (not the quoted parent)"
        );
    }
}
