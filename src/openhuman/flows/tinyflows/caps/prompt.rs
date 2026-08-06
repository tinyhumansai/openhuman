//! Turning a node's request into a completion, and a completion back into a
//! node result.
//!
//! Message assembly, the `input_context` carrier and its size cap, the
//! structured-output contract, and the tolerant JSON extraction a model reply
//! needs when it wraps its object in prose or a fenced block.

#![allow(unused_imports)]

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::caps::*;
use tinyflows::error::{EngineError, Result};

use super::*;
use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::{is_raw_passthrough_model, UsageInfo};

/// Maps a `UsageInfo` (not `Serialize`) into a JSON value field-by-field, so
/// [`OpenHumanLlm::complete`] can surface it in its response `Value` without
/// requiring an upstream `Serialize` impl change.
pub(crate) fn usage_to_json(usage: &Option<UsageInfo>) -> Value {
    match usage {
        None => Value::Null,
        Some(u) => json!({
            "input_tokens": u.input_tokens,
            "output_tokens": u.output_tokens,
            "context_window": u.context_window,
            "cached_input_tokens": u.cached_input_tokens,
            "cache_creation_tokens": u.cache_creation_tokens,
            "reasoning_tokens": u.reasoning_tokens,
            "charged_amount_usd": u.charged_amount_usd,
        }),
    }
}

pub(crate) fn model_response_to_completion_value(
    response: &tinyagents::harness::model::ModelResponse,
) -> Value {
    json!({
        "text": response.text(),
        "tool_calls": response
            .tool_calls()
            .iter()
            .map(crate::openhuman::agent::tinyagents::ta_call_to_oh_call)
            .collect::<Vec<_>>(),
        "usage": usage_to_json(
            &crate::openhuman::agent::tinyagents::model::usage_info_from_response(response)
        ),
        "reasoning_content": crate::openhuman::agent::tinyagents::reasoning_from_content(
            &response.message.content
        ),
    })
}

/// Cap on the serialized `input_context` block size (bytes of the pretty-
/// printed JSON) before truncation. Keeps a huge upstream payload (e.g. a
/// large fan-in `=items` array) from blowing the completion's context window;
/// generous enough that ordinary node outputs never hit it.
pub(crate) const INPUT_CONTEXT_MAX_LEN: usize = 50_000;

/// Renders an agent-node's `config.input_context` (an explicit `=`-bound
/// carrier for upstream data — see the module doc and
/// `flows/agents/workflow_builder/prompt.md`) into the system-message text
/// both completion paths ([`OpenHumanLlm::complete`] and
/// [`OpenHumanAgentRunner::run_via_harness`]) prepend ahead of the node's own
/// prompt/messages.
///
/// Returns `None` when `input_context` is absent or resolved to `null` (an
/// unset or dangling `=`-binding) so a node that doesn't opt in behaves
/// exactly as before this field existed — no injected block, no wording
/// change. This is the fix for the root cause: an `agent` node's only input
/// channel used to be `config.prompt` itself, forcing builders to smuggle
/// data in via a jq `=`-expression woven into prose (e.g. `"=You are given an
/// email: .item. Classify..."`), which is not a valid jq program and silently
/// resolves to `null` — the agent then runs with an empty prompt. An explicit
/// `input_context` binding (a clean `=item` / `=nodes.<id>.item.json`
/// expression) always resolves to real data or `null`, never to an
/// unparseable string, so this path can't repeat that failure.
pub(crate) fn input_context_block(request: &Value) -> Option<String> {
    let ctx = request.get("input_context").filter(|v| !v.is_null())?;
    let mut serialized = serde_json::to_string_pretty(ctx).unwrap_or_default();
    if serialized.is_empty() || serialized == "null" {
        return None;
    }
    if serialized.len() > INPUT_CONTEXT_MAX_LEN {
        // Truncate on a char boundary — `serialized` is UTF-8 and a naive byte
        // slice at exactly `INPUT_CONTEXT_MAX_LEN` could land mid-codepoint.
        let mut end = INPUT_CONTEXT_MAX_LEN;
        while !serialized.is_char_boundary(end) {
            end -= 1;
        }
        serialized.truncate(end);
        serialized.push_str("…(truncated)");
    }
    // `input_context` is untrusted upstream data (e.g. an email/webhook
    // payload) that could itself contain a run of backticks. A fixed
    // ```` ``` ```` fence would let such a payload prematurely close the
    // fence and have its own trailing text read as if it were prompt prose
    // rather than inert data. Use a fence one backtick longer than the
    // longest backtick run actually present in the payload — the same
    // "fence-following" convention Markdown renderers use — so the payload
    // can never break out.
    let fence = "`".repeat((longest_backtick_run(&serialized) + 1).max(3));
    Some(format!(
        "Here is the data from the previous step:\n{fence}json\n{serialized}\n{fence}\nUse this \
         data to complete the task described below."
    ))
}

