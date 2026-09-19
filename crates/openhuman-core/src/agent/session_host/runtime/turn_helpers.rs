//! Static per-turn helpers: iteration counting, history diffing, event-error
//! sanitisation, and tool-call id fallback / persistence shaping.

use super::super::types::OpenHumanSessionHost;
use crate::agent::error::AgentError;
use crate::agent::messages::ConversationMessage;
use crate::inference::provider::ToolCall;
use crate::util::truncate_with_ellipsis;
use tinytools_agent::ParsedToolCall;

impl OpenHumanSessionHost {
    const EVENT_ERROR_MAX_CHARS: usize = 256;

    // ─────────────────────────────────────────────────────────────────
    // Static helpers for turn parsing + telemetry
    // ─────────────────────────────────────────────────────────────────

    pub(in crate::agent::session_host) fn count_iterations(
        messages: &[ConversationMessage],
    ) -> usize {
        messages
            .iter()
            .filter(|message| matches!(message, ConversationMessage::AssistantToolCalls { .. }))
            .count()
            + 1
    }

    fn conversation_message_eq(left: &ConversationMessage, right: &ConversationMessage) -> bool {
        serde_json::to_string(left).ok() == serde_json::to_string(right).ok()
    }

    fn message_slice_eq(left: &[ConversationMessage], right: &[ConversationMessage]) -> bool {
        left.len() == right.len()
            && left
                .iter()
                .zip(right.iter())
                .all(|(left, right)| Self::conversation_message_eq(left, right))
    }

    pub(in crate::agent::session_host) fn new_entries_for_turn<'a>(
        history_snapshot: &[ConversationMessage],
        current_history: &'a [ConversationMessage],
    ) -> &'a [ConversationMessage] {
        let common_prefix_len = history_snapshot
            .iter()
            .zip(current_history.iter())
            .take_while(|(left, right)| Self::conversation_message_eq(left, right))
            .count();

        if common_prefix_len == history_snapshot.len() {
            return &current_history[common_prefix_len..];
        }

        let max_overlap = history_snapshot.len().min(current_history.len());
        for overlap in (0..=max_overlap).rev() {
            let snapshot_suffix = &history_snapshot[history_snapshot.len() - overlap..];
            let current_prefix = &current_history[..overlap];
            if Self::message_slice_eq(snapshot_suffix, current_prefix) {
                return &current_history[overlap..];
            }
        }

        current_history
    }

    pub(in crate::agent::session_host) fn sanitize_event_error_message(
        err: &anyhow::Error,
    ) -> String {
        let kind = match err.downcast_ref::<AgentError>() {
            Some(AgentError::ProviderError { .. }) => Some("provider_error"),
            Some(AgentError::ContextLimitExceeded { .. }) => Some("context_limit_exceeded"),
            Some(AgentError::ToolExecutionError { .. }) => Some("tool_execution_error"),
            Some(AgentError::CostBudgetExceeded { .. }) => Some("cost_budget_exceeded"),
            Some(AgentError::MaxIterationsExceeded { .. }) => Some("max_iterations_exceeded"),
            Some(AgentError::EmptyProviderResponse { .. }) => Some("empty_provider_response"),
            Some(AgentError::CompactionFailed { .. }) => Some("compaction_failed"),
            Some(AgentError::PermissionDenied { .. }) => Some("permission_denied"),
            Some(AgentError::RegistryValidationFailed { .. }) => Some("registry_validation_failed"),
            Some(AgentError::Other(_)) | None => None,
        };

        if let Some(kind) = kind {
            return kind.to_string();
        }

        let scrubbed = tinyinference_core::sanitize::sanitize_api_error(&err.to_string())
            .replace(['\n', '\r', '\t'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        truncate_with_ellipsis(&scrubbed, Self::EVENT_ERROR_MAX_CHARS)
    }

    /// Injects unique IDs into tool calls that are missing them.
    ///
    /// This is necessary for some tool dispatchers to correctly track and
    /// associate results.
    pub(in crate::agent::session_host) fn with_fallback_tool_call_ids(
        mut parsed_calls: Vec<ParsedToolCall>,
        iteration: usize,
    ) -> Vec<ParsedToolCall> {
        for (idx, call) in parsed_calls.iter_mut().enumerate() {
            if call.id.is_none() {
                call.id = Some(format!("parsed-{}-{}", iteration + 1, idx + 1));
            }
        }
        parsed_calls
    }

    /// Converts parsed tool calls into the provider-standard `ToolCall` format.
    ///
    /// If the provider response already contains native tool calls, they are
    /// returned as-is.
    pub(in crate::agent::session_host) fn persisted_tool_calls_for_history(
        response: &crate::inference::provider::ChatResponse,
        parsed_calls: &[ParsedToolCall],
        iteration: usize,
    ) -> Vec<ToolCall> {
        if !response.tool_calls.is_empty() {
            return response.tool_calls.clone();
        }

        parsed_calls
            .iter()
            .enumerate()
            .map(|(idx, call)| ToolCall {
                id: call
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("parsed-{}-{}", iteration + 1, idx + 1)),
                name: call.name.clone(),
                arguments: call.arguments.to_string(),
                // Prompt-based tool calls carry no provider extra_content.
                extra_content: None,
            })
            .collect()
    }
}
