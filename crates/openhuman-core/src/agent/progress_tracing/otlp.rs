//! Bounded OTLP/HTTP JSON export through the authenticated backend proxy.

use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::langfuse::{environment_for_base, ingestion_url, skip_push};
use super::types::{SpanKind, SpanStatus, TraceSpan};
use crate::api::jwt::bearer_authorization_value;
use crate::config::Config;
use crate::security::credentials::session_support::direct_backend_credential;

// The backend JSON parser caps requests at 10 MiB. A generation can carry a
// 200 KiB structured prompt, so use a much smaller transport batch.
const MAX_SPANS_PER_REQUEST: usize = 20;
const MAX_REQUEST_BYTES: usize = 8_000_000;

fn hex_id(source: &str, bytes: usize) -> String {
    let digest = Sha256::digest(source.as_bytes());
    digest[..bytes]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn attribute(key: &str, value: impl Into<String>) -> Value {
    json!({ "key": key, "value": { "stringValue": value.into() } })
}

fn json_attribute(key: &str, value: &Value) -> Value {
    attribute(key, value.to_string())
}

fn string_attr<'a>(span: &'a TraceSpan, key: &str) -> Option<&'a str> {
    span.attributes.get(key).and_then(Value::as_str)
}

fn usage_details(span: &TraceSpan) -> Option<Value> {
    let input = span.attributes.get("gen_ai.usage.input_tokens")?.as_u64()?;
    let output = span
        .attributes
        .get("gen_ai.usage.output_tokens")?
        .as_u64()?;
    let cached = span
        .attributes
        .get("gen_ai.usage.cached_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some(json!({
        "input": input.saturating_sub(cached),
        "cache_read_input_tokens": cached,
        "output": output,
        "total": input.saturating_add(output),
    }))
}

