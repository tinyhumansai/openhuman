//! Core turn execution: the main `turn()` method and `inject_agent_experience_context()`.

use super::super::types::Agent;
use super::{
    integration_announcement_note, mcp_announcement_note, newly_connected_slugs,
    skill_announcement_note, skill_retraction_note,
};
use crate::openhuman::agent::experience::{
    prepend_experience_block, render_experience_hits, retrieve_across_stores, AgentExperienceStore,
    ExperienceQuery,
};
use crate::openhuman::agent::harness;
use crate::openhuman::agent::harness::definition::TriggerMemoryAgent;
use crate::openhuman::agent::harness::fork_context::ParentExecutionContext;
use crate::openhuman::agent::hooks::{self, TurnContext};
use crate::openhuman::agent::messages::{ChatMessage, ConversationMessage};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::memory::agent::memory_loader::collect_recall_citations;
use crate::openhuman::memory::MemoryCategory;
use crate::openhuman::util::truncate_with_ellipsis;

use anyhow::Result;
use std::hash::{Hash, Hasher};

/// Flatten the assistant tool calls a turn produced into [`ToolCallRecord`]s for
/// post-turn hooks + the deterministic cap checkpoint. Per-call success +
/// sanitized output summary are recovered from the turn's captured
/// [`ToolCallOutcome`]s (correlated by provider call id), since the harness folds
/// a tool result into a `Message::tool` that drops its failure flag — matching the
/// engine's honest per-call accounting instead of recording every call as ok.
fn tool_records_from_conversation(
    conversation: &[ConversationMessage],
    tool_outcomes: &[crate::openhuman::agent::tinyagents::ToolCallOutcome],
) -> Vec<hooks::ToolCallRecord> {
    let mut records = Vec::new();
    for msg in conversation {
        if let ConversationMessage::AssistantToolCalls { tool_calls, .. } = msg {
            for call in tool_calls {
                let outcome = tool_outcomes.iter().find(|o| o.call_id == call.id);
                // Default a MISSING outcome to `false` (#4467, item 7): a call
                // with no captured outcome is a hallucinated/unknown tool the
                // crate recovered via `ReturnToolError` without running
                // `after_tool` (so the capture sink never saw it). Recording it as
                // succeeded misreports the timeline; real executed tools always
                // have an outcome, so this only flips the genuinely-unknown case.
                let success = outcome.map(|o| o.success).unwrap_or(false);
                let output_summary = outcome
                    .map(|o| hooks::sanitize_tool_output(&o.content, &call.name, success))
                    .unwrap_or_default();
                records.push(hooks::ToolCallRecord {
                    name: call.name.clone(),
                    arguments: serde_json::from_str(&call.arguments)
                        .unwrap_or(serde_json::Value::Null),
                    success,
                    output_summary,
                    duration_ms: 0,
                });
            }
        }
    }
    records
}

/// The cap checkpoint's view of this turn's tool calls: name, status, and a
/// truncated slice of the **actual output** (issue #6014).
///
/// The sibling of [`tool_records_from_conversation`] above, and separate from
/// it on purpose. That one builds `hooks::ToolCallRecord`s, whose
/// `output_summary` is deliberately sanitized to carry no raw output — right
/// for the learning pipeline it feeds, useless for a checkpoint the user reads
/// in place of the answer the turn ran out of room to write. Reading
/// `ToolCallOutcome::content` directly here keeps the raw payload on the one
/// path that needs it instead of widening the sanitized type for everyone.
fn checkpoint_results_from_conversation(
    conversation: &[ConversationMessage],
    tool_outcomes: &[crate::openhuman::agent::tinyagents::ToolCallOutcome],
) -> Vec<super::super::turn_checkpoint::CheckpointToolResult> {
    let mut results = Vec::new();
    for msg in conversation {
        if let ConversationMessage::AssistantToolCalls { tool_calls, .. } = msg {
            for call in tool_calls {
                let outcome = tool_outcomes.iter().find(|o| o.call_id == call.id);
                // Same missing-outcome rule as `tool_records_from_conversation`:
                // a call the crate recovered without running `after_tool` never
                // reached the capture sink, so it is reported as failed rather
                // than silently as a success.
                let success = outcome.map(|o| o.success).unwrap_or(false);
                let content = outcome
                    .map(|o| {
                        super::super::turn_checkpoint::truncate_chars(
                            &o.content,
                            super::super::turn_checkpoint::CHECKPOINT_RESULT_CHARS,
                        )
                    })
                    .unwrap_or_default();
                results.push(super::super::turn_checkpoint::CheckpointToolResult {
                    name: call.name.clone(),
                    success,
                    content,
                });
            }
        }
    }
    results
}

