//! Parsing and response-extraction free functions for the OpenAI-compatible provider.
//!
//! All functions here are stateless transforms — no I/O, no HTTP. They take
//! raw strings or deserialized values and return structured results.

use crate::openhuman::providers::traits::{
    ChatMessage, StreamError, StreamResult, ToolCall as ProviderToolCall,
};

use super::compatible_types::{
    ApiChatResponse, ResponsesInput, ResponsesResponse, StreamChunkResponse,
};

// ── Think-tag stripping ───────────────────────────────────────────────────────

/// Remove `<think>...</think>` blocks from model output.
/// Some reasoning models (e.g. MiniMax) embed their chain-of-thought inline
/// in the `content` field rather than a separate `reasoning_content` field.
/// The resulting `<think>` tags must be stripped before returning to the user.
pub(crate) fn strip_think_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        if let Some(start) = rest.find("<think>") {
            result.push_str(&rest[..start]);
            if let Some(end) = rest[start..].find("</think>") {
                rest = &rest[start + end + "</think>".len()..];
            } else {
                // Unclosed tag: drop the rest to avoid leaking partial reasoning.
                break;
            }
        } else {
            result.push_str(rest);
            break;
        }
    }
    result.trim().to_string()
}

// ── SSE line parser ───────────────────────────────────────────────────────────

/// Parse a single SSE (Server-Sent Events) line from OpenAI-compatible providers.
/// Handles the `data: {...}` format and `[DONE]` sentinel.
pub(crate) fn parse_sse_line(line: &str) -> StreamResult<Option<String>> {
    let line = line.trim();

    // Skip empty lines and comments
    if line.is_empty() || line.starts_with(':') {
        return Ok(None);
    }

    // SSE format: "data: {...}"
    if let Some(data) = line.strip_prefix("data:") {
        let data = data.trim();

        // Check for [DONE] sentinel
        if data == "[DONE]" {
            return Ok(None);
        }

        // Parse JSON delta
        let chunk: StreamChunkResponse = serde_json::from_str(data).map_err(StreamError::Json)?;

        // Extract content from delta
        if let Some(choice) = chunk.choices.first() {
            if let Some(content) = &choice.delta.content {
                if !content.is_empty() {
                    return Ok(Some(content.clone()));
                }
            }
            // Fallback to reasoning_content for thinking models
            if let Some(reasoning) = &choice.delta.reasoning_content {
                return Ok(Some(reasoning.clone()));
            }
        }
    }

    Ok(None)
}

// ── Response body parsers ─────────────────────────────────────────────────────

pub(crate) fn compact_sanitized_body_snippet(body: &str) -> String {
    // super = compatible module; super::super = providers module (where sanitize_api_error lives)
    super::super::sanitize_api_error(body)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn parse_chat_response_body(
    provider_name: &str,
    body: &str,
) -> anyhow::Result<ApiChatResponse> {
    serde_json::from_str::<ApiChatResponse>(body).map_err(|error| {
        let snippet = compact_sanitized_body_snippet(body);
        anyhow::anyhow!(
            "{provider_name} API returned an unexpected chat-completions payload: {error}; body={snippet}"
        )
    })
}

pub(crate) fn parse_responses_response_body(
    provider_name: &str,
    body: &str,
) -> anyhow::Result<ResponsesResponse> {
    serde_json::from_str::<ResponsesResponse>(body).map_err(|error| {
        let snippet = compact_sanitized_body_snippet(body);
        anyhow::anyhow!(
            "{provider_name} Responses API returned an unexpected payload: {error}; body={snippet}"
        )
    })
}

// ── Tool-call argument normalisation ─────────────────────────────────────────

pub(crate) fn normalize_function_arguments(arguments: Option<serde_json::Value>) -> String {
    match arguments {
        Some(serde_json::Value::String(raw)) => {
            if raw.trim().is_empty() {
                "{}".to_string()
            } else {
                raw
            }
        }
        Some(serde_json::Value::Null) | None => "{}".to_string(),
        Some(other) => serde_json::to_string(&other).unwrap_or_else(|_| "{}".to_string()),
    }
}

pub(crate) fn parse_provider_tool_call_from_value(
    value: &serde_json::Value,
) -> Option<ProviderToolCall> {
    if let Ok(call) = serde_json::from_value::<ProviderToolCall>(value.clone()) {
        if !call.name.trim().is_empty() {
            return Some(ProviderToolCall {
                id: if call.id.trim().is_empty() {
                    uuid::Uuid::new_v4().to_string()
                } else {
                    call.id
                },
                name: call.name,
                arguments: if call.arguments.trim().is_empty() {
                    "{}".to_string()
                } else {
                    call.arguments
                },
            });
        }
    }

    let function = value.get("function")?;
    let name = function.get("name").and_then(serde_json::Value::as_str)?;
    if name.trim().is_empty() {
        return None;
    }

    let id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    Some(ProviderToolCall {
        id,
        name: name.to_string(),
        arguments: normalize_function_arguments(function.get("arguments").cloned()),
    })
}

pub(crate) fn parse_tool_calls_from_content_json(
    content: &str,
) -> Option<(Option<String>, Vec<ProviderToolCall>)> {
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let tool_calls_value = value.get("tool_calls")?.as_array()?;
    let tool_calls: Vec<ProviderToolCall> = tool_calls_value
        .iter()
        .filter_map(parse_provider_tool_call_from_value)
        .collect();
    if tool_calls.is_empty() {
        return None;
    }

    let text = value
        .get("content")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    Some((text, tool_calls))
}

// ── Responses API helpers ─────────────────────────────────────────────────────

pub(crate) fn first_nonempty(text: Option<&str>) -> Option<String> {
    text.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(crate) fn normalize_responses_role(role: &str) -> &'static str {
    match role {
        "assistant" => "assistant",
        "tool" => "assistant",
        _ => "user",
    }
}

pub(crate) fn build_responses_prompt(
    messages: &[ChatMessage],
) -> (Option<String>, Vec<ResponsesInput>) {
    let mut instructions_parts = Vec::new();
    let mut input = Vec::new();

    for message in messages {
        if message.content.trim().is_empty() {
            continue;
        }

        if message.role == "system" {
            instructions_parts.push(message.content.clone());
            continue;
        }

        input.push(ResponsesInput {
            role: normalize_responses_role(&message.role).to_string(),
            content: message.content.clone(),
        });
    }

    let instructions = if instructions_parts.is_empty() {
        None
    } else {
        Some(instructions_parts.join("\n\n"))
    };

    (instructions, input)
}

pub(crate) fn extract_responses_text(response: ResponsesResponse) -> Option<String> {
    if let Some(text) = first_nonempty(response.output_text.as_deref()) {
        return Some(text);
    }

    for item in &response.output {
        for content in &item.content {
            if content.kind.as_deref() == Some("output_text") {
                if let Some(text) = first_nonempty(content.text.as_deref()) {
                    return Some(text);
                }
            }
        }
    }

    for item in &response.output {
        for content in &item.content {
            if let Some(text) = first_nonempty(content.text.as_deref()) {
                return Some(text);
            }
        }
    }

    None
}
