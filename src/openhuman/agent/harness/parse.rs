//! Tool-call parsing — OpenHuman's adapter over
//! [`tinyagents_harness::tool_calling`].
//!
//! The parsers themselves moved to the crate: `<tool_call>` tags in their
//! several spellings, fenced blocks, bare JSON, Anthropic-style `<invoke>` XML,
//! the GLM grammar, and the p-format bodies. Nothing about recovering a tool
//! call from model text is OpenHuman-specific, so none of it stayed.
//!
//! What is left below speaks OpenHuman's own wire vocabulary — its
//! `inference::provider::ToolCall`, its native-history JSON, its OpenAI
//! function-calling payload. All of it is `#[cfg(test)]`: these are the
//! fixtures its own tests assert against, and they never compiled into a
//! production build even before the move.

pub(crate) use tinyagents_harness::tool_calling::{
    extract_json_values, parse_tool_calls, parse_tool_calls_with_pformat,
};

// The rest of the crate's re-exports are only reached from this module's own
// tests (`tests.rs`) and `harness_gap_tests.rs`, not from any production call
// site — gated so a non-test build doesn't warn (and fail `-D warnings`) on
// them.
#[cfg(test)]
pub(crate) use tinyagents_harness::tool_calling::{
    parse_arguments_value, parse_glm_style_tool_calls, parse_tool_call_value,
    parse_tool_calls_from_json_value,
};

#[cfg(test)]
use crate::openhuman::inference::provider::ToolCall;
#[cfg(test)]
use crate::openhuman::tools::Tool;
#[cfg(test)]
use tinyagents_harness::tool_calling::ParsedToolCall;

#[cfg(test)]
pub(crate) fn parse_structured_tool_calls(tool_calls: &[ToolCall]) -> Vec<ParsedToolCall> {
    tool_calls
        .iter()
        .map(|call| ParsedToolCall {
            name: call.name.clone(),
            arguments: serde_json::from_str::<serde_json::Value>(&call.arguments)
                .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new())),
            id: Some(call.id.clone()),
        })
        .collect()
}

/// Build assistant history entry in JSON format for native tool-call APIs.
/// `convert_messages` in the OpenRouter provider parses this JSON to reconstruct
/// the proper `NativeMessage` with structured `tool_calls`.
///
/// `reasoning_content` carries the model's thinking output (when the provider
/// surfaced it). It is persisted so the next request can replay it: DeepSeek's
/// thinking mode rejects an `assistant` turn that carries `tool_calls` if its
/// `reasoning_content` is not passed back (Sentry TAURI-RUST-4KB). Omitted from
/// the JSON when empty, so non-reasoning models are unaffected.
#[cfg(test)]
pub(crate) fn build_native_assistant_history(
    text: &str,
    reasoning_content: Option<&str>,
    tool_calls: &[ToolCall],
) -> String {
    let calls_json: Vec<serde_json::Value> = tool_calls
        .iter()
        .map(|tc| {
            let mut call = serde_json::json!({
                "id": tc.id,
                "name": tc.name,
                "arguments": tc.arguments,
            });
            // Persist Gemini's per-call `thought_signature` (TAURI-RUST-4PK /
            // 4PJ) into the stored assistant turn. PR #3553 threaded the
            // signature through the live response→request hop and the
            // stored-history *parser* (`parse_provider_tool_call_from_value`),
            // but this writer — the single sink the agent loop persists every
            // native tool-call turn through (engine/core.rs) — dropped it. On a
            // history reload the rebuilt assistant turn therefore lacked
            // `extra_content`, so the echoed `functionCall` part went out with
            // no `thought_signature` and Gemini 400'd ("Function call is
            // missing a thought_signature in functionCall parts"). Write it
            // per-part so EVERY call in a parallel/multi-call turn round-trips,
            // not just the first; `skip_serializing_if = "Option::is_none"` on
            // `extra_content` keeps the stored JSON byte-identical for every
            // provider that doesn't emit it.
            if let Some(extra) = tc.extra_content.clone() {
                if let Some(obj) = call.as_object_mut() {
                    obj.insert("extra_content".to_string(), extra);
                }
            }
            call
        })
        .collect();

    let content = if text.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::String(text.trim().to_string())
    };

    let mut entry = serde_json::json!({
        "content": content,
        "tool_calls": calls_json,
    });

    if let Some(reasoning) = reasoning_content.map(str::trim).filter(|r| !r.is_empty()) {
        entry["reasoning_content"] = serde_json::Value::String(reasoning.to_string());
    }

    entry.to_string()
}

#[cfg(test)]
pub(crate) fn build_assistant_history_with_tool_calls(
    text: &str,
    tool_calls: &[ToolCall],
) -> String {
    let mut parts = Vec::new();

    if !text.trim().is_empty() {
        parts.push(text.trim().to_string());
    }

    for call in tool_calls {
        let arguments = serde_json::from_str::<serde_json::Value>(&call.arguments)
            .unwrap_or_else(|_| serde_json::Value::String(call.arguments.clone()));
        let payload = serde_json::json!({
            "id": call.id,
            "name": call.name,
            "arguments": arguments,
        });
        parts.push(format!("<tool_call>\n{payload}\n</tool_call>"));
    }

    parts.join("\n")
}

/// Convert a tool registry to OpenAI function-calling format for native tool support.
#[cfg(test)]
pub(crate) fn tools_to_openai_format(tools_registry: &[Box<dyn Tool>]) -> Vec<serde_json::Value> {
    tools_registry
        .iter()
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name(),
                    "description": tool.description(),
                    "parameters": tool.parameters_schema()
                }
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
