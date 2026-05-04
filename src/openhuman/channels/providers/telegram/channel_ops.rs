//! Telegram channel — `Channel` trait implementation: send, listen, draft streaming, typing.

use super::attachments::{parse_attachment_markers, parse_path_only_attachment};
use super::channel_types::{TelegramChannel, TelegramTypingTask};
use super::text::{strip_tool_call_tags, TELEGRAM_MAX_MESSAGE_LENGTH};
use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::channels::traits::{Channel, ChannelMessage, SendMessage};
use crate::openhuman::config::StreamMode;
use async_trait::async_trait;
use std::time::Duration;

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
