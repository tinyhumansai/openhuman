//! Pure prompt/text helpers shared by the turn handler and the agent builder:
//! extracting the user prompt from a relayed `messages` array, classifying it
//! (content-free, answerable, read-back), and picking the spoken text out of
//! a streamed progress event.

use crate::agent::progress::AgentProgress;
use serde_json::Value;

/// Instruction prefix used for "speak-back": when a deferred result is ready, the
/// renderer's live voice session sends it back as a user message wrapped with this
/// prefix so the agent reads it aloud verbatim. The core recognises the prefix to
/// avoid re-arming speak-back on the read-back turn itself (which would loop). MUST
/// match the string the renderer prepends (`useRealtimeVoiceSession.ts`).
pub(super) const VOICE_READBACK_PREFIX: &str =
    "Please read the following to me, word for word, and say nothing else:";

/// Convert an OpenAI-style `messages` array into `(role, content)` history pairs
/// for [`OpenHumanSessionHost::seed_resume_from_messages`]. Drops `system` turns — the relayed
/// system prompt is the ElevenLabs agent's, not ours — and flattens multimodal
/// content the same way [`extract_prompt`] does. Pure + unit-tested.
pub(super) fn messages_to_history_pairs(messages: &[Value]) -> Vec<(String, String)> {
    messages
        .iter()
        .filter_map(|msg| {
            let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
            if role != "user" && role != "assistant" && role != "agent" {
                return None;
            }
            let text = content_to_text(msg.get("content"));
            if text.trim().is_empty() {
                return None;
            }
            Some((role.to_string(), text))
        })
        .collect()
}

/// Extract the user prompt from an OpenAI-style `messages` array: the content of
/// the last `user` message. Content may be a plain string or an array of
/// `{ type: 'text', text }` parts (multimodal shape). Pure + unit-tested.
pub fn extract_prompt(messages: &[Value]) -> String {
    for msg in messages.iter().rev() {
        if msg.get("role").and_then(Value::as_str) == Some("user") {
            return content_to_text(msg.get("content"));
        }
    }
    String::new()
}

fn content_to_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// The spoken text carried by a progress event, or `None` for events that must
/// not be voiced. Only the top-level assistant `TextDelta` is spoken; sub-agent
/// deltas, thinking, tool-call args, and lifecycle events are internal. Pure +
/// unit-tested.
pub(super) fn spoken_delta(progress: &AgentProgress) -> Option<&str> {
    match progress {
        AgentProgress::TextDelta { delta, .. } => Some(delta),
        _ => None,
    }
}

/// The answer a read-back turn is asking to have spoken, or `None` for an
/// ordinary turn. Leading whitespace is tolerated because the renderer joins the
/// prefix and payload with a blank line. Pure + unit-tested.
pub(super) fn readback_payload(prompt: &str) -> Option<&str> {
    let trimmed = prompt.trim_start();
    trimmed.strip_prefix(VOICE_READBACK_PREFIX).map(str::trim)
}

/// Whether a prompt carries nothing to answer. Speech recognition emits `"..."`
/// (and similar punctuation-only artefacts) for a pause, and the provider relays
/// those as real turns. Anything with a letter or a digit in it — in any script —
/// is a genuine prompt. Pure + unit-tested.
pub(super) fn is_content_free(prompt: &str) -> bool {
    !prompt.chars().any(char::is_alphanumeric)
}

/// Whether the user is waiting on this turn's answer, and so should be told when
/// it fails. False for a read-back (its answer is already in chat) and for a
/// recognition artefact (nothing was asked). Pure + unit-tested.
pub(super) fn is_answerable_prompt(prompt: &str) -> bool {
    !is_content_free(prompt) && should_arm_speak_back(prompt)
}

/// Whether a completed voice turn should arm speak-back — i.e. push its deferred
/// answer back into the live session to be read aloud. A read-back turn is itself
/// a verbatim-read request (its prompt is wrapped with [`VOICE_READBACK_PREFIX`]
/// by the renderer), so re-arming speak-back on it would deliver the spoken copy
/// to a turn that then asks to read it again — an unbounded loop. Suppress those.
/// Pure + unit-tested; leading whitespace is tolerated because the renderer joins
/// the prefix and payload with a blank line.
pub(super) fn should_arm_speak_back(prompt: &str) -> bool {
    !prompt.trim_start().starts_with(VOICE_READBACK_PREFIX)
}