/// Length of the longest run of consecutive backtick characters in `s` (0 if
/// `s` contains none). Used by [`input_context_block`] to size a code fence
/// that the untrusted payload cannot prematurely close.
pub(crate) fn longest_backtick_run(s: &str) -> usize {
    s.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}

/// Returns true when an agent-node completion `request` asked for structured
/// output: an `output_parser.schema` is configured on the node, or the config
/// sets `response_format: "json"`.
///
/// This is the host-side contract for **agent → tool wiring**: downstream
/// `=item.<field>` bindings only work when the agent's emitted item is a
/// structured object, so an agent feeding a `tool_call` should declare an
/// output schema (or `response_format: "json"`).
pub(crate) fn structured_output_requested(request: &Value) -> bool {
    let has_schema = request
        .get("output_parser")
        .and_then(|p| p.get("schema"))
        .is_some_and(|s| !s.is_null());
    let json_format = request.get("response_format").and_then(Value::as_str) == Some("json");
    has_schema || json_format
}

/// Builds [`OpenHumanLlm::complete`]'s chat message list: the node's
/// `messages` array (when non-empty) or its `prompt` string as a single user
/// message, with up to two leading messages prepended in this exact order
/// when present — `input_context` (the upstream data, see
/// [`input_context_block`]'s doc for why this exists) first, then the
/// structured-output steering instruction — so a model reading the
/// conversation top-to-bottom sees "here is your data" before "here is how to
/// format your answer". `input_context` is prepended as a **user**-role
/// message rather than `system`: it's untrusted upstream data (an
/// email/webhook payload, a prior node's output, …), and giving attacker-
/// influenced content system-role authority would let a crafted payload
/// masquerade as host instructions. The structured-output steering message
/// stays `system` — that instruction is ours, not upstream data. Pulled out
/// as its own pure function (rather than inlined in `complete`) so the
/// prepend order is unit-testable without a real provider/network call.
pub(crate) fn build_completion_messages(request: &Value) -> Vec<ChatMessage> {
    let mut messages: Vec<ChatMessage> = match request.get("messages").and_then(Value::as_array) {
        Some(entries) if !entries.is_empty() => entries
            .iter()
            .filter_map(|entry| {
                let content = entry.get("content").and_then(Value::as_str)?.to_string();
                let role = entry.get("role").and_then(Value::as_str).unwrap_or("user");
                Some(match role {
                    "system" => ChatMessage::system(content),
                    "assistant" => ChatMessage::assistant(content),
                    "tool" => ChatMessage::tool(content),
                    _ => ChatMessage::user(content),
                })
            })
            .collect(),
        _ => {
            let prompt = request
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or_default();
            vec![ChatMessage::user(prompt)]
        }
    };

    // Built as a separate prelude (rather than two `messages.insert(0, …)`
    // calls) specifically to guarantee `input_context` lands ahead of the
    // structured-output steering message regardless of which is present.
    let mut prelude: Vec<ChatMessage> = Vec::new();
    if let Some(block) = input_context_block(request) {
        prelude.push(ChatMessage::user(block));
    }
    if structured_output_requested(request) {
        let mut instruction = "Respond with a single JSON object only — no prose, no markdown \
                               code fences."
            .to_string();
        if let Some(schema) = request
            .get("output_parser")
            .and_then(|p| p.get("schema"))
            .filter(|s| !s.is_null())
        {
            instruction.push_str(&format!(
                " The object must match this JSON Schema:\n{schema}"
            ));
        }
        prelude.push(ChatMessage::system(instruction));
    }

    if !prelude.is_empty() {
        messages.splice(0..0, prelude);
    }
    messages
}

/// Best-effort parse of an LLM completion as structured JSON.
///
/// Accepts a bare JSON object/array or one wrapped in a markdown code fence
/// (```json … ``` or ``` … ```). Returns `None` for anything that doesn't
/// parse to an object or array — scalars pass through the legacy `{text}`
/// shape instead, since `item.<field>` addressing is meaningless on them.
pub(crate) fn parse_llm_json(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    let candidate = match trimmed.strip_prefix("```") {
        Some(rest) => {
            let rest = rest.strip_prefix("json").unwrap_or(rest);
            match rest.rsplit_once("```") {
                Some((inner, _)) => inner.trim(),
                None => trimmed,
            }
        }
        None => trimmed,
    };
    let parsed = serde_json::from_str::<Value>(candidate).ok()?;
    matches!(parsed, Value::Object(_) | Value::Array(_)).then_some(parsed)
}

