//! OpenHuman's lossless transcript dialect adapter.
//!
//! TinyAgents owns transcript deltas and persistence.  This module owns only
//! the conversion to OpenHuman's established wire rows; keeping it here makes
//! the runtime usable by non-OpenHuman hosts without inheriting our metadata.

use crate::agent::{
    message_convert,
    messages::{chat_message_from_transcript, transcript_message_from_chat},
    tinyagents::host::OpenHumanRunContext,
};
use tinyagents_runtime::{RuntimeError, TranscriptCodec, TranscriptTurnOptions};
use tinyagents_session::transcript::{
    MessageUsage, SessionTranscript, ToolFailure, TranscriptMessage, TranscriptToolCall, TurnUsage,
};
use tinyinference_llm::message::Message;

/// Converts OpenHuman's durable transcript rows at the TinyAgents boundary.
#[derive(Default)]
pub struct OpenHumanTranscriptCodec;

impl TranscriptCodec<OpenHumanRunContext> for OpenHumanTranscriptCodec {
    fn decode_history(&self, transcript: &SessionTranscript) -> Result<Vec<Message>, RuntimeError> {
        let rows = transcript
            .messages
            .iter()
            .cloned()
            .map(chat_message_from_transcript)
            .collect::<Vec<_>>();
        Ok(message_convert::history_to_messages(&rows))
    }

    fn reconcile(
        &self,
        prior: &[TranscriptMessage],
        previous: &[Message],
        next: &[Message],
        options: &TranscriptTurnOptions<OpenHumanRunContext>,
    ) -> Result<Vec<TranscriptMessage>, RuntimeError> {
        let mut rows = next
            .iter()
            .map(message_convert::message_to_native_chat_message)
            .map(|message| transcript_message_from_chat(&message))
            .collect::<Vec<_>>();

        // `Message` intentionally cannot represent all durable transcript
        // data.  Match each next message to one *unused* previous position and
        // retain the authoritative raw row there.  Matching by the model
        // message, rather than by role/content alone, keeps native tool-call
        // envelopes distinct and also survives a compaction replacement that
        // keeps non-prefix messages.  New messages alone receive this turn's
        // request correlation id.
        let mut consumed = vec![false; previous.len().min(prior.len())];
        for (next_index, next_message) in next.iter().enumerate() {
            let matched = previous.iter().enumerate().take(consumed.len()).find_map(
                |(previous_index, previous_message)| {
                    (!consumed[previous_index] && previous_message == next_message)
                        .then_some(previous_index)
                },
            );
            if let Some(previous_index) = matched {
                rows[next_index] = prior[previous_index].clone();
                consumed[previous_index] = true;
            } else {
                rows[next_index].request_id = options.request_id.clone();
            }
        }
        // The generic inference `Message::Tool` intentionally carries only a
        // result body and correlation id. OpenHuman's explicit per-turn
        // sidecar preserves the execution failure bit until this persistence
        // boundary, so resumed transcript rows retain the same failure status
        // the live tool timeline observed.
        let failures = options
            .context
            .session_sidecar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .tool_outcomes
            .iter()
            .filter(|outcome| !outcome.success)
            .map(|outcome| outcome.call_id.clone())
            .collect::<std::collections::HashSet<_>>();
        for row in &mut rows {
            if row.role == "tool" && row.id.as_ref().is_some_and(|id| failures.contains(id)) {
                row.tool_failure = Some(ToolFailure {
                    failed: true,
                    detail: None,
                });
            }
        }
        Ok(rows)
    }

    fn turn_usage(
        &self,
        options: &TranscriptTurnOptions<OpenHumanRunContext>,
    ) -> Result<Option<TurnUsage>, RuntimeError> {
        let subagents = options.context.subagent_usage_entries();
        let mut sidecar = options
            .context
            .session_sidecar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        // The root driver returns only after synchronous delegates have
        // completed. Snapshot their explicit ledger here, immediately before
        // the runtime's atomic append, so durable billing cannot lag the UI.
        sidecar.subagents = subagents;
        let child_input = sidecar
            .subagents
            .iter()
            .map(|entry| entry.usage.input_tokens)
            .sum::<u64>();
        let child_output = sidecar
            .subagents
            .iter()
            .map(|entry| entry.usage.output_tokens)
            .sum::<u64>();
        let child_cached_input = sidecar
            .subagents
            .iter()
            .map(|entry| entry.usage.cached_input_tokens)
            .sum::<u64>();
        let child_cost = sidecar
            .subagents
            .iter()
            .map(|entry| entry.usage.charged_amount_usd)
            .sum::<f64>();
        // Keep the sidecar authoritative for the post-commit UI too.
        *options
            .context
            .session_sidecar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = sidecar.clone();
        let route = sidecar.resolved_route;
        // Preserve the old contract of omitting a synthetic all-zero usage
        // record, while ensuring every observed driver sidecar travels in the
        // same atomic runtime append as its transcript rows.
        if sidecar.input_tokens.saturating_add(child_input) == 0
            && sidecar.output_tokens.saturating_add(child_output) == 0
            && sidecar
                .cached_input_tokens
                .saturating_add(child_cached_input)
                == 0
            && sidecar.cost_usd + child_cost == 0.0
            && route.is_none()
        {
            return Ok(None);
        }
        Ok(Some(TurnUsage {
            provider: route
                .as_ref()
                .map(|route| route.provider.clone())
                .unwrap_or_default(),
            model: route
                .as_ref()
                .map(|route| route.model.clone())
                .unwrap_or_default(),
            usage: MessageUsage {
                input: sidecar.input_tokens.saturating_add(child_input),
                output: sidecar.output_tokens.saturating_add(child_output),
                cached_input: sidecar
                    .cached_input_tokens
                    .saturating_add(child_cached_input),
                context_window: sidecar.context_window,
                cost_usd: sidecar.cost_usd + child_cost,
            },
            ts: chrono::Utc::now().to_rfc3339(),
            reasoning_content: None,
            tool_calls: sidecar
                .tool_outcomes
                .iter()
                .map(|outcome| TranscriptToolCall {
                    id: outcome.call_id.clone(),
                    name: outcome.name.clone(),
                    arguments: outcome.arguments.to_string(),
                    extra_content: None,
                })
                .collect(),
            iteration: sidecar.model_calls.min(u32::MAX as usize) as u32,
        }))
    }
}
