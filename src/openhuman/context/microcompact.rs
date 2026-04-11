//! Stage 3: Microcompact.
//!
//! Microcompact is the cheap summarisation substitute. It does **not**
//! generate prose summaries — instead it walks the history and replaces
//! the payload of older `ToolResults` envelopes with a short placeholder
//! string. The envelope itself is preserved so the API invariant
//! `AssistantToolCalls ⇔ ToolResults` holds and the provider still
//! accepts the next request.
//!
//! OpenHuman's inference backend does automatic prefix caching, so we
//! skip any cache-editing dance and go straight to the placeholder
//! strategy: overwrite the old bodies in place, let the backend
//! re-prefill once, and let the next turn pick up the new (smaller)
//! cache target.
//!
//! # Cache implications
//!
//! Microcompact mutates bytes that were previously sent to the backend,
//! so it **deliberately invalidates the KV-cache prefix** for this
//! session. The upside is that the new, smaller prefix becomes the next
//! stable cache target, so subsequent turns hit the cache again. This
//! stage is therefore only run when the next provider call would
//! otherwise be too large to fit — the pipeline orchestrator handles
//! gating.

use crate::openhuman::providers::ConversationMessage;

/// Placeholder used in place of cleared tool-result bodies. Must be
/// stable across versions so callers can pattern-match on it for
/// telemetry / diff tests. Keep it short — the whole point is to free
/// tokens.
pub const CLEARED_PLACEHOLDER: &str = "[Old tool result content cleared]";

/// Default number of most-recent `ToolResults` envelopes to leave
/// intact — the N most recent tool results are kept hot so the model
/// can still reason about them.
pub const DEFAULT_KEEP_RECENT_TOOL_RESULTS: usize = 5;

/// Summary of what a single microcompact pass changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MicrocompactStats {
    /// Number of `ToolResults` envelopes whose bodies were cleared.
    pub envelopes_cleared: usize,
    /// Number of individual tool-result entries within those envelopes
    /// whose `content` was replaced.
    pub entries_cleared: usize,
    /// Bytes freed from the rendered conversation (approximate — counts
    /// the `content` string length diff only).
    pub bytes_freed: usize,
}

