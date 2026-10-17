//! OpenHuman's lossless transcript dialect adapter.
//!
//! TinyAgents owns transcript deltas and persistence.  This module owns only
//! the conversion to OpenHuman's established wire rows; keeping it here makes
//! the runtime usable by non-OpenHuman hosts without inheriting our metadata.

use crate::agent::{message_convert, tinyagents::host::OpenHumanRunContext};
use tinyagents_runtime::{RuntimeError, TranscriptCodec, TranscriptTurnOptions};
use tinyagents_session::transcript::view::TOOL_RESULT_FAILURES_METADATA_KEY;
use tinyagents_session::transcript::{
    MessageUsage, SessionTranscript, ToolFailure, TranscriptMessage, TranscriptToolCall, TurnUsage,
    UsageCostSource,
};
use tinyinference_llm::message::Message;
use tinytools_agent::dialect::parse_replayed_results;

/// Converts OpenHuman's durable transcript rows at the TinyAgents boundary.
#[derive(Default)]
pub struct OpenHumanTranscriptCodec;

impl TranscriptCodec<OpenHumanRunContext> for OpenHumanTranscriptCodec {
    fn decode_history(&self, transcript: &SessionTranscript) -> Result<Vec<Message>, RuntimeError> {
        let rows = replayable_host_rows(&transcript.messages);
        tinyagents_runtime::transcript::native_transcript_to_messages(&rows)?;
        Ok(message_convert::history_to_messages(&rows))
    }