/// Stamp each **failed** tool-result [`ChatMessage`] with its failure outcome
/// before persistence, so the derived transcript view can render an error tool
/// row instead of a false success.
///
/// The harness folds a tool result into a `role:"tool"` message whose native
/// content envelope (`{"tool_call_id":…,"content":…}`) has already dropped
/// `ToolResult::is_error`. The only structured per-call success signal is the
/// captured [`ToolCallOutcome`] side-channel; correlate by provider call id and
/// re-attach an additive failure marker (see
/// `transcript::attach_tool_failure_metadata`). Non-tool messages, tool messages
/// with no matching outcome, and successful calls are left untouched.
fn stamp_tool_failures(
    messages: &mut [ChatMessage],
    tool_outcomes: &[crate::openhuman::agent::tinyagents::ToolCallOutcome],
) {
    use crate::openhuman::agent::harness::session::transcript;
    if tool_outcomes.is_empty() {
        return;
    }
    for msg in messages.iter_mut() {
        if msg.role != "tool" {
            continue;
        }
        let Some(call_id) = parse_tool_call_id(&msg.content) else {
            continue;
        };
        let Some(outcome) = tool_outcomes.iter().find(|o| o.call_id == call_id) else {
            continue;
        };
        if outcome.success {
            continue;
        }
        let detail = short_failure_detail(&outcome.content);
        log::debug!(
            "[transcript] stamping tool failure call_id={call_id} name={}",
            outcome.name
        );
        transcript::attach_tool_failure_metadata(msg, detail.as_deref());
    }
}

/// Extract the `tool_call_id` from a native tool-result content envelope
/// (`{"tool_call_id":…,"content":…}`). `None` for non-envelope content (XML /
/// P-Format dispatchers, which don't emit `role:"tool"` messages anyway).
fn parse_tool_call_id(content: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    value.get("tool_call_id")?.as_str().map(str::to_string)
}

/// Reduce a tool's error output to a short, single-line reason for display.
fn short_failure_detail(content: &str) -> Option<String> {
    const MAX: usize = 160;
    let line = content.lines().map(str::trim).find(|l| !l.is_empty())?;
    let short: String = line.chars().take(MAX).collect();
    if short.is_empty() {
        None
    } else {
        Some(short)
    }
}

/// Rewrite the **trailing** assistant `Chat` message in `history` to `text`,
/// keeping the persisted transcript and the next turn's KV-cache prefix
/// consistent with a repaired required-output reply (issue #4117). Only the last
/// row is touched — when the tail is not an assistant `Chat` (defensive; a clean
/// finish, a cap checkpoint, and the #4093 close all end on one) a fresh
/// assistant message is appended rather than mutating an older entry.
#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;

/// Whether a history row is an assistant `Chat` with nothing in it.
///
/// The cap path's concluding call can answer with empty text, and that message
/// is folded into the history before the out-of-band wrap-up builds its request
/// from it. Anthropic rejects a message with empty content, so it has to go
/// (CodeRabbit on #6068).
pub(super) fn is_empty_assistant_chat(message: &ConversationMessage) -> bool {
    matches!(message, ConversationMessage::Chat(chat)
        if chat.role == "assistant" && chat.content.trim().is_empty())
}

fn replace_last_assistant_reply(history: &mut Vec<ConversationMessage>, text: &str) {
    match history.last_mut() {
        Some(ConversationMessage::Chat(chat)) if chat.role == "assistant" => {
            chat.content = text.to_string();
        }
        _ => history.push(ConversationMessage::Chat(ChatMessage::assistant(
            text.to_string(),
        ))),
    }
}

fn render_agent_context_status_note(sources: &[harness::AgentContextPreparedSource]) -> String {
    let sources = if sources.is_empty() {
        "the OpenHuman harness".to_string()
    } else {
        sources
            .iter()
            .map(|source| source.source.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "## Agent context status\n\nAgent context retrieval/preparation has already run once \
         for this turn in code via {sources}. Do not call `agent_prepare_context` again for \
         general context preparation. Use the prepared context below, and call only specific \
         follow-up tools if a concrete missing detail is required."
    )
}

include!("core_turn.rs");
include!("core_session.rs");
