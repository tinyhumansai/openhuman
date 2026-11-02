//! The memory block at the end of the system prompt, and when it changes.
//!
//! An agent whose definition sets `memory_block` is sent what long-term
//! memory knows about the user (beliefs, then durable facts; see
//! `memory::lifecycle::memory_block`) as the last system message of its
//! prompt, opened by [`MEMORY_BLOCK_MARKER`]. Placing it last keeps every
//! static prompt byte ahead of it where it was.
//!
//! The block is part of the frozen prefix, so changing it costs an uncached
//! request and seals the transcript generation. It is therefore rendered
//! only when a generation starts, and re-sent verbatim on every other turn
//! however memory has changed since:
//!
//! - a new thread (or a reset head): the prompt is prepared cold;
//! - a resumed thread whose recorded prefix carries no block yet;
//! - a turn whose prefix changes anyway (a replaced prompt under
//!   `refresh_prompt_on_resume`, a changed permanent-tool catalogue): the
//!   generation that turn opens gets a fresh block at no extra cost;
//! - the first turn of a generation another path opened (an in-turn or
//!   out-of-band compaction) while carrying the old block over: the head
//!   generation has a parent, has committed at most one turn, and its block
//!   is byte-identical to the parent's ([`OpenHumanTurnPrelude::memory_block_inherited`]).
//!   A head whose block differs from its parent's was opened by a block
//!   refresh, so it is kept, which is what stops a refresh from re-firing on
//!   the generation it just opened.
//!
//! A memory change alone never seals a generation: the block is left out of
//! the "did the prompt change" comparison (`prompt_refresh::prompt_messages`)
//! exactly like the permanent-tool catalogue, so it never adds
//! `PROMPT_UPDATE_NOTE` either. Nothing known renders no block (and drops a
//! stale one at a generation start); a memory read error keeps whatever the
//! thread recorded and the turn goes on.
//!
//! **Seam for code that opens a generation itself** (an out-of-band
//! compaction): call [`OpenHumanSessionHost::memory_block_for_new_generation`]
//! and put its message into the successor's prefix with
//! [`with_memory_block`]. Doing nothing is also correct: the next turn finds
//! the inherited block and renders a fresh one.
//!
//! Logs: `[memory_block] rendered|kept|empty|error` with the thread and agent
//! ids and the block's token count; never its content.

use tinyagents_runtime::{PrefixSnapshot, SessionStateView, TranscriptCodec, TurnPreparation};
use tinyinference_llm::message::Message;

use super::super::types::OpenHumanSessionHost;
use super::OpenHumanTurnPrelude;
use crate::memory::lifecycle::memory_block as render;
pub(crate) use crate::memory::lifecycle::memory_block::MEMORY_BLOCK_MARKER;

/// Whether `message` is a memory block.
pub(super) fn is_memory_block(message: &Message) -> bool {
    matches!(message, Message::System(_)) && message.text().starts_with(MEMORY_BLOCK_MARKER)
}

/// The memory block `prefix` carries, if any.
pub(super) fn recorded_block(prefix: &PrefixSnapshot) -> Option<&Message> {
    prefix
        .messages()
        .iter()
        .find(|message| is_memory_block(message))
}

/// `prefix` with `block` as its last message, in place of any block it
/// carried; without a block, `prefix` with none.
pub(crate) fn with_memory_block(prefix: &PrefixSnapshot, block: Option<Message>) -> PrefixSnapshot {
    let mut messages: Vec<Message> = prefix
        .messages()
        .iter()
        .filter(|message| !is_memory_block(message))
        .cloned()
        .collect();
    messages.extend(block);
    PrefixSnapshot::new(messages)
}

/// What one read of memory gave.
pub(super) enum RenderedBlock {
    /// A block to send.
    Block(Message),
    /// Nothing known, or memory off: no block.
    Empty,
    /// The read failed.
    Failed,
}

impl OpenHumanTurnPrelude {
    /// The block's token budget, when the agent's definition asks for one.
    pub(super) fn memory_block_budget(&self) -> Option<u32> {
        self.definition
            .as_ref()
            .and_then(|definition| definition.extensions.memory_block.as_ref())
            .map(|block| block.max_tokens)
    }

