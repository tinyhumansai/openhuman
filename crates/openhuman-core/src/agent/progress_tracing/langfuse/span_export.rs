//! Span-tree export: converts the in-memory [`TraceSpan`] tree into a Langfuse
//! ingestion batch (trace-create + one observation per span, usage promoted to
//! `generation` observations) and pushes it to the backend proxy.

use serde_json::{json, Map, Value};

use crate::api::jwt::bearer_authorization_value;
use crate::config::Config;
use crate::security::credentials::session_support::direct_backend_credential;

use super::ingestion_batch::{iso_millis, new_event_id};
use super::{environment_for_base, ingestion_url, skip_push, LOG_TARGET, PUSH_TIMEOUT};
use super::{SpanStatus, TraceSpan};

/// Langfuse observation level for a span status. Only `Error` is elevated so
/// failed tool calls / turns surface in the Langfuse UI.
fn level_for(status: SpanStatus) -> &'static str {
    match status {
        SpanStatus::Error => "ERROR",
        SpanStatus::Ok | SpanStatus::Unset => "DEFAULT",
    }
}

/// Build the Langfuse `metadata` object from the span's (secret-free)
/// attributes plus its structured kind.
fn langfuse_metadata(span: &TraceSpan) -> Value {
    let mut map = Map::new();
    for (key, value) in &span.attributes {
        map.insert(key.clone(), value.clone());
    }
    if let Ok(kind) = serde_json::to_value(span.kind) {
        map.insert("kind".to_string(), kind);
    }
    Value::Object(map)
}

/// Convert finished spans into a Langfuse `/api/public/ingestion` batch payload:
/// a single `trace-create` for the shared trace id followed by one
/// `span-create` observation per span. Field names are Langfuse's camelCase
/// (`traceId`, `startTime`, `parentObservationId`); timestamps are ISO strings.
/// `environment` lands as the trace's top-level Langfuse environment.
pub(crate) fn spans_to_langfuse_batch(
    spans: &[TraceSpan],
    include_content: bool,
    environment: &str,
) -> Value {
    let mut batch: Vec<Value> = Vec::with_capacity(spans.len() + 1);

    // One trace-create for the run, keyed by the shared trace id. Prefer the
    // root (parentless) span for the trace name/start; fall back to the first.
    if let Some(root) = spans
        .iter()
        .find(|s| s.parent_span_id.is_none())
        .or_else(|| spans.first())
    {
        let mut trace_body = json!({
            "id": root.trace_id,
            "name": root.name,
            "timestamp": iso_millis(root.start_unix_ms),
            // Top-level Langfuse trace fields (not metadata): deployment
            // environment + the core release that produced the trace.
            "environment": environment,
            "release": env!("CARGO_PKG_VERSION"),
        });
        // Attribute the trace to the user and group per-turn traces under the
        // conversation via Langfuse's native `userId`/`sessionId` (read from the
        // turn span's stamped attributes). Every trace gets a sessionId: the
        // stamped thread.id when present, else the trace id itself.
        if let Some(user) = root.attributes.get("user.id").and_then(Value::as_str) {
            trace_body["userId"] = json!(user);
        }
        let session = root
            .attributes
            .get("thread.id")
            .and_then(Value::as_str)
            .unwrap_or(root.trace_id.as_str());
        trace_body["sessionId"] = json!(session);
        // Trace-level metadata: transport client, agent attribution, run
        // origin, and the core version — all secret-free identifiers.
        let mut trace_meta = Map::new();
        for key in ["client.id", "agent.id", "channel.source", "gen_ai.provider"] {
            if let Some(value) = root.attributes.get(key) {
                trace_meta.insert(key.to_string(), value.clone());
            }
        }
        trace_meta.insert("app.version".to_string(), json!(env!("CARGO_PKG_VERSION")));
        // Run-type tags so traces filter by kind of run in the Langfuse UI:
        // `run:<type>` (interactive_chat / autonomous_task /
        // channel_inbound) plus `source:<channel.source>` when known.
        let mut tags: Vec<String> = Vec::with_capacity(2);
        if let Some(run_type) = root.attributes.get("run.type").and_then(Value::as_str) {
            tags.push(format!("run:{run_type}"));
            trace_meta.insert("run_type".to_string(), json!(run_type));
        }
        if let Some(source) = root
            .attributes
            .get("channel.source")
            .and_then(Value::as_str)
        {
            tags.push(format!("source:{source}"));
        }
        if !tags.is_empty() {
            trace_body["tags"] = json!(tags);
        }
        trace_body["metadata"] = Value::Object(trace_meta);
        // Trace-level input/output mirror the root turn span's content so the
        // Langfuse trace list shows the prompt/reply at a glance. Same opt-out
        // gate as the observations.
        if include_content {
            if let Some(input) = &root.input {
                trace_body["input"] = input.clone();
            }
            if let Some(output) = &root.output {
                trace_body["output"] = output.clone();
            }
        }
        batch.push(json!({
            "id": new_event_id(),
            "type": "trace-create",
            "timestamp": iso_millis(root.start_unix_ms),
            "body": trace_body,
        }));
    }

    for span in spans {
        let mut body = json!({
            "id": span.span_id,
            "traceId": span.trace_id,
            "name": span.name,
            "startTime": iso_millis(span.start_unix_ms),
            "metadata": langfuse_metadata(span),
            "level": level_for(span.status),
        });
        if let Some(end) = span.end_unix_ms {
            body["endTime"] = json!(iso_millis(end));
        }
        if let Some(parent) = &span.parent_span_id {
            body["parentObservationId"] = json!(parent);
        }
        // Failed spans surface their captured error text as the Langfuse
        // statusMessage (the collector already truncated + content-gated it).
        if let Some(message) = span.attributes.get("error.message").and_then(Value::as_str) {
            body["statusMessage"] = json!(message);
        }
        // Prompt/reply content is transmitted only when the caller opted in
        // (`observability.agent_tracing.capture_content`); otherwise it never
        // leaves the device even though it may sit on the in-memory span.
        if include_content {
            if let Some(input) = &span.input {
                body["input"] = input.clone();
            }
            if let Some(output) = &span.output {
                body["output"] = output.clone();
            }
        }
        // A span carrying `gen_ai.usage.*` attributes (today only the root turn
        // span) is emitted as a Langfuse `generation` so the UI renders native
        // token usage + cost instead of burying them in metadata. Token counts
        // and cost are non-PII, so this promotion is unconditional.
        let event_type = if apply_usage_fields(&mut body, span) {
            "generation-create"
        } else {
            "span-create"
        };
        batch.push(json!({
            "id": new_event_id(),
            "type": event_type,
            "timestamp": iso_millis(span.start_unix_ms),
            "body": body,
        }));
    }

    json!({ "batch": batch })
}