/// Find and parse a fenced JSON block (```json … ``` or ``` … ```) anywhere
/// in `text`, not just when the whole text starts with it. Returns `None` when
/// no fenced block parses to an object or array.
pub(crate) fn extract_fenced_json_block(text: &str) -> Option<Value> {
    let text = text.trim();
    // Look for the first opening ``` fence
    let fence_start = text.find("```")?;
    let after_fence = text[fence_start + 3..].trim();
    // Skip optional "json" after the opening fence
    let content = after_fence
        .strip_prefix("json")
        .unwrap_or(after_fence)
        .trim();
    // Find the *last* closing ``` (preferring the outermost fence, which
    // matches how Markdown renderers treat nested fences — the last ``` is
    // the one that closes the block the LLM opened).
    let close = content.rfind("```")?;
    let inner = content[..close].trim();
    let parsed = serde_json::from_str::<Value>(inner).ok()?;
    matches!(parsed, Value::Object(_) | Value::Array(_)).then_some(parsed)
}

/// Find and parse the first balanced `{…}` or `[…]` span in `text`. Walks
/// through the text byte by byte tracking brace depth, skipping JSON string
/// literals and their escapes so braces inside values cannot close the span.
pub(crate) fn extract_balanced_json(text: &str) -> Option<Value> {
    let text = text.trim();
    let bytes = text.as_bytes();
    let len = bytes.len();
    for start in 0..len {
        let open_byte = bytes[start];
        let (open, close) = match open_byte {
            b'{' => (b'{', b'}'),
            b'[' => (b'[', b']'),
            _ => continue,
        };
        let mut depth = 0u32;
        let mut in_string = false;
        let mut escaped = false;
        for end in start..len {
            let b = bytes[end];
            if in_string {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'"' {
                    in_string = false;
                }
                continue;
            }
            if b == b'"' {
                in_string = true;
                continue;
            }
            if b == open {
                depth = depth.checked_add(1)?;
            } else if b == close {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    // Found a balanced span — try to parse it.
                    let candidate = &text[start..=end];
                    if let Ok(parsed) = serde_json::from_str::<Value>(candidate) {
                        if matches!(parsed, Value::Object(_) | Value::Array(_)) {
                            return Some(parsed);
                        }
                    }
                    // Span didn't parse; continue scanning from the position
                    // after this false-positive open byte.
                    break;
                }
            }
        }
    }
    None
}

/// Apply the shared ordered extraction chain for structured model output.
pub(crate) fn extract_structured_json(text: &str) -> Option<Value> {
    parse_llm_json(text)
        .or_else(|| extract_fenced_json_block(text))
        .or_else(|| extract_balanced_json(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn backtick_run_reports_longest_sequence() {
        assert_eq!(longest_backtick_run("a``b````c"), 4);
        assert_eq!(longest_backtick_run("plain"), 0);
    }

    #[test]
    fn fenced_json_can_be_embedded_in_prose() {
        assert_eq!(
            extract_fenced_json_block("before ```json\n{\"ok\":true}\n``` after"),
            Some(json!({"ok": true}))
        );
    }

    #[test]
    fn balanced_json_ignores_delimiters_and_escapes_inside_strings() {
        assert_eq!(
            extract_balanced_json(r#"before {"text":"} and \"quoted\"","ok":true} after"#),
            Some(json!({"text": "} and \"quoted\"", "ok": true}))
        );
        assert_eq!(extract_balanced_json("no structured value"), None);
    }
}

/// Select the model an `agent` node completion actually runs on.
///
/// `resolved_model` is what [`create_chat_provider`] returned for the node's
/// mapped workload role. A node may instead pin a **raw/BYOK** model id
/// (e.g. `claude-opus-4`) that [`role_for_model_tier`] collapsed to the `chat`
/// role — in that case the pinned id, not the role default, is the model the
/// user selected, so it is forwarded verbatim (issue #4598). Managed tiers and
/// every `hint:*` alias fall through to `resolved_model` unchanged.
pub(crate) fn resolve_completion_model(node_model: Option<&str>, resolved_model: String) -> String {
    match node_model {
        Some(pinned) if is_raw_passthrough_model(pinned) => {
            tracing::debug!(
                target: "flows",
                raw_model = pinned,
                "[flows] llm.complete: forwarding raw/BYOK node model verbatim (not a managed tier)"
            );
            pinned.to_string()
        }
        _ => resolved_model,
    }
}