    /// Reads memory as this session's identity and renders the block.
    pub(super) async fn render_memory_block(&self, max_tokens: u32, reason: &str) -> RenderedBlock {
        let thread_id = self.thread_id.as_deref().unwrap_or("");
        let agent_id = self.agent_definition_id.as_str();
        let config = match self.memory_config().await {
            Some(config) if self.memory_readable() => config,
            _ => {
                tracing::debug!(
                    thread_id,
                    agent_id,
                    reason,
                    tokens = 0,
                    "[memory_block] empty: memory unavailable to this session"
                );
                return RenderedBlock::Empty;
            }
        };
        let identity = self.memory_identity(&config);
        match render::read(&config, &identity, max_tokens).await {
            Ok(Some(text)) => {
                tracing::info!(
                    thread_id,
                    agent_id,
                    reason,
                    tokens = tinymemory_tools::recall::estimate_tokens(&text),
                    "[memory_block] rendered"
                );
                RenderedBlock::Block(Message::system(text))
            }
            Ok(None) => {
                tracing::info!(
                    thread_id,
                    agent_id,
                    reason,
                    tokens = 0,
                    "[memory_block] empty"
                );
                RenderedBlock::Empty
            }
            Err(error) => {
                tracing::warn!(
                    thread_id,
                    agent_id,
                    reason,
                    %error,
                    "[memory_block] error: keeping the recorded block"
                );
                RenderedBlock::Failed
            }
        }
    }

    /// Puts the memory block this turn is sent into `preparation`'s prefix
    /// (see the module docs). `cold` is whether the prompt was prepared as a
    /// new session's.
    pub(super) async fn refresh_memory_block(
        &self,
        preparation: &mut TurnPreparation,
        view: &SessionStateView<'_>,
        cold: bool,
    ) {
        let Some(max_tokens) = self.memory_block_budget() else {
            return;
        };
        let recorded = recorded_block(view.prefix).cloned();
        let offered = preparation.prefix.as_ref().unwrap_or(view.prefix);
        let static_changed = !offered
            .messages()
            .iter()
            .filter(|message| !is_memory_block(message))
            .eq(view
                .prefix
                .messages()
                .iter()
                .filter(|message| !is_memory_block(message)));
        let reason = if cold {
            Some("new_generation")
        } else if recorded.is_none() {
            Some("no_block")
        } else if static_changed {
            Some("prefix_changed")
        } else if recorded
            .as_ref()
            .is_some_and(|block| self.memory_block_inherited(view, block))
        {
            Some("generation_inherited")
        } else {
            None
        };
        let block = match reason {
            Some(reason) => match self.render_memory_block(max_tokens, reason).await {
                RenderedBlock::Block(block) => Some(block),
                RenderedBlock::Empty => None,
                RenderedBlock::Failed => recorded.clone(),
            },
            None => {
                tracing::debug!(
                    thread_id = self.thread_id.as_deref().unwrap_or(""),
                    agent_id = self.agent_definition_id.as_str(),
                    tokens = recorded.as_ref().map_or(0, |block| {
                        tinymemory_tools::recall::estimate_tokens(&block.text())
                    }),
                    "[memory_block] kept"
                );
                recorded.clone()
            }
        };
        let next = with_memory_block(offered, block);
        if preparation.prefix.is_some() || next.messages() != view.prefix.messages() {
            preparation.prefix = Some(next.refreshing());
        }
    }

    /// Whether the head generation in `view` took `block` over unchanged
    /// from the generation it succeeded, and has not yet committed a turn of
    /// its own past the one that opened it. Any read failure answers `false`:
    /// the recorded block is kept.
    pub(super) fn memory_block_inherited(
        &self,
        view: &SessionStateView<'_>,
        block: &Message,
    ) -> bool {
        let Some(target) = view.transcript_target else {
            return false;
        };
        let Some(head) = target.session.as_ref() else {
            return false;
        };
        if head.generation == 0 || target.meta.turn_count > 1 {
            return false;
        }
        let mut parent = head.clone();
        parent.generation -= 1;
        let Some(read) = target.locator.read_session_transcript(&parent) else {
            return false;
        };
        let Ok(Some(transcript)) = read.read_session() else {
            return false;
        };
        let Ok(messages) = super::super::OpenHumanTranscriptCodec.decode_history(&transcript)
        else {
            return false;
        };
        let prefix_len = transcript.meta.prefix_message_count.unwrap_or_else(|| {
            messages
                .iter()
                .take_while(|message| matches!(message, Message::System(_)))
                .count()
        });
        messages
            .iter()
            .take(prefix_len)
            .any(|message| is_memory_block(message) && message == block)
    }
}

impl OpenHumanSessionHost {
    /// A freshly rendered memory block for a transcript generation this
    /// session opens outside a turn (an out-of-band compaction): put it into
    /// the successor's prefix with [`with_memory_block`]. `None` when the
    /// agent has no memory block, nothing is known, or memory cannot be read.
    pub(crate) async fn memory_block_for_new_generation(&self) -> Option<Message> {
        let prelude = self.create_turn_prelude(false);
        let max_tokens = prelude.memory_block_budget()?;
        match prelude
            .render_memory_block(max_tokens, "new_generation")
            .await
        {
            RenderedBlock::Block(block) => Some(block),
            RenderedBlock::Empty | RenderedBlock::Failed => None,
        }
    }
}

#[cfg(test)]
#[path = "memory_block_tests.rs"]
mod tests;
