//! The compaction hook: what memory recalls about the turns a compaction
//! folds away rides into the checkpoint that replaces them.
//!
//! [`MemoryRecallSummarizer`] wraps the turn's summarizer. While the inner
//! summarizer folds the dropped messages, it queues memory's refresh of that
//! span and takes only completed cached context. A non-empty cached pack is
//! appended under "Recalled from memory"; engine I/O never delays the inner
//! summarizer. The
//! checkpoint is a fresh user-role message after a compaction, which already
//! resets the provider's cache from that point, so it costs no warm prefix.
//!
//! The inner summarizer's failure is the summarizer's failure; memory's never
//! is.

use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::error::Result;
use tinyagents_harness::summarization::{is_checkpoint, Summarizer, SummaryRecord, SummaryRequest};
use tinyinference_llm::message::{ContentBlock, Message};
use tinymemory_api::{Role, Turn};

use crate::memory::lifecycle::hooks::MemoryTurn;

/// Heading of the recalled section in a checkpoint.
pub(crate) const RECALLED_HEADING: &str = "## Recalled from memory";

/// A summarizer that adds memory's recall of the folded span.
pub(crate) struct MemoryRecallSummarizer {
    inner: Box<dyn Summarizer>,
    turn: Arc<MemoryTurn>,
}

impl MemoryRecallSummarizer {
    /// Wraps `inner` for `turn`; without a turn binding the summarizer is
    /// returned as is. With recall off the wrapper adds no recall, but still
    /// moves the turn's memory window past what the fold took: the agent's
    /// `memory` tool reads by it either way.
    pub(crate) fn wrap(
        inner: Box<dyn Summarizer>,
        turn: Option<Arc<MemoryTurn>>,
    ) -> Box<dyn Summarizer> {
        match turn {
            Some(turn) => Box::new(Self { inner, turn }),
            None => inner,
        }
    }

    async fn with_recall(
        &self,
        messages: &[Message],
        summary: impl std::future::Future<Output = Result<SummaryRecord>>,
    ) -> Result<SummaryRecord> {
        let pack = if self.turn.identity.recall {
            crate::memory::lifecycle::prefetch::compaction(
                &self.turn.dispatch_ctx(),
                self.turn.config.clone(),
                self.turn.identity.clone(),
                self.turn.thread_id.clone(),
                dropped_turns(messages),
            )
        } else {
            None
        };
        let mut record = summary.await?;
        // The folded user turns are out of the prompt now, so the turn's next
        // memory read may return them.
        self.turn.in_prompt_from.advance_past(
            messages
                .iter()
                .filter(|message| matches!(message, Message::User(_)) && !is_checkpoint(message))
                .map(Message::text),
        );
        if !self.turn.identity.recall {
            return Ok(record);
        }
        // A later compaction folds the previous checkpoint, recall included;
        // keep one recall section, the current one.
        strip_recalled(&mut record.summary);
        if let Some(pack) = pack {
            append(
                &mut record.summary,
                &format!("\n\n{RECALLED_HEADING}\n\n{}", pack.markdown.trim()),
            );
            tracing::debug!(
                thread_id = %self.turn.thread_id,
                tokens = pack.tokens,
                "[context_compression] memory recall added to the checkpoint"
            );
        }
        Ok(record)
    }
}

#[async_trait]
impl Summarizer for MemoryRecallSummarizer {
    async fn summarize(&self, messages: &[Message]) -> Result<SummaryRecord> {
        self.with_recall(messages, self.inner.summarize(messages))
            .await
    }

    async fn summarize_request(&self, request: &SummaryRequest) -> Result<SummaryRecord> {
        self.with_recall(&request.messages, self.inner.summarize_request(request))
            .await
    }

    async fn merge(&self, summaries: &[SummaryRecord]) -> Result<SummaryRecord> {
        self.inner.merge(summaries).await
    }
}

/// The user and assistant text of `messages`, as memory turns. Tool results
/// and system text are the summarizer's to fold, not memory's to query by.
pub(crate) fn dropped_turns(messages: &[Message]) -> Vec<Turn> {
    messages
        .iter()
        .filter_map(|message| {
            let role = match message {
                Message::User(_) => Role::User,
                Message::Assistant(_) => Role::Assistant,
                _ => return None,
            };
            let text = message.text();
            (!text.trim().is_empty()).then(|| Turn::new(role, text))
        })
        .collect()
}

/// Drops a "Recalled from memory" section a summary carried over from an
/// earlier checkpoint: the heading and everything after it.
fn strip_recalled(summary: &mut Message) {
    let blocks = match summary {
        Message::User(message) => &mut message.content,
        Message::System(message) => &mut message.content,
        Message::Assistant(message) => &mut message.content,
        Message::Tool(message) => &mut message.content,
        Message::Custom(_) => return,
    };
    let Some(at) = blocks.iter().position(
        |block| matches!(block, ContentBlock::Text(text) if text.contains(RECALLED_HEADING)),
    ) else {
        return;
    };
    blocks.truncate(at + 1);
    if let Some(ContentBlock::Text(text)) = blocks.last_mut() {
        if let Some(cut) = text.find(RECALLED_HEADING) {
            text.truncate(cut);
            let kept = text.trim_end().len();
            text.truncate(kept);
        }
    }
    if matches!(blocks.last(), Some(ContentBlock::Text(text)) if text.is_empty()) {
        blocks.pop();
    }
}

/// Appends `text` to the summary message, whatever its role.
fn append(summary: &mut Message, text: &str) {
    let block = ContentBlock::Text(text.to_string());
    match summary {
        Message::User(message) => message.content.push(block),
        Message::System(message) => message.content.push(block),
        Message::Assistant(message) => message.content.push(block),
        Message::Tool(message) => message.content.push(block),
        Message::Custom(_) => {}
    }
}

#[cfg(test)]
#[path = "memory_summarizer_tests.rs"]
mod tests;
