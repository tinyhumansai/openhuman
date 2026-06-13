//! Pre-dispatch token budgeting for agent conversation history.
//!
//! Estimates prompt size with the same ~4 chars/token heuristic used elsewhere
//! in the codebase and drops the oldest non-system messages until the payload
//! fits the target model's context window.

use crate::openhuman::inference::provider::{ChatMessage, ConversationMessage};

/// Tokens reserved for the model's completion, tool schemas, and provider overhead.
pub const DEFAULT_OUTPUT_RESERVE_TOKENS: u64 = 8_192;

/// Minimum reserve when the context window is very small.
const MIN_OUTPUT_RESERVE_TOKENS: u64 = 512;

/// Outcome of a pre-dispatch trim pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenBudgetOutcome {
    pub original_tokens: usize,
    pub final_tokens: usize,
    pub messages_removed: usize,
    pub trimmed: bool,
}

/// `[IMAGE:<data-uri>]` marker prefix. Mirrors
/// [`crate::openhuman::agent::multimodal`]: the harness embeds image
/// attachments as these markers inside the message text until the provider
/// layer promotes them into structured `image_url` parts. Kept local to avoid
/// a dependency on the (private) multimodal constant.
const IMAGE_MARKER_PREFIX: &str = "[IMAGE:";

/// Token allowance charged per `[IMAGE:…]` marker instead of counting its
/// base64 `data:` URI as text.
///
/// **Why this exists (#3205):** an attachment rides as a `[IMAGE:<base64>]`
/// marker in the message string, so an 8 MiB image is ~11 M characters. At the
/// `len/4` heuristic that reads as ~2.7 M "tokens" — orders of magnitude past
/// any context window — so the pre-dispatch budget trimmer evicted the whole
/// message *before* the image was ever extracted into `image_url` parts, and
/// the model received a text-only turn (empty/garbage response). Vision models
/// bill an image at a small fixed cost (OpenAI ≈ 85–1100 tokens by detail); we
/// charge a conservative upper bound so the budget stays realistic without the
/// base64 payload ever inflating it.
const IMAGE_MARKER_TOKEN_COST: usize = 1_200;

/// Rough token estimate: ~4 characters per token (matches tree summarizer).
///
/// Image markers are charged at a flat [`IMAGE_MARKER_TOKEN_COST`] rather than
/// counting their base64 payload as text — see that constant for the rationale.
/// Markerless text takes the fast path and is unchanged.
pub fn estimate_tokens(text: &str) -> usize {
    if !text.contains(IMAGE_MARKER_PREFIX) {
        return text.len().saturating_add(3) / 4;
    }

    let mut text_bytes = 0usize;
    let mut images = 0usize;
    let mut cursor = 0usize;
    while let Some(rel) = text[cursor..].find(IMAGE_MARKER_PREFIX) {
        let start = cursor + rel;
        text_bytes += start - cursor; // text preceding the marker
        let after = start + IMAGE_MARKER_PREFIX.len();
        match text[after..].find(']') {
            Some(rel_end) => {
                images += 1;
                cursor = after + rel_end + 1; // skip the whole marker payload
            }
            None => {
                // Unterminated marker — count the remainder as text and stop.
                text_bytes += text.len() - start;
                cursor = text.len();
                break;
            }
        }
    }
    text_bytes += text.len() - cursor; // trailing text after the last marker

    (text_bytes.saturating_add(3) / 4)
        .saturating_add(images.saturating_mul(IMAGE_MARKER_TOKEN_COST))
}

pub fn estimate_chat_message_tokens(msg: &ChatMessage) -> usize {
    estimate_tokens(&msg.content)
}

pub fn estimate_conversation_message_tokens(msg: &ConversationMessage) -> usize {
    match msg {
        ConversationMessage::Chat(chat) => estimate_chat_message_tokens(chat),
        ConversationMessage::AssistantToolCalls {
            text,
            tool_calls,
            reasoning_content,
        } => {
            let body = text.as_deref().unwrap_or_default();
            let mut total = estimate_tokens(body);
            if let Some(reasoning) = reasoning_content.as_deref() {
                total = total.saturating_add(estimate_tokens(reasoning));
            }
            for call in tool_calls {
                total = total.saturating_add(estimate_tokens(&call.name));
                total = total.saturating_add(estimate_tokens(&call.arguments));
            }
            total
        }
        ConversationMessage::ToolResults(results) => results
            .iter()
            .map(|r| estimate_tokens(&r.tool_call_id).saturating_add(estimate_tokens(&r.content)))
            .sum(),
    }
}