fn message_content(payload: &Value) -> String {
    if let Some(content) = payload.get("content").and_then(Value::as_str) {
        return content.to_string();
    }
    payload
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| {
                    if let Some(text) = block.as_str() {
                        return Some(text.to_string());
                    }
                    let object = block.as_object()?;
                    if let Some(text) = object.get("text").and_then(Value::as_str) {
                        return Some(text.to_string());
                    }
                    if let Some(value) = object.get("json") {
                        return Some(value.to_string());
                    }
                    object.keys().next().map(|kind| format!("[{kind} content]"))
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn normalize_message(role: &str, payload: &Value) -> Value {
    let mut message = json!({ "role": role, "content": message_content(payload) });
    if role == "tool" {
        if let Some(id) = payload.get("tool_call_id") {
            message["tool_call_id"] = id.clone();
        }
    }
    if role == "assistant" {
        if let Some(calls) = payload.get("tool_calls").and_then(Value::as_array) {
            if !calls.is_empty() {
                message["tool_calls"] = Value::Array(
                    calls
                        .iter()
                        .map(|call| {
                            let arguments = call.get("arguments").cloned().unwrap_or(Value::Null);
                            json!({
                                "id": call.get("id").cloned().unwrap_or(Value::Null),
                                "type": "function",
                                "function": {
                                    "name": call.get("name").cloned().unwrap_or(Value::Null),
                                    "arguments": if let Some(raw) = arguments.as_str() {
                                        raw.to_string()
                                    } else {
                                        arguments.to_string()
                                    },
                                },
                            })
                        })
                        .collect(),
                );
            }
        }
        let reasoning = payload
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| block.get("thinking")?.get("text")?.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if !reasoning.is_empty() {
            message["reasoning"] = json!(reasoning);
        }
    }
    message
}

fn normalize_generation_io(value: &Value, output: bool) -> Value {
    if let Some(messages) = value.as_array() {
        return Value::Array(
            messages
                .iter()
                .filter_map(|message| {
                    let object = message.as_object()?;
                    if object.get("role").and_then(Value::as_str).is_some() {
                        return Some(message.clone());
                    }
                    for role in ["system", "user", "assistant", "tool"] {
                        if let Some(payload) = object.get(role) {
                            return Some(normalize_message(role, payload));
                        }
                    }
                    None
                })
                .collect(),
        );
    }
    if output && value.get("content").is_some() {
        return normalize_message("assistant", value);
    }
    value.clone()
}

/// Journal replay puts serialized model payloads on the child root. Replace
/// those with the delegated user task and last assistant answer for the trace
/// overview; the full per-call messages remain on generation observations.
pub(super) fn prepare_subagent_root(spans: &mut [TraceSpan]) {
    let first_input = spans
        .iter()
        .find(|span| span.kind == SpanKind::Generation)
        .and_then(|span| span.input.as_ref())
        .map(|input| normalize_generation_io(input, false))
        .and_then(|messages| {
            messages.as_array().and_then(|messages| {
                messages
                    .iter()
                    .rev()
                    .find(|message| message["role"] == "user")
                    .and_then(|message| message.get("content"))
                    .cloned()
            })
        });
    let last_output = spans
        .iter()
        .rev()
        .find(|span| span.kind == SpanKind::Generation)
        .and_then(|span| span.output.as_ref())
        .map(|output| normalize_generation_io(output, true))
        .and_then(|message| message.get("content").cloned());
    if let Some(root) = spans.iter_mut().find(|span| span.parent_span_id.is_none()) {
        if first_input.is_some() {
            root.input = first_input;
        }
        if last_output.is_some() {
            root.output = last_output;
        }
    }
}

fn compact_internal_searches(spans: &[TraceSpan]) -> Vec<TraceSpan> {
    let mut output = Vec::with_capacity(spans.len());
    let mut groups: BTreeMap<String, Vec<&TraceSpan>> = BTreeMap::new();
    for span in spans {
        if span.name == "tool.tool_search" && span.status != SpanStatus::Error {
            groups
                .entry(span.parent_span_id.clone().unwrap_or_default())
                .or_default()
                .push(span);
        } else {
            output.push(span.clone());
        }
    }
    for (parent, group) in groups {
        if group.len() == 1 {
            output.push((*group[0]).clone());
            continue;
        }
        let mut summary = (*group[0]).clone();
        summary.span_id = format!("{}:tool-search-summary", parent);
        summary.name = "tool.search_catalog".to_string();
        summary.start_unix_ms = group
            .iter()
            .map(|span| span.start_unix_ms)
            .min()
            .unwrap_or(0);
        summary.end_unix_ms = group.iter().filter_map(|span| span.end_unix_ms).max();
        summary.input = None;
        summary.output = None;
        summary.attributes.clear();
        summary
            .attributes
            .insert("tool.call_count".to_string(), json!(group.len()));
        output.push(summary);
    }
    output.sort_by_key(|span| span.start_unix_ms);
    output
}

fn without_child_run_details(spans: &[TraceSpan]) -> Vec<TraceSpan> {
    let by_id: HashMap<&str, &TraceSpan> = spans
        .iter()
        .map(|span| (span.span_id.as_str(), span))
        .collect();
    spans
        .iter()
        .filter(|span| {
            let mut parent = span.parent_span_id.as_deref();
            while let Some(id) = parent {
                let Some(ancestor) = by_id.get(id) else {
                    break;
                };
                if ancestor.kind == SpanKind::Subagent {
                    return false;
                }
                parent = ancestor.parent_span_id.as_deref();
            }
            true
        })
        .cloned()
        .collect()
}

fn span_to_otlp(span: &TraceSpan, root: &TraceSpan, environment: &str) -> Value {
    let mut attrs = vec![
        attribute(
            "langfuse.observation.type",
            match span.kind {
                SpanKind::Turn | SpanKind::Subagent => "agent",
                SpanKind::Tool => "tool",
                SpanKind::Generation => "generation",
                SpanKind::Iteration | SpanKind::SubagentIteration => "span",
            },
        ),
        attribute("langfuse.trace.name", root.name.clone()),
        attribute(
            "langfuse.trace.metadata.openhuman_trace_id",
            root.trace_id.clone(),
        ),
        attribute("langfuse.environment", environment),
        attribute("langfuse.version", env!("CARGO_PKG_VERSION")),
        attribute("langfuse.release", env!("CARGO_PKG_VERSION")),
        attribute(
            "langfuse.session.id",
            string_attr(root, "thread.id")
                .unwrap_or(root.trace_id.as_str())
                .to_string(),
        ),
    ];
    if let Some(run_type) = string_attr(root, "run.type") {
        attrs.push(attribute("langfuse.trace.metadata.run_type", run_type));
        let mut tags = vec![format!("run:{run_type}")];
        if let Some(source) = string_attr(root, "channel.source") {
            tags.push(format!("source:{source}"));
        }
        attrs.push(json!({ "key": "langfuse.trace.tags", "value": {
            "arrayValue": { "values": tags.into_iter().map(|tag| json!({ "stringValue": tag })).collect::<Vec<_>>() }
        } }));
    }
    for key in [
        "agent.id",
        "channel.source",
        "run_id",
        "parent_run_id",
        "root_run_id",
    ] {
        if let Some(value) = string_attr(root, key) {
            attrs.push(attribute(
                &format!("langfuse.trace.metadata.{}", key.replace('.', "_")),
                value,
            ));
        }
    }
    if let Some(input) = &span.input {
        let input = if span.kind == SpanKind::Generation {
            normalize_generation_io(input, false)
        } else {
            input.clone()
        };
        attrs.push(json_attribute("langfuse.observation.input", &input));
    }
    if let Some(output) = &span.output {
        let output = if span.kind == SpanKind::Generation {
            normalize_generation_io(output, true)
        } else {
            output.clone()
        };
        attrs.push(json_attribute("langfuse.observation.output", &output));
    }
    if span.status == SpanStatus::Error {
        attrs.push(attribute("langfuse.observation.level", "ERROR"));
        if let Some(message) = string_attr(span, "error.message") {
            attrs.push(attribute("langfuse.observation.status_message", message));
        }
    }
    for (key, value) in &span.attributes {
        if key.starts_with("gen_ai.usage.") || key == "gen_ai.request.model" {
            continue;
        }
        attrs.push(json_attribute(
            &format!("langfuse.observation.metadata.{}", key.replace('.', "_")),
            value,
        ));
    }
    if span.kind == SpanKind::Generation {
        if let Some(model) = string_attr(span, "gen_ai.request.model") {
            attrs.push(attribute("langfuse.observation.model.name", model));
        }
        if let Some(provider) = string_attr(span, "gen_ai.provider") {
            attrs.push(attribute(
                "langfuse.observation.metadata.provider",
                provider,
            ));
        }
        if provider_is_openrouter(span) {
            attrs.push(attribute(
                "langfuse.observation.metadata.model_route",
                "openrouter",
            ));
        }
        if let Some(usage) = usage_details(span) {
            attrs.push(json_attribute("langfuse.observation.usage_details", &usage));
        }
        if let Some(cost) = span
            .attributes
            .get("gen_ai.usage.cost_usd")
            .and_then(Value::as_f64)
            .filter(|cost| *cost > 0.0)
        {
            attrs.push(json_attribute(
                "langfuse.observation.cost_details",
                &json!({ "total": cost }),
            ));
        }
    } else if span.span_id == root.span_id {
        for key in [
            "gen_ai.usage.input_tokens",
            "gen_ai.usage.output_tokens",
            "gen_ai.usage.cost_usd",
        ] {
            if let Some(value) = span.attributes.get(key) {
                attrs.push(json_attribute(
                    &format!(
                        "langfuse.observation.metadata.run_total_{}",
                        key.rsplit('.').next().unwrap_or(key)
                    ),
                    value,
                ));
            }
        }
    }
    let mut body = json!({
        "traceId": hex_id(&span.trace_id, 16),
        "spanId": hex_id(&span.span_id, 8),
        "name": span.name,
        "startTimeUnixNano": span.start_unix_ms.saturating_mul(1_000_000).to_string(),
        "endTimeUnixNano": span.end_unix_ms.unwrap_or(span.start_unix_ms).saturating_mul(1_000_000).to_string(),
        "attributes": attrs,
    });
    if let Some(parent) = &span.parent_span_id {
        body["parentSpanId"] = json!(hex_id(parent, 8));
    }
    body
}

fn provider_is_openrouter(span: &TraceSpan) -> bool {
    string_attr(span, "gen_ai.provider") == Some("openrouter")
        || string_attr(span, "gen_ai.request.model")
            .is_some_and(|model| model.contains("openrouter/"))
}

pub(super) fn otlp_requests(spans: &[TraceSpan], environment: &str) -> Vec<Value> {
    let Some(root) = spans.iter().find(|span| span.parent_span_id.is_none()) else {
        return Vec::new();
    };
    // Child runs have their own journal-backed trace under the same session.
    // Keep their parent placeholder here, but export each child model/tool call
    // only once so Langfuse usage metrics do not count it twice.
    let visible = without_child_run_details(spans);
    let compacted = compact_internal_searches(&visible);
    let make_request = |spans: Vec<Value>| {
        json!({ "resourceSpans": [{
            "resource": { "attributes": [attribute("service.name", "openhuman")] },
            "scopeSpans": [{
                "scope": { "name": "openhuman.agent" },
                "spans": spans,
            }],
        }] })
    };
    let mut requests = Vec::new();
    let mut batch = Vec::new();
    let mut bytes = 256;
    for span in &compacted {
        let converted = span_to_otlp(span, root, environment);
        let span_bytes = converted.to_string().len();
        if !batch.is_empty()
            && (batch.len() >= MAX_SPANS_PER_REQUEST || bytes + span_bytes > MAX_REQUEST_BYTES)
        {
            requests.push(make_request(std::mem::take(&mut batch)));
            bytes = 256;
        }
        bytes += span_bytes;
        batch.push(converted);
    }
    if !batch.is_empty() {
        requests.push(make_request(batch));
    }
    requests
}

pub(super) async fn push_spans(config: &Config, spans: &[TraceSpan]) -> Result<(), String> {
    if spans.is_empty() {
        return Ok(());
    }
    let legacy_url = ingestion_url(config);
    let url = legacy_url.replace("/langfuse/ingestion", "/langfuse/otel/v1/traces");
    let environment = environment_for_base(&url);
    if skip_push(environment) {
        return Ok(());
    }
    if !url.starts_with("http") {
        return Err("Langfuse backend proxy URL is unavailable".to_string());
    }
    // No TinyHumans connection, or no usable credential (signed out, offline
    // local session): a configured state, so skip quietly rather than failing
    // every turn's push.
    let Some(credential) = direct_backend_credential(config, "langfuse otlp push") else {
        return Ok(());
    };
    let token = credential.into_secret();
    let (product_header, product_value) = crate::api::product::product_identity_header();
    let client = reqwest::Client::new();
    for payload in otlp_requests(spans, environment) {
        let response = client
            .post(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                bearer_authorization_value(&token),
            )
            .header(product_header.clone(), product_value.clone())
            .timeout(std::time::Duration::from_secs(10))
            .json(&payload)
            .send()
            .await
            .map_err(|err| format!("Langfuse OTLP transport failed: {err}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("Langfuse OTLP proxy returned {status}"));
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if body["partialSuccess"]["rejectedSpans"]
            .as_u64()
            .unwrap_or(0)
            > 0
            || body["partialSuccess"]["errorMessage"]
                .as_str()
                .is_some_and(|message| !message.is_empty())
        {
            return Err("Langfuse OTLP proxy partially rejected spans".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "otlp_tests.rs"]
mod tests;
