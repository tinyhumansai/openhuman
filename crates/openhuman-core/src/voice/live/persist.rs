//! Saves a live session's final transcripts into the open chat thread.
//!
//! Each final user or agent utterance becomes one thread message with a
//! deterministic id (`voice-<session>-<n>-<role>`), so a retried write is a
//! no-op and the UI can refresh the thread to show it. Partial transcripts and
//! tool traffic are not stored.

use crate::core::runtime::DispatchCtx;
use std::path::PathBuf;

use serde_json::json;

use super::types::TranscriptRole;
use crate::threads::store::ConversationMessage;

/// Writes final transcripts for one session.
#[derive(Debug)]
pub(crate) struct TranscriptPersister {
    ctx: DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: String,
    session_id: String,
    provider: String,
    sequence: u64,
}

impl TranscriptPersister {
    pub(crate) fn new(
        ctx: DispatchCtx,
        workspace_dir: PathBuf,
        thread_id: String,
        session_id: String,
        provider: String,
    ) -> Self {
        Self {
            ctx,
            workspace_dir,
            thread_id,
            session_id,
            provider,
            sequence: 0,
        }
    }

    /// The message a final utterance becomes, or `None` for blank text.
    pub(crate) fn message(
        &mut self,
        role: TranscriptRole,
        text: &str,
    ) -> Option<ConversationMessage> {
        let content = text.trim();
        if content.is_empty() {
            return None;
        }
        self.sequence += 1;
        let (sender, suffix) = match role {
            TranscriptRole::User => ("user", "user"),
            TranscriptRole::Agent => ("agent", "agent"),
        };
        Some(ConversationMessage {
            id: format!("voice-{}-{}-{suffix}", self.session_id, self.sequence),
            content: content.to_string(),
            message_type: "text".to_string(),
            extra_metadata: json!({
                "source": "voice",
                "provider": self.provider,
                "voiceSessionId": self.session_id,
            }),
            sender: sender.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    /// Appends a final utterance to the thread. Failures are logged, never
    /// fatal: losing a transcript row must not end a live conversation.
    pub(crate) async fn save(&mut self, role: TranscriptRole, text: &str) {
        let Some(message) = self.message(role, text) else {
            return;
        };
        let id = message.id.clone();
        match crate::threads::store::blocking::append_message(
            self.ctx.clone(),
            self.workspace_dir.clone(),
            self.thread_id.clone(),
            message,
        )
        .await
        {
            Ok(_) => tracing::debug!(message_id = %id, "[voice-live] transcript saved to thread"),
            Err(error) => tracing::warn!(
                message_id = %id,
                "[voice-live] could not save transcript to thread: {error}"
            ),
        }
    }
}

#[cfg(test)]
#[path = "persist_tests.rs"]
mod tests;