fn output_reserve_tokens(context_window: u64) -> u64 {
    let pct = context_window / 10;
    pct.max(MIN_OUTPUT_RESERVE_TOKENS)
        .min(DEFAULT_OUTPUT_RESERVE_TOKENS.max(context_window / 4))
}

fn max_input_tokens(context_window: u64) -> u64 {
    context_window.saturating_sub(output_reserve_tokens(context_window))
}

/// Trim `messages` oldest-first (never removing `system` role) until the
/// estimated prompt fits `context_window`.
pub fn trim_chat_messages_to_budget(
    messages: &mut Vec<ChatMessage>,
    context_window: u64,
) -> TokenBudgetOutcome {
    trim_messages_to_budget(
        messages,
        context_window,
        estimate_chat_message_tokens,
        |msg| msg.role == "system",
        |msg| msg.role == "tool",
    )
}

/// Trim conversation `history` oldest-first, preserving system chat messages.
pub fn trim_conversation_history_to_budget(
    history: &mut Vec<ConversationMessage>,
    context_window: u64,
) -> TokenBudgetOutcome {
    trim_messages_to_budget(
        history,
        context_window,
        estimate_conversation_message_tokens,
        |msg| matches!(msg, ConversationMessage::Chat(c) if c.role == "system"),
        |msg| matches!(msg, ConversationMessage::ToolResults(_)),
    )
}

