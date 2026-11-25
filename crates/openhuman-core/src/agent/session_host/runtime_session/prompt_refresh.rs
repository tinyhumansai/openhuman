//! The system prompt a resumed thread is sent.
//!
//! A resumed thread's prefix is frozen: the runtime restores the system
//! messages its transcript recorded and sends them verbatim, which keeps the
//! provider's prefix cache warm across a restart. A turn can still offer a
//! replacement, which the runtime applies only when it differs from what is
//! recorded; a thread-bound session then seals the transcript generation it
//! was on and carries its conversation into the next. Three things offer one:
//!
//! - the permanent-tool catalogue (`permanent.rs`), whenever it changes;
//! - the whole prompt, rendered afresh, when the agent's definition sets
//!   `refresh_prompt_on_resume`, so prompt edits reach existing threads too;
//! - the memory block (`memory_block.rs`), when a generation starts; it is
//!   left out of the comparison below like the catalogue, so a memory change
//!   alone never seals a generation or adds the note.
//!
//! Whatever is offered, the turn's transcript snapshot boundary moves with
//! it: the request starts with the offered prefix, not the recorded one.
//!
//! When the whole prompt is replaced, the user's message on that turn opens
//! with a one-time note saying so (`PROMPT_UPDATE_NOTE`). The conversation
//! before it was written under the old prompt, and a model keeps to what it
//! already said there (that something is "coming soon", say) over
//! instructions that changed since. Only a prompt rendered on that turn
//! counts: the catalogue changing under a prompt the turn kept says nothing.

use std::sync::PoisonError;

use tinyagents_definition::PromptSource;
use tinyagents_runtime::{
    PrefixSnapshot, RuntimeError, SessionStateView, SessionTurnRequest, TurnPreparation,
};
use tinyinference_llm::message::Message;

use super::memory_block::is_memory_block;
use super::permanent::PERMANENT_TOOLS_MARKER;
use super::OpenHumanTurnPrelude;
use crate::agent::message_convert::{user_message_from_text, user_text_with_markers};
use crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshotSink;

/// A prefix's messages without the permanent-tool catalogue and the memory
/// block, which the recorded prefix carries and a freshly rendered prompt
/// does not.
fn prompt_messages(prefix: &PrefixSnapshot) -> impl Iterator<Item = &Message> {
    prefix.messages().iter().filter(|message| {
        !message.text().starts_with(PERMANENT_TOOLS_MARKER) && !is_memory_block(message)
    })
}

fn prompt_refresh_log_fields(prompt_changed: bool) -> (&'static str, &'static str) {
    if prompt_changed {
        ("refreshed", "prompt_changed")
    } else {
        ("kept", "unchanged")
    }
}

/// Prepended to the user's message on the turn a resumed thread's prompt is
/// replaced; the turn's record keeps it, so later turns see where the change
/// happened.
pub(super) const PROMPT_UPDATE_NOTE: &str = "[instructions update] Your system instructions changed since the earlier messages in this conversation. Where they differ from anything said before, including what you said you could or could not do, follow your current instructions. This is an update, not a request: respond to the user's message below.";