/// Promote a span's `gen_ai.usage.*` / `gen_ai.request.model` attributes into
/// Langfuse's native `model` / `usageDetails` / `costDetails` fields so the
/// trace surfaces real token counts and cost (Langfuse only renders these on
/// `generation` observations). Returns `true` when usage was found, so the
/// caller emits the span as a `generation-create`. Only token/cost figures are
/// touched — never prompt text or PII.
fn apply_usage_fields(body: &mut Value, span: &TraceSpan) -> bool {
    let attrs = &span.attributes;
    let input = attrs
        .get("gen_ai.usage.input_tokens")
        .and_then(Value::as_u64);
    let output = attrs
        .get("gen_ai.usage.output_tokens")
        .and_then(Value::as_u64);
    if input.is_none() && output.is_none() {
        return false;
    }
    let input = input.unwrap_or(0);
    let output = output.unwrap_or(0);
    let cached = attrs
        .get("gen_ai.usage.cached_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    // #4454: `input_tokens` is INCLUSIVE of cached prompt tokens (cost.rs treats
    // cached as a subset of input). Langfuse sums `usageDetails` components as
    // disjoint buckets, so emit the NON-cached input (input - cached) — the
    // components (non_cached_input + cache_read + output) are then disjoint and
    // reconcile to `total` = input_tokens + output_tokens.
    let non_cached_input = input.saturating_sub(cached);
    let mut usage = Map::new();
    usage.insert("input".to_string(), json!(non_cached_input));
    usage.insert("output".to_string(), json!(output));
    usage.insert("total".to_string(), json!(input.saturating_add(output)));
    // Cache reads always flow into usageDetails (0 included) so the figure is
    // explicit rather than absent when no cache was hit.
    usage.insert("cache_read_input_tokens".to_string(), json!(cached));
    // Reasoning + cache-write tokens ride along whenever the span carries them
    // (the collector stamps them when > 0). Langfuse accepts arbitrary
    // usageDetails keys.
    if let Some(reasoning) = attrs
        .get("gen_ai.usage.reasoning_tokens")
        .and_then(Value::as_u64)
    {
        usage.insert("reasoning_tokens".to_string(), json!(reasoning));
    }
    if let Some(cache_write) = attrs
        .get("gen_ai.usage.cache_creation_tokens")
        .and_then(Value::as_u64)
    {
        usage.insert(
            "cache_creation_input_tokens".to_string(),
            json!(cache_write),
        );
    }
    body["usageDetails"] = Value::Object(usage);
    if let Some(model) = attrs.get("gen_ai.request.model").and_then(Value::as_str) {
        body["model"] = json!(model);
    }
    if let Some(cost) = attrs.get("gen_ai.usage.cost_usd").and_then(Value::as_f64) {
        body["costDetails"] = json!({ "total": cost });
    }
    true
}

/// Push `spans` to the co-hosted Langfuse server. Resolves the endpoint from the
/// current backend host and authenticates with the live session bearer. Returns
/// `Err` (for the caller to log + fall back) when there is no live session, the
/// host is unresolvable, the request fails, or Langfuse rejects the batch.
pub(crate) async fn push_spans(config: &Config, spans: &[TraceSpan]) -> Result<(), String> {
    if spans.is_empty() {
        return Ok(());
    }
    let url = ingestion_url(config);
    // Ahead of the URL check, the session lookup and the request: a skipped
    // environment must cost nothing per turn. An unresolvable URL lands in
    // `environment_for_base`'s catch-all, which is `external` — so a garbage
    // host skips rather than erroring, which is the right way round.
    let environment = environment_for_base(&url);
    if skip_push(environment) {
        return Ok(());
    }
    if !url.starts_with("http") {
        return Err(format!(
            "could not resolve Langfuse ingestion URL from backend host (got {url:?})"
        ));
    }
    // No TinyHumans connection, or no usable credential (signed out, offline
    // local session): a configured state, so skip quietly rather than failing
    // every turn's push.
    let Some(credential) = direct_backend_credential(config, "langfuse span push") else {
        return Ok(());
    };
    let token = credential.into_secret();
    let include_content = config.observability.agent_tracing.capture_content;
    let batch = spans_to_langfuse_batch(spans, include_content, environment);
    let span_count = spans.len();

    tracing::debug!(
        target: LOG_TARGET,
        "[agent-tracing] pushing {span_count} spans to Langfuse at {url}"
    );

    // `ingestion_url` resolves to the backend's own Langfuse proxy route on the
    // backend host, authenticated with a TinyHumans session token — backend
    // traffic, so it carries the product identity. This is a bare
    // `reqwest::Client`, not `BackendOAuthClient`'s, so nothing is inherited
    // from that path's default headers; see [`crate::api::product`].
    let (product_header, product_value) = crate::api::product::product_identity_header();
    let response = reqwest::Client::new()
        .post(&url)
        .header(
            reqwest::header::AUTHORIZATION,
            bearer_authorization_value(&token),
        )
        .header(product_header, product_value)
        .timeout(PUSH_TIMEOUT)
        .json(&batch)
        .send()
        .await
        .map_err(|err| format!("POST {url} failed: {err}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let excerpt: String = body.chars().take(200).collect();
        return Err(format!("Langfuse ingestion returned {status}: {excerpt}"));
    }
    // Langfuse returns 207 Multi-Status even when individual events are rejected
    // — the failures live in the response `errors` array, not the HTTP status.
    // Surface them (a partial rejection is logged but never fails the turn).
    let rejected = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.get("errors").and_then(Value::as_array).cloned())
        .filter(|errs| !errs.is_empty());
    if let Some(errs) = rejected {
        let excerpt: String = serde_json::to_string(&errs)
            .unwrap_or_default()
            .chars()
            .take(400)
            .collect();
        tracing::warn!(
            target: LOG_TARGET,
            "[agent-tracing] Langfuse ({status}) rejected {} of {span_count} span event(s): {excerpt}",
            errs.len()
        );
    } else {
        tracing::debug!(
            target: LOG_TARGET,
            "[agent-tracing] pushed {span_count} spans to Langfuse ({status})"
        );
    }
    Ok(())
}
