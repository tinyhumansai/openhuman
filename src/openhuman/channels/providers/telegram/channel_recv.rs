//! Telegram channel — inbound message/reaction parsing, allowlist checks, mention filtering,
//! unauthorized-message handling, and typing-action helpers.

use super::channel_types::{
    TelegramChannel, TelegramReactionEvent, TelegramVoiceAttachment, APPROVAL_PROMPT_DEBOUNCE_SECS,
    TELEGRAM_MAX_VOICE_FILE_BYTES,
};
use crate::openhuman::channels::traits::{Channel, ChannelMessage, SendMessage};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use std::time::Instant;

#[derive(Debug, Clone)]
pub(crate) struct TelegramIncomingMessageContext {
    pub(crate) sender_identity: String,
    pub(crate) reply_target: String,
    pub(crate) chat_id: String,
    pub(crate) message_id: i64,
    pub(crate) mention_text: Option<String>,
}

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

    /// Check whether an approval prompt should be suppressed due to the restart-race
    /// condition signature: `pairing.is_none()` (channel was constructed with a non-empty
    /// allowlist) AND the runtime `allowed_users` list is currently empty.
    ///
    /// This happens when the replacement process reads its config allowlist, stores it in
    /// `allowed_users`, but the old process has not yet shut down — Telegram redelivers
    /// the update to both.  The racing instance has `pairing = None` (correct — allowlist
    /// was non-empty at construction) but the runtime list may briefly show as empty before
    /// the config is loaded.
    ///
    /// Legitimate first-run pairing (`allowed_users=[]` at construction) always sets
    /// `pairing = Some(...)` so it is never suppressed here.
    pub(crate) fn is_race_condition_instance(&self) -> bool {
        let runtime_empty = self
            .allowed_users
            .read()
            .map(|users| users.is_empty())
            .unwrap_or(false);
        runtime_empty && self.pairing.is_none()
    }

    /// Build the de-bounce key for approval prompts: `"{chat_id}:{sender}"`.
    pub(crate) fn approval_debounce_key(chat_id: &str, sender: &str) -> String {
        format!("{chat_id}:{sender}")
    }

    /// Returns `true` if an approval prompt was already sent to this chat+sender within the
    /// de-bounce window, and updates the last-sent timestamp when returning `false`.
    pub(crate) fn check_and_update_approval_debounce(&self, chat_id: &str, sender: &str) -> bool {
        let key = Self::approval_debounce_key(chat_id, sender);
        let mut prompts = self.recent_approval_prompts.lock();
        if let Some(last_sent) = prompts.get(&key) {
            if last_sent.elapsed().as_secs() < APPROVAL_PROMPT_DEBOUNCE_SECS {
                return true; // still within de-bounce window
            }
        }
        // Evict entries older than the de-bounce window before inserting. Anything
        // past the window can never suppress again, so retaining it would let the
        // map grow without bound if the bot is exposed to a public group or spam
        // (review note on #1948). This caps the map to senders seen within the
        // last APPROVAL_PROMPT_DEBOUNCE_SECS.
        prompts
            .retain(|_, last_sent| last_sent.elapsed().as_secs() < APPROVAL_PROMPT_DEBOUNCE_SECS);
        prompts.insert(key, Instant::now());
        false
    }

    pub(crate) async fn handle_unauthorized_message(&self, update: &serde_json::Value) {
        let Some(message) = update.get("message") else {
            return;
        };

        if !Self::is_supported_unauthorized_message(message) {
            tracing::debug!("[telegram][approval] ignoring unsupported unauthorized update");
            return;
        }

        let text = message.get("text").and_then(serde_json::Value::as_str);

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
            tracing::warn!("[telegram][approval] missing chat_id in message, skipping");
            return;
        };

        let mut identities = vec![normalized_username.as_str()];
        if let Some(ref id) = normalized_sender_id {
            identities.push(id.as_str());
        }

        if self.is_any_user_allowed(identities.iter().copied()) {
            tracing::debug!(
                chat_id,
                username,
                sender_id = sender_id_str.as_deref().unwrap_or("unknown"),
                "[telegram][approval] message sender is allowed — no action"
            );
            return;
        }

        // ── Race-condition guard ─────────────────────────────────────────────────
        // Signature: pairing.is_none() (channel constructed with non-empty allowlist)
        // AND runtime allowed_users is currently empty.  This means we are the racing
        // instance spawned during a restart whose config hasn't propagated yet.
        // Sending an approval prompt here would spam the allowlisted user with false
        // "operator approval required" messages.  Log and suppress instead.
        if self.is_race_condition_instance() {
            tracing::warn!(
                chat_id,
                username,
                sender_id = sender_id_str.as_deref().unwrap_or("unknown"),
                "[telegram][approval] race-condition guard: allowlist is empty at runtime \
                 but channel was constructed with a non-empty allowlist (pairing=None). \
                 Suppressing approval prompt — this is a restart-overlap false positive."
            );
            return;
        }

        if let Some(code) = text.and_then(Self::extract_bind_code) {
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
                                        chat_id,
                                        identity,
                                        "[telegram][approval] paired and allowlisted identity"
                                    );
                                }
                                Err(e) => {
                                    tracing::error!(
                                        chat_id,
                                        error = %e,
                                        "[telegram][approval] failed to persist allowlist after bind"
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

        // ── De-bounce: suppress duplicate approval prompts within the window ────────
        // Key by chat_id + sender so multiple different senders are tracked independently.
        let sender_key = normalized_sender_id
            .as_deref()
            .unwrap_or(normalized_username.as_str());
        if self.check_and_update_approval_debounce(&chat_id, sender_key) {
            tracing::debug!(
                chat_id,
                sender = sender_key,
                "[telegram][approval] de-bounce: suppressing duplicate approval prompt \
                 (sent within {}s window)",
                APPROVAL_PROMPT_DEBOUNCE_SECS
            );
            return;
        }

        tracing::warn!(
            chat_id,
            username,
            sender_id = sender_id_str.as_deref().unwrap_or("unknown"),
            "[telegram][approval] unauthorized user; sending approval prompt. \
             Allowlist Telegram username (without '@') or numeric user ID."
        );

        let _ = self
            .send(&SendMessage::new(
                "🔐 This bot requires operator approval.\n\nAsk the operator to approve the pairing in the web UI, then send your message again.".to_string(),
                &chat_id,
            ))
            .await;

        if self.pairing_code_active() {
            tracing::debug!(
                chat_id,
                sender = sender_key,
                "[telegram][approval] pairing code active — sending /bind hint"
            );
            let _ = self
                .send(&SendMessage::new(
                    "ℹ️ If operator provides a one-time pairing code, you can also run `/bind <code>`.",
                    &chat_id,
                ))
                .await;
        }
    }

    pub(crate) fn is_supported_unauthorized_message(message: &serde_json::Value) -> bool {
        message
            .get("text")
            .and_then(serde_json::Value::as_str)
            .is_some()
            || message.get("voice").is_some()
    }

    pub(crate) fn parse_update_message(
        &self,
        update: &serde_json::Value,
    ) -> Option<ChannelMessage> {
        let message = update
            .get("message")
            .or_else(|| update.get("edited_message"))?;

        let text = message.get("text").and_then(serde_json::Value::as_str)?;
        let ctx = self.parse_incoming_message_context(message, Some(text))?;
        let content = match ctx.mention_text.clone() {
            Some(content) => content,
            None if self.mention_only && Self::is_group_message(message) => return None,
            None => text.to_string(),
        };

        (!content.trim().is_empty()).then(|| self.channel_message_from_context(ctx, content))
    }

    pub(crate) async fn parse_update_message_or_voice(
        &self,
        update: &serde_json::Value,
    ) -> Option<ChannelMessage> {
        let update_id = update.get("update_id").and_then(serde_json::Value::as_i64);
        let message = update
            .get("message")
            .or_else(|| update.get("edited_message"));
        let chat_id = message
            .and_then(|message| message.get("chat"))
            .and_then(|chat| chat.get("id"))
            .and_then(serde_json::Value::as_i64);
        let message_id = message
            .and_then(|message| message.get("message_id"))
            .and_then(serde_json::Value::as_i64);
        let has_text = message
            .and_then(|message| message.get("text"))
            .and_then(serde_json::Value::as_str)
            .is_some();
        let has_voice = message.and_then(|message| message.get("voice")).is_some();

        tracing::debug!(
            update_id = ?update_id,
            chat_id = ?chat_id,
            message_id = ?message_id,
            has_text,
            has_voice,
            "[telegram:voice] parse update dispatch"
        );

        if let Some(msg) = self.parse_update_message(update) {
            tracing::debug!(
                update_id = ?update_id,
                chat_id = ?chat_id,
                message_id = ?message_id,
                "[telegram:voice] selected text parser"
            );
            return Some(msg);
        }

        if has_voice {
            tracing::debug!(
                update_id = ?update_id,
                chat_id = ?chat_id,
                message_id = ?message_id,
                "[telegram:voice] selected voice parser"
            );
        } else {
            tracing::debug!(
                update_id = ?update_id,
                chat_id = ?chat_id,
                message_id = ?message_id,
                "[telegram:voice] update has no supported text or voice payload"
            );
        }

        let msg = self.parse_update_voice_message(update).await;
        tracing::debug!(
            update_id = ?update_id,
            chat_id = ?chat_id,
            message_id = ?message_id,
            parsed = msg.is_some(),
            "[telegram:voice] parse update result"
        );
        msg
    }

    pub(crate) fn parse_update_voice_attachment(
        update: &serde_json::Value,
    ) -> Option<TelegramVoiceAttachment> {
        let voice = update
            .get("message")
            .or_else(|| update.get("edited_message"))?
            .get("voice")?;

        let file_id = voice
            .get("file_id")
            .and_then(serde_json::Value::as_str)?
            .trim();
        if file_id.is_empty() {
            return None;
        }

        Some(TelegramVoiceAttachment {
            file_id: file_id.to_string(),
            file_unique_id: voice
                .get("file_unique_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            file_size: voice.get("file_size").and_then(serde_json::Value::as_u64),
            mime_type: voice
                .get("mime_type")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        })
    }

    pub(crate) async fn parse_update_voice_message(
        &self,
        update: &serde_json::Value,
    ) -> Option<ChannelMessage> {
        let update_id = update.get("update_id").and_then(serde_json::Value::as_i64);
        let message = update
            .get("message")
            .or_else(|| update.get("edited_message"))?;
        let voice = Self::parse_update_voice_attachment(update)?;
        let caption = message.get("caption").and_then(serde_json::Value::as_str);
        let chat_id = message
            .get("chat")
            .and_then(|chat| chat.get("id"))
            .and_then(serde_json::Value::as_i64);
        let message_id = message
            .get("message_id")
            .and_then(serde_json::Value::as_i64);

        tracing::debug!(
            update_id = ?update_id,
            chat_id = ?chat_id,
            message_id = ?message_id,
            file_unique_id = voice.file_unique_id.as_deref().unwrap_or("unknown"),
            has_caption = caption.is_some_and(|caption| !caption.trim().is_empty()),
            "[telegram:voice] parse voice message entry"
        );

        let ctx = self.parse_incoming_message_context(message, caption)?;
        tracing::debug!(
            update_id = ?update_id,
            chat_id = %ctx.chat_id,
            message_id = ctx.message_id,
            file_unique_id = voice.file_unique_id.as_deref().unwrap_or("unknown"),
            has_mention_text = ctx.mention_text.is_some(),
            "[telegram:voice] voice message accepted for transcription"
        );

        match self.transcribe_telegram_voice(&voice).await {
            Ok(transcript) => {
                let transcript = transcript.trim();
                if transcript.is_empty() {
                    tracing::warn!(
                        chat_id = %ctx.chat_id,
                        message_id = ctx.message_id,
                        file_unique_id = voice.file_unique_id.as_deref().unwrap_or("unknown"),
                        "[telegram:voice] inbound voice transcription returned empty text"
                    );
                    self.send_voice_transcription_failure(&ctx).await;
                    return None;
                }

                let mention_only_group = self.mention_only && Self::is_group_message(message);
                let content =
                    Self::voice_message_content(transcript, caption, &ctx, mention_only_group);

                tracing::debug!(
                    update_id = ?update_id,
                    chat_id = %ctx.chat_id,
                    message_id = ctx.message_id,
                    file_unique_id = voice.file_unique_id.as_deref().unwrap_or("unknown"),
                    content_chars = content.chars().count(),
                    "[telegram:voice] voice transcript ready for channel dispatch"
                );

                Some(self.channel_message_from_context(ctx, content))
            }
            Err(error) => {
                let error = self.redact_bot_token(error.to_string());
                tracing::warn!(
                    chat_id = %ctx.chat_id,
                    message_id = ctx.message_id,
                    file_unique_id = voice.file_unique_id.as_deref().unwrap_or("unknown"),
                    %error,
                    "[telegram:voice] inbound voice transcription failed"
                );
                self.send_voice_transcription_failure(&ctx).await;
                None
            }
        }
    }

    pub(crate) fn voice_message_content(
        transcript: &str,
        caption: Option<&str>,
        ctx: &TelegramIncomingMessageContext,
        mention_only_group: bool,
    ) -> String {
        let caption_prefix = if mention_only_group {
            ctx.mention_text.as_deref()
        } else {
            caption
        }
        .map(str::trim)
        .filter(|caption| !caption.is_empty());

        match caption_prefix {
            Some(prefix) => format!("{prefix}\n\n{transcript}"),
            None => transcript.to_string(),
        }
    }

    pub(crate) fn parse_incoming_message_context(
        &self,
        message: &serde_json::Value,
        mention_source: Option<&str>,
    ) -> Option<TelegramIncomingMessageContext> {
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
                message_len = mention_source.map(str::len).unwrap_or_default(),
                "[telegram] dropped message: sender not in allowed_users (unauthorized handler may reply)"
            );
            return None;
        }

        let is_group = Self::is_group_message(message);
        let mention_text = if self.mention_only && is_group {
            let mention_source = mention_source?;
            let bot_username = self.bot_username.lock();
            if let Some(ref bot_username) = *bot_username {
                if !Self::contains_bot_mention(mention_source, bot_username) {
                    return None;
                }
                Self::normalize_incoming_content(mention_source, bot_username)
            } else {
                return None;
            }
        } else {
            None
        };

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
        tracing::debug!(
            chat_id,
            message_id,
            reply_to_parent = replied_parent_message_id.as_deref().unwrap_or("none"),
            "Telegram inbound message parsed for reply mapping"
        );

        Some(TelegramIncomingMessageContext {
            sender_identity,
            reply_target,
            chat_id,
            message_id,
            mention_text,
        })
    }

    pub(crate) fn channel_message_from_context(
        &self,
        ctx: TelegramIncomingMessageContext,
        content: String,
    ) -> ChannelMessage {
        ChannelMessage {
            id: format!("telegram_{}_{}", ctx.chat_id, ctx.message_id),
            sender: ctx.sender_identity,
            reply_target: ctx.reply_target,
            content,
            channel: "telegram".to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            thread_ts: Some(ctx.message_id.to_string()),
        }
    }

    pub(crate) async fn send_voice_transcription_failure(
        &self,
        ctx: &TelegramIncomingMessageContext,
    ) {
        let _ = self
            .send(
                &SendMessage::new(
                    "Voice transcription failed. Please try again or send text.",
                    &ctx.reply_target,
                )
                .in_thread(Some(ctx.message_id.to_string())),
            )
            .await;
    }

    pub(crate) async fn transcribe_telegram_voice(
        &self,
        voice: &TelegramVoiceAttachment,
    ) -> anyhow::Result<String> {
        tracing::debug!(
            file_unique_id = voice.file_unique_id.as_deref().unwrap_or("unknown"),
            declared_file_size = ?voice.file_size,
            mime_type = voice.mime_type.as_deref().unwrap_or("unknown"),
            "[telegram:voice] transcribe entry"
        );

        if let Some(file_size) = voice.file_size {
            if file_size > TELEGRAM_MAX_VOICE_FILE_BYTES {
                anyhow::bail!(
                    "Telegram voice file too large: {file_size} bytes (max {TELEGRAM_MAX_VOICE_FILE_BYTES})"
                );
            }
        }

        let (audio_bytes, file_name, api_file_size) = self
            .download_telegram_voice_file(&voice.file_id, voice.file_unique_id.as_deref())
            .await?;
        tracing::debug!(
            file_unique_id = voice.file_unique_id.as_deref().unwrap_or("unknown"),
            downloaded_bytes = audio_bytes.len(),
            api_file_size = ?api_file_size,
            file_name = %file_name,
            "[telegram:voice] download completed before transcription"
        );

        if let Some(file_size) = api_file_size {
            if file_size > TELEGRAM_MAX_VOICE_FILE_BYTES {
                anyhow::bail!(
                    "Telegram getFile reported voice file too large: {file_size} bytes (max {TELEGRAM_MAX_VOICE_FILE_BYTES})"
                );
            }
        }
        if u64::try_from(audio_bytes.len()).unwrap_or(u64::MAX) > TELEGRAM_MAX_VOICE_FILE_BYTES {
            anyhow::bail!(
                "downloaded Telegram voice file too large: {} bytes (max {TELEGRAM_MAX_VOICE_FILE_BYTES})",
                audio_bytes.len()
            );
        }

        let audio_base64 = BASE64.encode(&audio_bytes);
        let config = crate::openhuman::config::rpc::load_config_with_timeout()
            .await
            .map_err(anyhow::Error::msg)?;
        let provider_name = crate::openhuman::voice::effective_stt_provider(&config);
        let model = crate::openhuman::voice::DEFAULT_WHISPER_MODEL.to_string();
        let provider =
            crate::openhuman::voice::create_stt_provider(&provider_name, &model, &config)?;
        let mime_type = voice.mime_type.as_deref().unwrap_or("audio/ogg");

        tracing::debug!(
            provider = provider.name(),
            mime_type = %mime_type,
            file_name = %file_name,
            bytes = audio_bytes.len(),
            "[telegram:voice] calling STT provider for inbound voice"
        );

        let outcome = provider
            .transcribe(
                &config,
                &audio_base64,
                Some(mime_type),
                Some(&file_name),
                None,
            )
            .await
            .map_err(anyhow::Error::msg)?;

        let text = outcome.value.text;
        tracing::debug!(
            file_unique_id = voice.file_unique_id.as_deref().unwrap_or("unknown"),
            transcript_chars = text.chars().count(),
            "[telegram:voice] transcription completed"
        );

        Ok(text)
    }

    pub(crate) fn redact_bot_token(&self, value: impl AsRef<str>) -> String {
        if self.bot_token.is_empty() {
            return value.as_ref().to_string();
        }
        value.as_ref().replace(&self.bot_token, "<redacted>")
    }

    pub(crate) async fn download_telegram_voice_file(
        &self,
        file_id: &str,
        file_unique_id: Option<&str>,
    ) -> anyhow::Result<(Vec<u8>, String, Option<u64>)> {
        tracing::debug!(
            file_unique_id = file_unique_id.unwrap_or("unknown"),
            file_id_present = !file_id.trim().is_empty(),
            "[telegram:voice:download] requesting Telegram getFile"
        );

        let resp = self
            .http_client()
            .post(self.api_url("getFile"))
            .json(&serde_json::json!({ "file_id": file_id }))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("{}", self.redact_bot_token(e.to_string())))?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::debug!(
            file_unique_id = file_unique_id.unwrap_or("unknown"),
            status = %status,
            response_bytes = body.len(),
            "[telegram:voice:download] Telegram getFile response received"
        );

        if !status.is_success() {
            tracing::debug!(
                file_unique_id = file_unique_id.unwrap_or("unknown"),
                status = %status,
                error = %self.redact_bot_token(&body),
                "[telegram:voice:download] Telegram getFile failed"
            );
            anyhow::bail!(
                "Telegram getFile failed ({status}): {}",
                self.redact_bot_token(body)
            );
        }

        let payload: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("Telegram getFile returned invalid JSON: {e}"))?;
        if !payload
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            let description = payload
                .get("description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown Telegram getFile error");
            tracing::debug!(
                file_unique_id = file_unique_id.unwrap_or("unknown"),
                description,
                "[telegram:voice:download] Telegram getFile returned ok=false"
            );
            anyhow::bail!("Telegram getFile returned ok=false: {description}");
        }

        let result = payload
            .get("result")
            .ok_or_else(|| anyhow::anyhow!("Telegram getFile response missing result"))?;
        let file_path = result
            .get("file_path")
            .and_then(serde_json::Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("Telegram getFile response missing file_path"))?;
        let file_size = result.get("file_size").and_then(serde_json::Value::as_u64);
        if let Some(file_size) = file_size {
            if file_size > TELEGRAM_MAX_VOICE_FILE_BYTES {
                tracing::debug!(
                    file_unique_id = file_unique_id.unwrap_or("unknown"),
                    file_size,
                    max_bytes = TELEGRAM_MAX_VOICE_FILE_BYTES,
                    "[telegram:voice:download] Telegram getFile file_size exceeds cap"
                );
                anyhow::bail!(
                    "Telegram getFile reported voice file too large: {file_size} bytes (max {TELEGRAM_MAX_VOICE_FILE_BYTES})"
                );
            }
        }
        let file_name = file_path
            .rsplit('/')
            .next()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("voice.ogg")
            .to_string();
        tracing::debug!(
            file_unique_id = file_unique_id.unwrap_or("unknown"),
            file_name = %file_name,
            file_size = ?file_size,
            "[telegram:voice:download] Telegram getFile result parsed"
        );

        let download_url = format!("{}/file/bot{}/{}", self.api_base, self.bot_token, file_path);
        tracing::debug!(
            file_unique_id = file_unique_id.unwrap_or("unknown"),
            file_name = %file_name,
            "[telegram:voice:download] starting Telegram voice file download"
        );
        let mut file_resp = self
            .http_client()
            .get(download_url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("{}", self.redact_bot_token(e.to_string())))?;
        if let Some(content_length) = file_resp.content_length() {
            if content_length > TELEGRAM_MAX_VOICE_FILE_BYTES {
                tracing::debug!(
                    file_unique_id = file_unique_id.unwrap_or("unknown"),
                    content_length,
                    max_bytes = TELEGRAM_MAX_VOICE_FILE_BYTES,
                    "[telegram:voice:download] voice download Content-Length exceeds cap"
                );
                anyhow::bail!(
                    "Telegram voice download too large: {content_length} bytes (max {TELEGRAM_MAX_VOICE_FILE_BYTES})"
                );
            }
        }
        tracing::debug!(
            file_unique_id = file_unique_id.unwrap_or("unknown"),
            content_length = ?file_resp.content_length(),
            "[telegram:voice:download] voice download headers received"
        );

        let status = file_resp.status();
        if !status.is_success() {
            let body = file_resp.text().await.unwrap_or_default();
            tracing::debug!(
                file_unique_id = file_unique_id.unwrap_or("unknown"),
                status = %status,
                error = %self.redact_bot_token(&body),
                "[telegram:voice:download] Telegram voice download failed"
            );
            anyhow::bail!(
                "Telegram voice download failed ({status}): {}",
                self.redact_bot_token(body)
            );
        }

        let mut bytes = Vec::new();
        while let Some(chunk) = file_resp
            .chunk()
            .await
            .map_err(|e| anyhow::anyhow!("{}", self.redact_bot_token(e.to_string())))?
        {
            Self::append_telegram_voice_download_chunk(
                &mut bytes,
                &chunk,
                TELEGRAM_MAX_VOICE_FILE_BYTES,
            )?;
            tracing::debug!(
                file_unique_id = file_unique_id.unwrap_or("unknown"),
                file_name = %file_name,
                chunk_bytes = chunk.len(),
                downloaded_bytes = bytes.len(),
                "[telegram:voice:download] received voice file chunk"
            );
        }
        tracing::debug!(
            file_unique_id = file_unique_id.unwrap_or("unknown"),
            file_name = %file_name,
            downloaded_bytes = bytes.len(),
            "[telegram:voice:download] completed Telegram voice file download"
        );

        Ok((bytes, file_name, file_size))
    }

    pub(crate) fn append_telegram_voice_download_chunk(
        bytes: &mut Vec<u8>,
        chunk: &[u8],
        max_bytes: u64,
    ) -> anyhow::Result<()> {
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| anyhow::anyhow!("Telegram voice download size overflow"))?;
        if u64::try_from(next_len).unwrap_or(u64::MAX) > max_bytes {
            anyhow::bail!("Telegram voice download too large: {next_len} bytes (max {max_bytes})");
        }

        bytes.extend_from_slice(chunk);
        Ok(())
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
