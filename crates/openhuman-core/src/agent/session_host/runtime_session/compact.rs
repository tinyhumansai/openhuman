//! Compacting a bound conversation with no model turn.
//!
//! A host whose user is idle can compact their thread ahead of the next
//! message, so that turn resends a summary instead of the whole history. This
//! is a turn's compaction without the turn: the same compression middleware
//! folds the head generation's history (`context_wiring::compact_history`),
//! and the folded history opens the next generation the way a compacting
//! turn's commit does: the head is sealed byte-for-byte
//! (`begin_generation_from_baseline`) and the successor's first write carries
//! the head's system prompt rows and recorded tool declarations unchanged,
//! except the memory block, which a new generation renders afresh
//! (`memory_block_for_new_generation`; see `memory_block.rs`).
//!
//! It holds the session's turn lock (`lock_session_turn`, keyed by the first
//! generation) from the read to the write, so a turn and a compaction on one
//! session never interleave.

use anyhow::{anyhow, Result};
use tinyagents_runtime::{
    ResumeMode, SessionIdentityRequest, SessionIdentityResolver, TranscriptCodec,
    TranscriptTurnOptions,
};
use tinyagents_session::transcript::{lock_session_turn, SessionRef, TranscriptTurn};
use tinyinference_llm::message::Message;

use super::super::types::OpenHumanSessionHost;
use crate::agent::session_host::OpenHumanTranscriptCodec;
use crate::agent::tinyagents::context_wiring::{compact_history, CompactionSummarizer, IdleFold};
use crate::agent::tinyagents::host::OpenHumanRunContext;

/// How [`OpenHumanSessionHost::compact_bound_history`] decides and sizes a
/// compaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompactOptions {
    /// Compact only a conversation of at least this many estimated tokens
    /// (the estimate the compression middleware uses, over the head
    /// generation's messages, system prompt included).
    pub min_tokens: u64,
    /// Tokens of the most recent messages kept verbatim. `None` sizes the
    /// tail as a turn's compaction would against a `min_tokens` trigger;
    /// either way it is at most half of `min_tokens`.
    pub keep_recent_tokens: Option<u64>,
}

/// What [`OpenHumanSessionHost::compact_bound_history`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactOutcome {
    /// The head generation was sealed and the next one opened with the
    /// summary after the system prompt.
    Compacted {
        /// Estimated tokens of the conversation before.
        tokens_before: u64,
        /// Estimated tokens of the conversation the next turn resends.
        tokens_after: u64,
    },
    /// Nothing was written: the conversation is below `min_tokens`, or has
    /// nothing older than its kept tail to fold.
    Skipped {
        /// Estimated tokens of the conversation.
        tokens: u64,
    },
    /// The thread has no conversation yet.
    Empty,
}