fn trim_messages_to_budget<T, F, P, R>(
    messages: &mut Vec<T>,
    context_window: u64,
    estimate: F,
    is_system: P,
    is_tool_result: R,
) -> TokenBudgetOutcome
where
    F: Fn(&T) -> usize,
    P: Fn(&T) -> bool,
    R: Fn(&T) -> bool,
{
    let max_tokens = max_input_tokens(context_window) as usize;
    let original_tokens: usize = messages.iter().map(&estimate).sum();

    if original_tokens <= max_tokens {
        return TokenBudgetOutcome {
            original_tokens,
            final_tokens: original_tokens,
            messages_removed: 0,
            trimmed: false,
        };
    }

    // Drop oldest non-system messages until the budget fits, preserving the
    // original relative order of every retained message (system + non-system).
    // Rebuilding as `system ++ other` would reorder history when a system
    // message appears after non-system messages, which changes prompt
    // semantics (see PR #2100 CodeRabbit review).
    let mut removable_positions: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter_map(|(idx, msg)| (!is_system(msg)).then_some(idx))
        .collect();

    let mut removed = 0usize;
    while !removable_positions.is_empty() {
        let total: usize = messages.iter().map(&estimate).sum();
        if total <= max_tokens {
            break;
        }
        let absolute_idx = removable_positions.remove(0);
        // Subsequent positions shift left by one for every prior removal.
        let remove_at = absolute_idx - removed;
        messages.remove(remove_at);
        removed += 1;
    }

    // Snap the window forward past any leading orphaned tool results.
    //
    // Oldest-first eviction removes whole messages from the front, so it can
    // drop an `assistant(tool_calls)` while keeping the `tool` result(s) that
    // answered it — leaving the window opening on a tool message with no
    // preceding `tool_calls`. The provider rejects that with a 400 (`messages
    // with role 'tool' must be a response to a preceding message with
    // 'tool_calls'`), which streams back empty and surfaces as a generic
    // "Something went wrong". Drop leading tool-result messages (covering both
    // single and parallel tool cycles) until the first non-system message is a
    // clean turn boundary. Mirrors `session::turn::trim_history`'s orphan-snap
    // and the summarizer's `snap_split_forward`; the wire-boundary
    // `enforce_tool_message_invariants` remains the final repair.
    while let Some(first_non_system) = messages.iter().position(|m| !is_system(m)) {
        if is_tool_result(&messages[first_non_system]) {
            messages.remove(first_non_system);
            removed += 1;
        } else {
            break;
        }
    }

    let final_tokens: usize = messages.iter().map(&estimate).sum();

    TokenBudgetOutcome {
        original_tokens,
        final_tokens,
        messages_removed: removed,
        trimmed: removed > 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::inference::provider::ToolCall;

    fn user_msg(content: &str) -> ChatMessage {
        ChatMessage::user(content.to_string())
    }

    #[test]
    fn estimate_tokens_charges_flat_cost_per_image_marker_not_base64_length() {
        // A ~120k-char base64 data URI inside an `[IMAGE:]` marker must NOT be
        // counted as text (that read as ~30k tokens and got the image trimmed,
        // #3205). It should cost a flat IMAGE_MARKER_TOKEN_COST plus the small
        // surrounding text, regardless of payload size.
        let surrounding = "describe this picture";
        let big_uri = format!("data:image/png;base64,{}", "Q".repeat(120_000));
        let with_image = format!("{surrounding} [IMAGE:{big_uri}]");

        let est = estimate_tokens(&with_image);
        let text_only = estimate_tokens(surrounding);
        assert_eq!(est, text_only.saturating_add(IMAGE_MARKER_TOKEN_COST));
        assert!(
            est < 2_000,
            "image marker must not be counted by base64 length (got {est})"
        );
    }

    #[test]
    fn estimate_tokens_counts_each_image_marker_once() {
        let two = "[IMAGE:data:image/png;base64,AAA] and [IMAGE:https://x/y.jpg]";
        let est = estimate_tokens(two);
        assert!(est >= 2 * IMAGE_MARKER_TOKEN_COST);
        assert!(est < 2 * IMAGE_MARKER_TOKEN_COST + 50);
    }

    #[test]
    fn estimate_tokens_markerless_text_uses_char_heuristic() {
        let text = "x".repeat(400);
        assert_eq!(estimate_tokens(&text), 100);
    }

    #[test]
    fn image_marker_message_is_not_trimmed_within_budget() {
        // End-to-end: a huge base64 image in the newest user turn must survive
        // the budget trim (it used to be evicted as ~30k tokens).
        let big = format!("look [IMAGE:data:image/png;base64,{}]", "Q".repeat(150_000));
        let mut messages = vec![ChatMessage::system("sys"), user_msg(&big)];
        let outcome = trim_chat_messages_to_budget(&mut messages, 200_000);
        assert!(
            !outcome.trimmed,
            "image message must fit the budget, not be trimmed"
        );
        assert_eq!(messages.len(), 2);
        assert!(messages[1].content.contains("[IMAGE:"));
    }

    #[test]
    fn under_limit_passes_through_unchanged() {
        let mut messages = vec![
            ChatMessage::system("sys"),
            user_msg("hello"),
            ChatMessage::assistant("hi"),
        ];
        let before_len = messages.len();
        let outcome = trim_chat_messages_to_budget(&mut messages, 100_000);
        assert!(!outcome.trimmed);
        assert_eq!(outcome.original_tokens, outcome.final_tokens);
        assert_eq!(messages.len(), before_len);
    }

    #[test]
    fn over_limit_truncates_oldest_non_system_first() {
        let mut messages = vec![
            ChatMessage::system("system prompt"),
            user_msg(&"x".repeat(400_000)),
            user_msg("keep-me"),
        ];
        let outcome = trim_chat_messages_to_budget(&mut messages, 1_000);
        assert!(outcome.trimmed);
        assert!(outcome.final_tokens < outcome.original_tokens);
        assert!(outcome.messages_removed >= 1);
        assert_eq!(messages.first().unwrap().role, "system");
        assert!(
            messages.iter().any(|m| m.content.contains("keep-me")),
            "newest user message should survive trimming"
        );
    }

    #[test]
    fn trim_conversation_history_drops_oldest_messages() {
        let mut messages = vec![ConversationMessage::Chat(user_msg(&"y".repeat(80_000)))];
        let outcome = trim_conversation_history_to_budget(&mut messages, 1_000);
        assert!(outcome.trimmed);
        assert!(outcome.original_tokens > outcome.final_tokens);
    }

    #[test]
    fn conversation_tool_results_are_counted_in_estimate() {
        let msg = ConversationMessage::ToolResults(vec![
            crate::openhuman::inference::provider::ToolResultMessage {
                tool_call_id: "c1".into(),
                content: "z".repeat(8_000),
            },
        ]);
        assert!(estimate_conversation_message_tokens(&msg) > 1_000);
    }

    #[test]
    fn trim_preserves_relative_order_when_system_appears_late() {
        // System message in the middle of history must not be moved to the
        // front during trimming. Regression guard for PR #2100 review.
        let mut messages = vec![
            user_msg(&"a".repeat(40_000)), // oldest non-system, expected to drop
            user_msg("first-user"),
            ChatMessage::system("late-system"),
            user_msg("last-user"),
        ];
        let outcome = trim_chat_messages_to_budget(&mut messages, 1_000);
        assert!(outcome.trimmed);
        // System position relative to surrounding messages is preserved.
        let roles: Vec<&str> = messages.iter().map(|m| m.role.as_str()).collect();
        let sys_idx = roles
            .iter()
            .position(|r| *r == "system")
            .expect("system message must be retained");
        // At least one user message should still precede the late system message.
        assert!(
            sys_idx > 0,
            "late system message must remain after earlier surviving non-system messages"
        );
        assert!(
            messages.iter().any(|m| m.content == "last-user"),
            "newest user message must survive"
        );
    }

    #[test]
    fn assistant_tool_calls_estimate_includes_arguments() {
        let msg = ConversationMessage::AssistantToolCalls {
            text: Some("thinking".into()),
            tool_calls: vec![ToolCall {
                id: "1".into(),
                name: "echo".into(),
                arguments: "{\"value\":\"x\"}".into(),
                extra_content: None,
            }],
            reasoning_content: None,
        };
        assert!(estimate_conversation_message_tokens(&msg) > 0);
    }

    fn tool_results(ids: &[&str]) -> ConversationMessage {
        ConversationMessage::ToolResults(
            ids.iter()
                .map(
                    |id| crate::openhuman::inference::provider::ToolResultMessage {
                        tool_call_id: (*id).into(),
                        content: format!("result-{id}"),
                    },
                )
                .collect(),
        )
    }

    fn tool_call(id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "f".into(),
            arguments: "{}".into(),
            extra_content: None,
        }
    }

    #[test]
    fn chat_budget_trim_snaps_past_orphaned_tool_result() {
        // Oldest-first eviction drops the assistant turn and would leave the
        // `tool` result as a leading orphan (provider 400). The snap must drop
        // it so the window opens on a clean turn boundary.
        let mut messages = vec![
            ChatMessage::system("sys"),
            ChatMessage::assistant(&"a".repeat(400_000)), // oldest non-system → evicted
            ChatMessage::tool("result for the evicted tool call"),
            user_msg("keep-me"),
        ];
        let outcome = trim_chat_messages_to_budget(&mut messages, 1_000);
        assert!(outcome.trimmed);
        let first_non_system = messages
            .iter()
            .find(|m| m.role != "system")
            .expect("a non-system message survives");
        assert_ne!(
            first_non_system.role, "tool",
            "leading orphan tool result must be snapped away, not sent to the provider"
        );
        assert!(messages.iter().any(|m| m.content == "keep-me"));
    }

    #[test]
    fn conversation_budget_trim_snaps_past_orphaned_parallel_tool_results() {
        // A parallel cycle: one AssistantToolCalls answered by two ToolResults.
        // Evicting the call must not leave either result orphaned at the head.
        let mut history = vec![
            ConversationMessage::Chat(ChatMessage::system("sys")),
            ConversationMessage::AssistantToolCalls {
                text: Some("x".repeat(400_000)), // oldest non-system → evicted
                tool_calls: vec![tool_call("X"), tool_call("Y")],
                reasoning_content: None,
            },
            tool_results(&["X"]),
            tool_results(&["Y"]),
            ConversationMessage::Chat(user_msg("keep-me")),
        ];
        let outcome = trim_conversation_history_to_budget(&mut history, 1_000);
        assert!(outcome.trimmed);
        let first_non_system = history
            .iter()
            .find(|m| !matches!(m, ConversationMessage::Chat(c) if c.role == "system"))
            .expect("a non-system message survives");
        assert!(
            !matches!(first_non_system, ConversationMessage::ToolResults(_)),
            "leading orphan tool results must be snapped away"
        );
        assert!(history
            .iter()
            .any(|m| matches!(m, ConversationMessage::Chat(c) if c.content == "keep-me")));
    }

    #[test]
    fn conversation_budget_trim_keeps_paired_call_and_result_at_window_head() {
        // When the post-trim window opens on an assistant(tool_calls) followed
        // by its result, the snap must NOT remove the (validly paired) result.
        let mut history = vec![
            ConversationMessage::Chat(user_msg(&"x".repeat(400_000))), // evicted
            ConversationMessage::AssistantToolCalls {
                text: None,
                tool_calls: vec![tool_call("A")],
                reasoning_content: None,
            },
            tool_results(&["A"]),
            ConversationMessage::Chat(user_msg("keep")),
        ];
        let outcome = trim_conversation_history_to_budget(&mut history, 1_000);
        assert!(outcome.trimmed);
        assert!(
            matches!(
                history.first(),
                Some(ConversationMessage::AssistantToolCalls { .. })
            ),
            "window should open on the surviving tool call"
        );
        assert!(
            history
                .iter()
                .any(|m| matches!(m, ConversationMessage::ToolResults(_))),
            "a validly-paired tool result must be retained, not over-snapped"
        );
    }
}
