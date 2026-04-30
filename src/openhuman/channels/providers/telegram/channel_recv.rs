//! Telegram channel — inbound message/reaction parsing, allowlist checks, mention filtering,
//! unauthorized-message handling, and typing-action helpers.

use super::channel_types::{TelegramChannel, TelegramReactionEvent};
use crate::openhuman::channels::traits::{Channel, ChannelMessage, SendMessage};

impl TelegramChannel {
    pub(crate) fn typing_body_for_recipient(recipient: &str) -> serde_json::Value {
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

    pub(crate) async fn send_typing_action_once(&self, recipient: &str) {
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

    pub(crate) fn is_telegram_username_char(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || ch == '_'
    }

    pub(crate) fn find_bot_mention_spans(text: &str, bot_username: &str) -> Vec<(usize, usize)> {
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

    pub(crate) fn contains_bot_mention(text: &str, bot_username: &str) -> bool {
        !Self::find_bot_mention_spans(text, bot_username).is_empty()
    }

    pub(crate) fn normalize_incoming_content(text: &str, bot_username: &str) -> Option<String> {
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

    pub(crate) fn is_group_message(message: &serde_json::Value) -> bool {
        message
            .get("chat")
            .and_then(|c| c.get("type"))
            .and_then(|t| t.as_str())
            .map(|t| t == "group" || t == "supergroup")
            .unwrap_or(false)
    }

    pub(crate) fn is_user_allowed(&self, username: &str) -> bool {
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

    pub(crate) fn is_any_user_allowed<'a, I>(&self, identities: I) -> bool
    where
        I: IntoIterator<Item = &'a str>,
    {
        identities.into_iter().any(|id| self.is_user_allowed(id))
    }

    pub(crate) async fn handle_unauthorized_message(&self, update: &serde_json::Value) {
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

    pub(crate) fn parse_update_message(
        &self,
        update: &serde_json::Value,
    ) -> Option<ChannelMessage> {
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

    pub(crate) fn parse_update_reaction(
        &self,
        update: &serde_json::Value,
    ) -> Option<TelegramReactionEvent> {
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
}