impl OpenHumanSessionHost {
    /// Compact this session's bound conversation now, with no model turn.
    ///
    /// # Errors
    ///
    /// When the session is bound to no thread (or is a sub-agent's), when its
    /// stores cannot be admitted or read, when the summarizer models cannot be
    /// built, or when the new generation cannot be written.
    pub async fn compact_bound_history(
        &mut self,
        options: CompactOptions,
    ) -> Result<CompactOutcome> {
        let Some(thread_id) = self.thread_id.clone() else {
            return Err(anyhow!("compact_thread needs a session bound to a thread"));
        };
        if self.session_parent_prefix.is_some() {
            return Err(anyhow!(
                "compact_thread does not compact a sub-agent session"
            ));
        }
        // The turn's own identity: the runtime resolves the same request with
        // the same resolver before every turn (`identity_resolver` in ensure).
        let context = OpenHumanRunContext::new().with_core(self.dispatch_ctx().resolved());
        let request = SessionIdentityRequest {
            thread_id: Some(thread_id.clone()),
            session_key: self.runtime_session_key(),
            parent: None,
            child_key: None,
        };
        let resolved = self
            .session_identity_resolver()
            .resolve(&request, &context)
            .map_err(|error| anyhow!(error.to_string()))?;
        let session = SessionRef::scoped(request.session_key.clone(), resolved.agent_id.clone());
        let locator = resolved.stores.stores().transcripts.clone();

        tracing::debug!(thread_id = %thread_id, "[compact_thread] waiting for the session");
        let _turn = lock_session_turn(locator.as_ref(), &session).await;
        let head = locator.head_generation(&session);
        let transcript = match locator.read_session_transcript(&head) {
            Some(read) => read.read_session()?,
            None => None,
        };
        let Some(transcript) = transcript else {
            tracing::info!(thread_id = %thread_id, outcome = "empty", "[compact_thread] no conversation");
            return Ok(CompactOutcome::Empty);
        };
        let codec = OpenHumanTranscriptCodec;
        let history = codec
            .decode_history(&transcript)
            .map_err(|error| anyhow!(error.to_string()))?;
        if history
            .iter()
            .all(|message| matches!(message, Message::System(_)))
        {
            tracing::info!(thread_id = %thread_id, outcome = "empty", "[compact_thread] no conversation");
            return Ok(CompactOutcome::Empty);
        }
        let tokens = tinyagents_harness::token_estimation::estimate_slice_tokens(&history);
        if tokens < options.min_tokens {
            tracing::info!(
                thread_id = %thread_id,
                outcome = "skipped",
                tokens,
                min_tokens = options.min_tokens,
                "[compact_thread] below the threshold"
            );
            return Ok(CompactOutcome::Skipped { tokens });
        }

        let context_window = self
            .turn_model_source
            .effective_context_window(&self.model_name)
            .await;
        let models = self.turn_model_source.build(
            &self.model_name,
            self.temperature,
            context_window,
            Some(&thread_id),
        )?;
        let summarizer = CompactionSummarizer {
            model: models.summarizer.clone(),
            model_id: models.summarizer_model_id().to_owned(),
            settings: models.summarizer_settings,
        };
        let fold = IdleFold {
            min_tokens: options.min_tokens,
            keep_recent_tokens: options.keep_recent_tokens,
            strategy: self.context_config.compaction_strategy,
            context_window,
            model: &self.model_name,
        };
        let Some(compacted) = compact_history(&history, fold, summarizer)
            .await
            .map_err(|error| anyhow!(error.to_string()))?
        else {
            tracing::info!(
                thread_id = %thread_id,
                outcome = "skipped",
                tokens,
                "[compact_thread] nothing old enough to fold"
            );
            return Ok(CompactOutcome::Skipped { tokens });
        };
        let leading_system = history
            .iter()
            .take_while(|message| matches!(message, Message::System(_)))
            .count();
        let prefix_len = transcript
            .meta
            .prefix_message_count
            .unwrap_or(leading_system);
        let (compacted, prefix_len) = self.with_fresh_memory_block(compacted, prefix_len).await;
        let tokens_after = tinyagents_harness::token_estimation::estimate_slice_tokens(&compacted);

        // Unchanged positions keep their stored rows, so the system prompt
        // and the kept tail are written back exactly as they were read.
        let turn_options = TranscriptTurnOptions {
            request_id: None,
            thread_id: Some(thread_id.clone()),
            stream: false,
            resume: ResumeMode::Session,
            context,
        };
        let rows = codec
            .reconcile(&transcript.messages, &history, &compacted, &turn_options)
            .map_err(|error| anyhow!(error.to_string()))?;
        let (successor, handle) = locator.begin_generation_from_baseline(
            &head,
            transcript.meta.clone(),
            &transcript.messages,
        )?;
        // As a compacting turn's commit writes its successor (`persist`).
        let mut meta = transcript.meta.clone();
        meta.turn_count = 1;
        meta.session_id = Some(successor.session_id());
        meta.parent_session_id = successor.parent_session_id();
        meta.prefix_message_count = Some(prefix_len);
        meta.prefix_reset = false;
        meta.updated = chrono::Utc::now().to_rfc3339();
        meta.thread_id = Some(thread_id.clone());
        handle.append_turn(TranscriptTurn {
            prev: &[],
            next: &rows,
            meta: &meta,
            turn_usage: None,
            request_id: None,
            tools: transcript.tools.as_ref(),
        })?;
        tracing::info!(
            thread_id = %thread_id,
            outcome = "compacted",
            tokens_before = tokens,
            tokens_after,
            sealed = %head.session_id(),
            head = %successor.session_id(),
            "[compact_thread] sealed the head and opened the next generation"
        );
        Ok(CompactOutcome::Compacted {
            tokens_before: tokens,
            tokens_after,
        })
    }

    /// `compacted` with its prefix's memory block rendered for the new
    /// generation, and the prefix's length after it. Unchanged when the agent
    /// has no block, nothing is known, or memory cannot be read: the next
    /// turn then refreshes an inherited block itself.
    async fn with_fresh_memory_block(
        &self,
        compacted: Vec<Message>,
        prefix_len: usize,
    ) -> (Vec<Message>, usize) {
        let Some(block) = self.memory_block_for_new_generation().await else {
            return (compacted, prefix_len);
        };
        let prefix_len = prefix_len.min(
            compacted
                .iter()
                .take_while(|message| matches!(message, Message::System(_)))
                .count(),
        );
        let prefix = super::memory_block::with_memory_block(
            &tinyagents_runtime::PrefixSnapshot::new(compacted[..prefix_len].to_vec()),
            Some(block),
        );
        let refreshed_len = prefix.messages().len();
        let mut messages = prefix.messages().to_vec();
        messages.extend(compacted[prefix_len..].iter().cloned());
        tracing::debug!(
            thread_id = self.thread_id.as_deref().unwrap_or(""),
            "[compact_thread] memory block rendered for the new generation"
        );
        (messages, refreshed_len)
    }
}