    fn reconcile(
        &self,
        prior: &[TranscriptMessage],
        previous: &[Message],
        next: &[Message],
        options: &TranscriptTurnOptions<OpenHumanRunContext>,
    ) -> Result<Vec<TranscriptMessage>, RuntimeError> {
        // Match against the same adopted view used by decode_history; dropped
        // legacy head results must not shift retained rows' metadata.
        let prior = replayable_host_rows(prior);
        let mut rows = next
            .iter()
            .filter_map(tinyagents_runtime::transcript::message_to_native_transcript)
            .collect::<Vec<_>>();

        // `Message` intentionally cannot represent all durable transcript
        // data.  Match each next message to one *unused* previous position and
        // retain the authoritative raw row there.  Matching by the model
        // message, rather than by role/content alone, keeps native tool-call
        // envelopes distinct and also survives a compaction replacement that
        // keeps non-prefix messages.  New messages alone receive this turn's
        // request correlation id.
        let mut consumed = vec![false; previous.len().min(prior.len())];
        let mut fresh = vec![false; rows.len()];
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
                fresh[next_index] = true;
            }
        }
        // The generic inference `Message::Tool` intentionally carries only a
        // result body and correlation id. OpenHuman's explicit per-turn
        // sidecar preserves the execution failure bit until this persistence
        // boundary, so resumed transcript rows retain the same failure status
        // the live tool timeline observed.
        let sidecar = options
            .context
            .session_sidecar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let failures = sidecar
            .tool_outcomes
            .iter()
            .filter(|outcome| !outcome.success)
            .map(|outcome| outcome.call_id.clone())
            .collect::<std::collections::HashSet<_>>();
        // Only this turn's rows: providers reuse call ids across turns
        // (`call_0`, `call_1`, ...), and re-flagging a carried-over row would
        // rewrite a persisted result, so the commit would stop extending the
        // transcript and open a new generation.
        for (row, is_fresh) in rows.iter_mut().zip(&fresh) {
            if *is_fresh
                && row.role == "tool"
                && row.id.as_ref().is_some_and(|id| failures.contains(id))
            {
                row.tool_failure = Some(ToolFailure {
                    failed: true,
                    detail: None,
                });
            }
        }
        attach_text_dialect_rounds(
            &mut rows,
            &fresh,
            &sidecar.tool_outcomes,
            sidecar.resolved_route.as_ref(),
        );
        Ok(rows)
    }

    /// The durable per-turn usage record: **this agent's own spend, excluding
    /// its children.**
    ///
    /// Every transcript in `session_raw` now means the same thing — what the
    /// agent that owns the file spent on its own provider calls. A sub-agent's
    /// transcript has always been written that way (`SubagentUsage` is
    /// "accumulated across every provider call this sub-agent made", and
    /// `subagent_host::ops::graph::transcript` stores exactly that), so a reader
    /// that walks a root plus its descendants counts every token once, at any
    /// delegation depth. Folding children in here instead made the root record
    /// overlap its own children's records, and `threads::ops::usage` added both
    /// — a double count masked only by the dead `_meta` rollup (#6460).
    ///
    /// Exclusivity also makes the record independent of whether a child's usage
    /// reached the parent's in-turn ledger at all. A detached delegation's does
    /// not (#6459), so an inclusive record was silently inclusive for a blocking
    /// spawn and exclusive for the default async one, with nothing in the file
    /// saying which. The child's own transcript is written either way.
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
        // Keep the sidecar authoritative for the post-commit UI too. The live
        // `chat_done` projection (`holistic_last_turn_usage`) still folds these
        // child entries in, because a turn's *spend* is parent + children; only
        // the durable record below stays the parent's own, for the reason in
        // this method's own doc comment.
        *options
            .context
            .session_sidecar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = sidecar.clone();
        let route = sidecar.resolved_route;
        // Preserve the old contract of omitting a synthetic all-zero usage
        // record, while ensuring every observed driver sidecar travels in the
        // same atomic runtime append as its transcript rows.
        if sidecar.input_tokens == 0
            && sidecar.output_tokens == 0
            && sidecar.cached_input_tokens == 0
            && sidecar.cost.known_usd == 0.0
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
                input: sidecar.input_tokens,
                output: sidecar.output_tokens,
                cached_input: sidecar.cached_input_tokens,
                context_window: sidecar.context_window,
                cost_usd: sidecar.cost.known_usd,
                cost_source: Some(match sidecar.cost.source {
                    tinyagents_harness::cost::CostSource::Charged => UsageCostSource::Charged,
                    tinyagents_harness::cost::CostSource::Estimated => UsageCostSource::Estimated,
                    tinyagents_harness::cost::CostSource::Unknown => UsageCostSource::Unknown,
                }),
                last_call_input: sidecar.last_call_input_tokens,
                last_call_output: sidecar.last_call_output_tokens,
            },
            ts: chrono::Utc::now().to_rfc3339(),
            reasoning_content: None,
            // Deliberately empty. `TurnUsage` lands on the turn's *final*
            // assistant row, and the transcript writer falls back to
            // `tool_calls` here for any assistant row whose own content is not
            // a native tool-call envelope — i.e. the plain-text final answer.
            // Filling it with every outcome of the turn wrote each tool call a
            // second time onto that answer, so a reader projected the answer
            // as an interim step followed by duplicate, never-settled tool
            // rows (which also mis-paired later FIFO results). Each call is
            // already recorded, once, in the envelope of the assistant row
            // that issued it.
            // A text dialect has no envelope; `attach_text_dialect_rounds` files
            // each round's calls on its issuing row instead.
            tool_calls: Vec::new(),
            iteration: sidecar.model_calls.min(u32::MAX as usize) as u32,
        }))
    }
}

