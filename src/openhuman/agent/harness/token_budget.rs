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

/// Rough token estimate: ~4 characters per token (matches tree summarizer).
pub fn estimate_tokens(text: &str) -> usize {
    text.len().saturating_add(3) / 4
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
    )
}

fn trim_messages_to_budget<T, F, P>(
    messages: &mut Vec<T>,
    context_window: u64,
    estimate: F,
    is_system: P,
) -> TokenBudgetOutcome
where
    F: Fn(&T) -> usize,
    P: Fn(&T) -> bool,
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
            }],
            reasoning_content: None,
        };
        assert!(estimate_conversation_message_tokens(&msg) > 0);
    }
}