impl OpenHumanTurnPrelude {
    /// This turn's tool snapshot and, when it changes, its prefix: what
    /// `prepare` builds, plus the refreshes a resumed thread's recorded
    /// prefix may take (see the module docs). `snapshot`'s request boundary
    /// is rebased onto the prefix offered, and `request` carries the
    /// `PROMPT_UPDATE_NOTE` when the prompt is replaced.
    pub(super) async fn prepare_turn(
        &self,
        new_session: bool,
        view: &SessionStateView<'_>,
        snapshot: &TranscriptSnapshotSink,
        request: &mut SessionTurnRequest,
    ) -> Result<TurnPreparation, RuntimeError> {
        // Imported conversations can already be durable while lacking the
        // prefix the first host turn must commit. Only an authoritative or
        // inferred frozen prefix suppresses prompt preparation.
        let cold = super::turn::prepare_prefix_for_turn(new_session, view.prefix_is_frozen);
        let mut preparation = self
            .prepare(cold)
            .await
            .map_err(|error| RuntimeError::Driver(error.to_string()))?;
        // Whether this turn rendered the current prompt. Only then can a
        // changed prefix mean the prompt was replaced: the permanent-tool
        // catalogue below also changes the prefix of a prompt that was kept.
        let rendered = if view.resumed && self.refreshes_prompt_on_resume() {
            preparation.prefix = self.current_prompt_prefix(view).await;
            preparation.prefix.is_some()
        } else {
            false
        };
        // Compared before the catalogue refresh below, which changes the
        // prefix of a kept prompt too, and without the catalogue the recorded
        // prefix carries: only the rendered prompt itself counts.
        let changed = rendered
            && preparation
                .prefix
                .as_ref()
                .is_some_and(|prefix| !prompt_messages(prefix).eq(prompt_messages(view.prefix)));
        self.refresh_permanent_prefix(&mut preparation, view.prefix);
        // Last, so the block follows everything else the prefix carries.
        self.refresh_memory_block(&mut preparation, view, cold)
            .await;
        if let Some(prefix) = preparation.prefix.as_ref() {
            // The runtime sends `prefix` in place of the recorded one, so the
            // rounds this run produces start that much further on (or back).
            let mut snapshot = snapshot.lock().unwrap_or_else(PoisonError::into_inner);
            snapshot.request_base_len = snapshot
                .request_base_len
                .saturating_sub(view.prefix.messages().len())
                + prefix.messages().len();
        }
        if rendered {
            let (prefix_in_use, reason) = prompt_refresh_log_fields(changed);
            if changed {
                tracing::info!(
                    thread_id = ?self.thread_id,
                    prefix_in_use,
                    reason,
                    recorded_segments = view.prefix.messages().len(),
                    "[session] refresh_prompt_on_resume"
                );
                let text = user_text_with_markers(&request.input);
                request.input = user_message_from_text(&format!("{PROMPT_UPDATE_NOTE}\n\n{text}"));
            } else {
                tracing::info!(
                    thread_id = ?self.thread_id,
                    prefix_in_use,
                    reason,
                    "[session] refresh_prompt_on_resume"
                );
            }
        }
        Ok(preparation)
    }

    /// Whether the agent's definition asks for a resumed thread to take up
    /// the prompt rendered now (`refresh_prompt_on_resume`).
    fn refreshes_prompt_on_resume(&self) -> bool {
        self.definition
            .as_ref()
            .is_some_and(|definition| definition.extensions.refresh_prompt_on_resume)
    }

    /// The prompt this agent renders now, offered in place of the prefix the
    /// resumed thread in `view` restored from its transcript; `None` keeps
    /// the recorded prompt.
    ///
    /// It is kept when:
    ///
    /// - that prefix is empty while the history still opens on system rows:
    ///   the extent of the recorded prompt could not be recovered (a
    ///   compacted legacy head), so its old instructions are still part of
    ///   the history, and a new prompt beside them would leave them in place;
    /// - a dynamic prompt can render connected integrations and they have not
    ///   been read this session: a prompt rendered without them could replace
    ///   a complete one;
    /// - the prompt cannot be rendered: the thread goes on under the one it
    ///   has rather than failing its turn.
    ///
    /// It is rendered as a new session's is (`cold_prompt_prefix`), so an
    /// unchanged prompt is offered byte for byte as recorded and writes
    /// nothing.
    async fn current_prompt_prefix(&self, view: &SessionStateView<'_>) -> Option<PrefixSnapshot> {
        let unknown_extent = view.prefix.messages().is_empty()
            && matches!(view.history.first(), Some(Message::System(_)));
        if unknown_extent {
            tracing::warn!(
                thread_id = ?self.thread_id,
                prefix_in_use = "kept",
                reason = "unknown_extent",
                "[session] refresh_prompt_on_resume"
            );
            return None;
        }
        let uses_live_integrations = self.definition.as_ref().is_some_and(|definition| {
            matches!(definition.system_prompt, PromptSource::Dynamic(()))
        });
        let hydrated = {
            let mutable = self.mutable.lock().unwrap_or_else(PoisonError::into_inner);
            mutable.connected_integrations_initialized
                && mutable.connected_integrations_authoritative
        };
        if uses_live_integrations && !hydrated {
            tracing::info!(
                thread_id = ?self.thread_id,
                prefix_in_use = "kept",
                reason = "integrations_not_read",
                "[session] refresh_prompt_on_resume"
            );
            return None;
        }
        match self.cold_prompt_prefix().await {
            Ok(prefix) => Some(prefix.refreshing()),
            Err(error) => {
                tracing::warn!(
                    thread_id = ?self.thread_id,
                    prefix_in_use = "kept",
                    reason = "render_failed",
                    %error,
                    "[session] refresh_prompt_on_resume"
                );
                None
            }
        }
    }
}

#[cfg(test)]
#[path = "prompt_refresh_tests.rs"]
mod tests;
