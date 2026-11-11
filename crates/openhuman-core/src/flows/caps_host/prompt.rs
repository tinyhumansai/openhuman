//! Host-side inference message and response projection.

use serde_json::{json, Value};
use tinyagents_session::transcript::TranscriptMessage;
use tinyflows_bus::portability::CompletionMessage;

/// Map module-normalized wire messages onto the host's transcript type.
pub(crate) fn build_completion_messages(messages: &[CompletionMessage]) -> Vec<TranscriptMessage> {
    messages
        .iter()
        .map(|message| match message.role.as_str() {
            "system" => TranscriptMessage::system(message.content.clone()),
            "assistant" => TranscriptMessage::assistant(message.content.clone()),
            "tool" => TranscriptMessage::tool(message.content.clone()),
            _ => TranscriptMessage::user(message.content.clone()),
        })
        .collect()
}

/// Maps provider usage onto the established flow completion wire shape.
pub(crate) fn usage_to_json(usage: &Option<tinyinference_llm::usage::BilledUsage>) -> Value {
    match usage {
        None => Value::Null,
        Some(usage) => json!({
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "context_window": usage.context_window(),
            "cached_input_tokens": usage.cached_input_tokens(),
            "cache_creation_tokens": usage.cache_creation_tokens,
            "reasoning_tokens": usage.reasoning_tokens,
            "charged_amount_usd": usage.charged_amount_usd,
        }),
    }
}

/// Maps a provider response to the established completion result envelope.
pub(crate) fn model_response_to_completion_value(
    response: &tinyinference_llm::model::ModelResponse,
) -> Value {
    json!({
        "text": response.text(),
        "tool_calls": response
            .tool_calls()
            .iter()
            .map(crate::agent::tinyagents::ta_call_to_oh_call)
            .collect::<Vec<_>>(),
        "usage": usage_to_json(
            &tinyinference_llm::usage::usage_info_from_response(response)
        ),
        "reasoning_content": tinyagents_runtime::transcript::reasoning_from_content(
            &response.message.content
        ),
    })
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