/// Walk `history` and clear the payload of every `ToolResults` envelope
/// except the `keep_recent` most recent ones. Returns a summary of the
/// changes.
///
/// The clearing is idempotent: running the pass twice on the same
/// history is a no-op on the second call because the already-cleared
/// entries will match `CLEARED_PLACEHOLDER` and be skipped.
pub fn microcompact(history: &mut [ConversationMessage], keep_recent: usize) -> MicrocompactStats {
    // First sweep: find the indices of every `ToolResults` envelope.
    let mut tool_result_indices: Vec<usize> = history
        .iter()
        .enumerate()
        .filter_map(|(i, msg)| matches!(msg, ConversationMessage::ToolResults(_)).then_some(i))
        .collect();

    // The most-recent envelopes are at the end of the vec — peel off
    // `keep_recent` of them and leave them untouched.
    if tool_result_indices.len() <= keep_recent {
        return MicrocompactStats::default();
    }
    let cut = tool_result_indices.len().saturating_sub(keep_recent);
    tool_result_indices.truncate(cut);

    let mut stats = MicrocompactStats::default();

    for idx in tool_result_indices {
        let ConversationMessage::ToolResults(results) = &mut history[idx] else {
            continue;
        };
        let mut envelope_changed = false;
        for entry in results.iter_mut() {
            if entry.content == CLEARED_PLACEHOLDER {
                // Already cleared on a previous pass — skip.
                continue;
            }
            let old_len = entry.content.len();
            entry.content = CLEARED_PLACEHOLDER.to_string();
            let freed = old_len.saturating_sub(CLEARED_PLACEHOLDER.len());
            stats.bytes_freed += freed;
            stats.entries_cleared += 1;
            envelope_changed = true;
        }
        if envelope_changed {
            stats.envelopes_cleared += 1;
        }
    }

    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::providers::{ChatMessage, ToolCall, ToolResultMessage};

    fn user(text: &str) -> ConversationMessage {
        ConversationMessage::Chat(ChatMessage::user(text))
    }

    fn assistant_call(id: &str, name: &str) -> ConversationMessage {
        ConversationMessage::AssistantToolCalls {
            text: None,
            tool_calls: vec![ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: "{}".into(),
            }],
        }
    }

    fn tool_result(id: &str, body: &str) -> ConversationMessage {
        ConversationMessage::ToolResults(vec![ToolResultMessage {
            tool_call_id: id.into(),
            content: body.into(),
        }])
    }

    #[test]
    fn noop_when_no_tool_results() {
        let mut history = vec![user("hi"), user("again")];
        let stats = microcompact(&mut history, 5);
        assert_eq!(stats, MicrocompactStats::default());
    }

    #[test]
    fn noop_when_all_tool_results_within_keep_recent() {
        let mut history = vec![
            user("q"),
            assistant_call("a", "t"),
            tool_result("a", "body-a"),
            assistant_call("b", "t"),
            tool_result("b", "body-b"),
        ];
        let stats = microcompact(&mut history, 5);
        assert_eq!(stats, MicrocompactStats::default());
        // Bodies unchanged.
        if let ConversationMessage::ToolResults(r) = &history[2] {
            assert_eq!(r[0].content, "body-a");
        } else {
            panic!();
        }
    }

    #[test]
    fn clears_oldest_when_over_keep_recent() {
        let large_body = "x".repeat(5_000);
        let mut history = vec![
            user("q1"),
            assistant_call("t1", "fn"),
            tool_result("t1", &large_body), // oldest — should be cleared
            assistant_call("t2", "fn"),
            tool_result("t2", &large_body), // oldest — should be cleared
            assistant_call("t3", "fn"),
            tool_result("t3", "recent-1"), // keep
            assistant_call("t4", "fn"),
            tool_result("t4", "recent-2"), // keep
        ];

        let stats = microcompact(&mut history, 2);
        assert_eq!(stats.envelopes_cleared, 2);
        assert_eq!(stats.entries_cleared, 2);
        assert!(stats.bytes_freed > 9_000);

        // Oldest two have been replaced.
        match &history[2] {
            ConversationMessage::ToolResults(r) => assert_eq!(r[0].content, CLEARED_PLACEHOLDER),
            _ => panic!(),
        }
        match &history[4] {
            ConversationMessage::ToolResults(r) => assert_eq!(r[0].content, CLEARED_PLACEHOLDER),
            _ => panic!(),
        }
        // Most-recent two are preserved verbatim.
        match &history[6] {
            ConversationMessage::ToolResults(r) => assert_eq!(r[0].content, "recent-1"),
            _ => panic!(),
        }
        match &history[8] {
            ConversationMessage::ToolResults(r) => assert_eq!(r[0].content, "recent-2"),
            _ => panic!(),
        }
    }

    #[test]
    fn envelope_invariant_preserved() {
        // API requires every AssistantToolCalls to have a matching
        // ToolResults envelope. Clearing bodies must not delete the
        // envelope or remove entries from the vec inside.
        let mut history = vec![
            assistant_call("t1", "fn"),
            tool_result("t1", "old-1"),
            assistant_call("t2", "fn"),
            tool_result("t2", "new-1"),
        ];
        microcompact(&mut history, 1);

        let mut call_count = 0;
        let mut result_count = 0;
        for msg in &history {
            match msg {
                ConversationMessage::AssistantToolCalls { .. } => call_count += 1,
                ConversationMessage::ToolResults(_) => result_count += 1,
                _ => {}
            }
        }
        assert_eq!(call_count, 2);
        assert_eq!(result_count, 2);
    }

    #[test]
    fn second_pass_is_idempotent() {
        let mut history = vec![
            assistant_call("t1", "fn"),
            tool_result("t1", "old-1"),
            assistant_call("t2", "fn"),
            tool_result("t2", "new-1"),
        ];
        let first = microcompact(&mut history, 1);
        assert_eq!(first.envelopes_cleared, 1);

        let second = microcompact(&mut history, 1);
        assert_eq!(second, MicrocompactStats::default());
    }

    #[test]
    fn clears_all_entries_in_a_multi_entry_envelope() {
        let mut history = vec![
            assistant_call("t1", "fn"),
            ConversationMessage::ToolResults(vec![
                ToolResultMessage {
                    tool_call_id: "a".into(),
                    content: "A".repeat(1_000),
                },
                ToolResultMessage {
                    tool_call_id: "b".into(),
                    content: "B".repeat(1_000),
                },
            ]),
            assistant_call("t2", "fn"),
            tool_result("t2", "recent"),
        ];
        let stats = microcompact(&mut history, 1);
        assert_eq!(stats.envelopes_cleared, 1);
        assert_eq!(stats.entries_cleared, 2);

        match &history[1] {
            ConversationMessage::ToolResults(r) => {
                assert_eq!(r.len(), 2);
                assert_eq!(r[0].content, CLEARED_PLACEHOLDER);
                assert_eq!(r[1].content, CLEARED_PLACEHOLDER);
            }
            _ => panic!(),
        }
    }
}
