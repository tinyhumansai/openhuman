//! Normalization and repair helpers for local Qwen tool responses.

use tinyinference::message::ContentBlock;
use tinyinference::model::ModelResponse;
use tinyinference::tool::{ToolCall as TaToolCall, ToolSchema};

/// Normalize Qwen tool responses by repairing malformed bare-name calls and
/// enforcing single-tool execution.
pub(crate) fn normalize_qwen_tool_response(
    response: ModelResponse,
    advertised_tools: &[ToolSchema],
) -> ModelResponse {
    enforce_single_qwen_tool_call(repair_qwen_bare_name_tool_call(response, advertised_tools))
}

/// Repair Qwen bare name tool call when the model emits arguments or unquoted name
/// inside explicit tool-call tags, for a tool advertised on this request, and
/// when it produces an otherwise exact valid call object.
pub(crate) fn repair_qwen_bare_name_tool_call(
    mut response: ModelResponse,
    advertised_tools: &[ToolSchema],
) -> ModelResponse {
    if !response.message.tool_calls.is_empty() {
        return response;
    }
    let text = response.text();
    if let Some(repaired) = repair_qwen_bare_name_text(&text, advertised_tools) {
        response
            .message
            .content
            .retain(|block| !matches!(block, ContentBlock::Text(_)));
        response.message.content.push(ContentBlock::Text(repaired));
        response = tinyagents_harness::tool::apply_prompt_tool_calls(response);
    }
    if response.message.tool_calls.is_empty()
        && response.text().trim_start().starts_with("<tool_call>")
    {
        // An invalid/unadvertised protocol frame is neither a user answer nor
        // safe executable data. Do not leak internal markup into chat history.
        response
            .message
            .content
            .retain(|block| !matches!(block, ContentBlock::Text(_)));
        response.message.content.push(ContentBlock::Text(
            "I couldn't execute that action because the model returned an invalid tool call."
                .to_string(),
        ));
    }
    response
}

/// Local Qwen is more reliable when it acts, observes, then decides again. A
/// single generation occasionally emits several near-duplicate retrieval calls;
/// admitting only the first keeps the tool loop serial and lets a terminal
/// failure stop the turn before more paid calls launch.
fn enforce_single_qwen_tool_call(mut response: ModelResponse) -> ModelResponse {
    response.message.tool_calls.truncate(1);
    response
}

fn repair_qwen_bare_name_text(text: &str, advertised_tools: &[ToolSchema]) -> Option<String> {
    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut changed = false;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start + OPEN.len()]);
        let after_open = &rest[start + OPEN.len()..];
        let end = after_open.find(CLOSE)?;
        let body = &after_open[..end];
        if let Some(repaired) = repair_qwen_bare_name_body(body, advertised_tools)
            .or_else(|| repair_qwen_missing_name_body(body, advertised_tools))
        {
            out.push_str(&repaired);
            changed = true;
        } else {
            out.push_str(body);
        }
        out.push_str(CLOSE);
        rest = &after_open[end + CLOSE.len()..];
    }
    out.push_str(rest);
    changed.then_some(out)
}

/// Recover a name-less Qwen call only when its arguments validate against one
/// and only one advertised schema. This cannot widen the turn's allowlist and
/// refuses ambiguous or destructive inference.
fn repair_qwen_missing_name_body(body: &str, advertised_tools: &[ToolSchema]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    let arguments = object.get("arguments")?.as_object()?;
    let arguments = serde_json::Value::Object(arguments.clone());
    let mut matches = advertised_tools.iter().filter(|schema| {
        // Never infer an acting tool. Name-less repair is limited to the
        // read-only retrieval call observed from this model alias.
        matches!(schema.name.as_str(), "web_fetch" | "browser_open")
            && schema
                .validate_call(&TaToolCall::new(
                    "qwen-missing-name",
                    &schema.name,
                    arguments.clone(),
                ))
                .is_ok()
    });
    let schema = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(
        serde_json::json!({
            "name": schema.name,
            "arguments": arguments,
        })
        .to_string(),
    )
}

fn repair_qwen_bare_name_body(body: &str, advertised_tools: &[ToolSchema]) -> Option<String> {
    let after_open = body.trim().strip_prefix('{')?.trim_start();
    let string_end = json_string_literal_end(after_open)?;
    let name_literal = &after_open[..=string_end];
    let name: String = serde_json::from_str(name_literal).ok()?;
    if !advertised_tools.iter().any(|tool| tool.name == name) {
        return None;
    }
    let remainder = &after_open[string_end + 1..];
    if !remainder.trim_start().starts_with(',') {
        return None;
    }
    let repaired = format!(r#"{{"name":{name_literal}{remainder}"#);
    let value: serde_json::Value = serde_json::from_str(&repaired).ok()?;
    let object = value.as_object()?;
    if object.len() != 2
        || object.get("name").and_then(serde_json::Value::as_str) != Some(name.as_str())
        || !object
            .get("arguments")
            .is_some_and(serde_json::Value::is_object)
    {
        return None;
    }
    Some(repaired)
}

fn json_string_literal_end(value: &str) -> Option<usize> {
    if !value.starts_with('"') {
        return None;
    }
    let mut escaped = false;
    for (index, byte) in value.bytes().enumerate().skip(1) {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            return Some(index);
        }
    }
    None
}
