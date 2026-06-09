//! Parsing and response-extraction free functions for the OpenAI-compatible provider.
//!
//! All functions here are stateless transforms — no I/O, no HTTP. They take
//! raw strings or deserialized values and return structured results.

use crate::openhuman::inference::provider::traits::{
    ChatMessage, StreamError, StreamResult, ToolCall as ProviderToolCall,
};

use super::compatible_types::{
    ApiChatResponse, ResponsesContentPart, ResponsesInput, ResponsesResponse, StreamChunkResponse,
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
    crate::openhuman::inference::provider::sanitize_api_error(body)
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
            } else if serde_json::from_str::<serde_json::Value>(&raw).is_ok() {
                raw
            } else {
                // OPENHUMAN-TAURI-6F: model emitted malformed JSON in
                // `function.arguments`. Log the discard so it's traceable
                // without leaking argument contents (which may contain PII).
                log::warn!(
                    "[providers] normalize_function_arguments: \
                     discarding malformed JSON string (len={}) — substituting {{}}",
                    raw.len()
                );
                "{}".to_string()
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
                // Route through normalize_function_arguments so malformed
                // JSON strings in pre-deserialized ProviderToolCall values
                // receive the same guard as the function.arguments path below.
                arguments: normalize_function_arguments(Some(serde_json::Value::String(
                    call.arguments,
                ))),
                // Preserve Gemini's thought_signature through the stored-history
                // recovery path so it is echoed on the next turn (TAURI-RUST-4PK).
                extra_content: call.extra_content,
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
        // Carry Gemini's thought_signature if the stored value had one
        // (TAURI-RUST-4PK).
        extra_content: value.get("extra_content").cloned(),
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
            content: vec![ResponsesContentPart {
                kind: if message.role == "assistant" {
                    "output_text".to_string()
                } else {
                    "input_text".to_string()
                },
                text: message.content.clone(),
            }],
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

/// Aggregate an OpenAI Responses-API **SSE** body into the final assistant
/// text (#3201).
///
/// The Codex/ChatGPT OAuth Responses endpoint
/// (`https://chatgpt.com/backend-api/codex/responses`) rejects requests with
/// `stream: false` (`Stream must be set to true`), so callers that target it
/// must send `stream: true` and parse the resulting Server-Sent Event
/// stream instead of the single JSON envelope `parse_responses_response_body`
/// handles.
///
/// SSE shape (simplified — only the parts we depend on):
///
/// ```text
/// event: response.output_text.delta
/// data: {"type":"response.output_text.delta","delta":"Hello"}
///
/// event: response.output_text.delta
/// data: {"type":"response.output_text.delta","delta":" world"}
///
/// event: response.completed
/// data: {"type":"response.completed","response":{"output_text":"Hello world", ...}}
///
/// data: [DONE]
/// ```
///
/// Strategy:
///
/// - Walk every `data: …` line (the `event:` line is informational; we route
///   off the `type` field inside the JSON payload for resilience to the
///   sentinel-style endings some providers emit).
/// - `[DONE]` and empty data lines terminate the loop cleanly.
/// - `response.output_text.delta` → push `delta` onto the accumulator.
/// - `response.completed` → if we have a non-empty terminal
///   `response.output_text`, prefer it (covers providers that batch the full
///   text in the completion event and omit deltas).
/// - Unrecognized `type` values are ignored — the spec is open-ended (tool
///   calls, reasoning summaries, …) and we only need the assistant text here.
///
/// Returns the joined text on success. The error path returns the
/// snippet-sanitised body just like [`parse_responses_response_body`] so a
/// genuinely malformed stream is debuggable without leaking arbitrary chunk
/// payloads.
pub(crate) fn aggregate_responses_sse_body(
    provider_name: &str,
    body: &str,
) -> anyhow::Result<String> {
    let mut accumulated = String::new();
    let mut terminal_text: Option<String> = None;

    for raw_line in body.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(error) => {
                // Skip individual unparseable events rather than failing the
                // whole turn — providers occasionally emit comments/keepalives
                // shaped like `data: {ping}` that aren't strict JSON.
                log::debug!(
                    "[providers][{provider_name}] Responses SSE: skipping unparseable event ({error})"
                );
                continue;
            }
        };

        let event_type = value.get("type").and_then(serde_json::Value::as_str);
        match event_type {
            Some("response.output_text.delta") => {
                if let Some(delta) = value.get("delta").and_then(serde_json::Value::as_str) {
                    accumulated.push_str(delta);
                }
            }
            Some("response.completed") => {
                // Use the same "non-empty-after-trim" policy as
                // `extract_responses_text` / `first_nonempty` so a
                // whitespace-only terminal `output_text` doesn't override
                // a non-empty accumulated delta stream and collapse a
                // valid streamed reply to blank output.
                terminal_text = value
                    .get("response")
                    .and_then(|response| response.get("output_text"))
                    .and_then(serde_json::Value::as_str)
                    .and_then(|text| first_nonempty(Some(text)));
            }
            // Treat error-shaped events as a hard failure so the caller
            // surfaces the upstream reason instead of an empty completion.
            Some("response.failed") | Some("response.error") | Some("error") => {
                let snippet = compact_sanitized_body_snippet(data);
                anyhow::bail!(
                    "{provider_name} Responses API stream reported a failure event: {snippet}"
                );
            }
            _ => {}
        }
    }

    // Prefer the terminal `response.output_text` when it carries a non-empty
    // string — some providers batch full text in `response.completed` and
    // skip per-token deltas, and others repeat what we accumulated. Either
    // way the terminal text is the authoritative version on the wire.
    // (`first_nonempty` in the match arm above already filtered whitespace.)
    if let Some(text) = terminal_text {
        return Ok(text);
    }
    if !accumulated.is_empty() {
        return Ok(accumulated);
    }

    let snippet = compact_sanitized_body_snippet(body);
    anyhow::bail!(
        "{provider_name} Responses API SSE stream produced no text events; body={snippet}"
    )
}