/// Give each text-dialect tool round's calls to the assistant row that issued
/// them, and record which of its results failed.
///
/// A native round is persisted as a `{content, tool_calls}` envelope followed
/// by `tool` rows, so its calls and failure bits already sit on the right rows.
/// A text dialect (`xml`, `pformat`, code) persists its replay form instead: the
/// issuing assistant row holds only prose, and every result of the round is
/// folded into one `[Tool results]` user row. Neither shape can say which calls
/// were made or which of them failed, so this reads the round's call ids back
/// out of that results row and takes names, arguments and outcomes from the
/// turn sidecar:
///
/// - the issuing row (the fresh assistant row directly before the results row)
///   gets a provenance-only [`TurnUsage`] — zero spend, since the turn's spend
///   is recorded once on its final row — whose `tool_calls` are this round's;
/// - the results row gets the ids of its failed results under
///   [`TOOL_RESULT_FAILURES_METADATA_KEY`], the per-result analogue of a native
///   `tool` row's `tool_failure`.
///
/// Rows carried over from a previous turn are left untouched.
fn attach_text_dialect_rounds(
    rows: &mut [TranscriptMessage],
    fresh: &[bool],
    outcomes: &[tinyagents_harness::turn_outcome::ToolCallOutcome],
    route: Option<&tinyinference_llm::model::ResolvedModelRoute>,
) {
    let mut iteration = 0u32;
    for index in 0..rows.len() {
        if !fresh[index] {
            continue;
        }
        if rows[index].role == "assistant" {
            iteration = iteration.saturating_add(1);
            continue;
        }
        if rows[index].role != "user" {
            continue;
        }
        let Some(results) = parse_replayed_results(&rows[index].content) else {
            continue;
        };
        let outcome_for = |id: &str| outcomes.iter().find(|outcome| outcome.call_id == id);

        let failed: Vec<serde_json::Value> = results
            .iter()
            .filter(|result| outcome_for(&result.tool_call_id).is_some_and(|o| !o.success))
            .map(|result| serde_json::Value::String(result.tool_call_id.clone()))
            .collect();
        if !failed.is_empty() {
            match rows[index]
                .extra_metadata
                .get_or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
            {
                serde_json::Value::Object(map) => {
                    map.insert(
                        TOOL_RESULT_FAILURES_METADATA_KEY.to_string(),
                        serde_json::Value::Array(failed),
                    );
                }
                _ => log::warn!(
                    "[session_host][codec] text-dialect results row has non-object metadata; \
                     failure status not recorded"
                ),
            }
        }

        let Some(issuer) = index.checked_sub(1) else {
            continue;
        };
        if !fresh[issuer] || rows[issuer].role != "assistant" || rows[issuer].turn_usage.is_some() {
            continue;
        }
        let calls: Vec<TranscriptToolCall> = results
            .iter()
            .filter_map(|result| outcome_for(&result.tool_call_id))
            .map(|outcome| TranscriptToolCall {
                id: outcome.call_id.clone(),
                name: outcome.name.clone(),
                arguments: outcome.arguments.to_string(),
                extra_content: None,
            })
            .collect();
        log::debug!(
            "[session_host][codec] text-dialect round iteration={iteration} results={} \
             attached_calls={} failed={}",
            results.len(),
            calls.len(),
            rows[index]
                .extra_metadata
                .as_ref()
                .and_then(|meta| meta.get(TOOL_RESULT_FAILURES_METADATA_KEY))
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len)
        );
        if calls.is_empty() {
            continue;
        }
        rows[issuer].turn_usage = Some(TurnUsage {
            provider: route
                .map(|route| route.provider.clone())
                .unwrap_or_default(),
            model: route.map(|route| route.model.clone()).unwrap_or_default(),
            usage: MessageUsage {
                input: 0,
                output: 0,
                cached_input: 0,
                context_window: 0,
                cost_usd: 0.0,
                ..Default::default()
            },
            ts: chrono::Utc::now().to_rfc3339(),
            reasoning_content: None,
            tool_calls: calls,
            iteration,
        });
    }
}

/// Apply OpenHuman's historical orphan-head adoption to the replay view only.
fn replayable_host_rows(rows: &[TranscriptMessage]) -> std::borrow::Cow<'_, [TranscriptMessage]> {
    // Old unpaired trims could persist tool results immediately after the
    // frozen system prefix (#6721). Later orphan results still fail native
    // validation. Merely resuming never rewrites the durable source.
    let prefix = rows.iter().take_while(|row| row.role == "system").count();
    let orphan_count = rows[prefix..]
        .iter()
        .take_while(|row| row.role == "tool")
        .count();
    if orphan_count == 0 {
        std::borrow::Cow::Borrowed(rows)
    } else {
        tracing::warn!(
            orphan_count,
            "[session_host::codec] repaired legacy orphaned tool head"
        );
        std::borrow::Cow::Owned(
            rows[..prefix]
                .iter()
                .chain(&rows[prefix + orphan_count..])
                .cloned()
                .collect(),
        )
    }
}

#[cfg(test)]
#[path = "codec_adoption_tests.rs"]
mod adoption_tests;
