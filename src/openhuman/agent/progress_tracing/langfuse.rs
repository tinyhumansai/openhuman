//! Langfuse ingestion exporter for agent trace spans (issue #4249 follow-up).
//!
//! When `[observability.agent_tracing]` has `enabled = true` and
//! `backend = "langfuse"`, a completed run's spans are POSTed to the OpenHuman
//! backend's Langfuse **proxy** route, `/telemetry/langfuse/ingestion`, derived
//! from the **current backend hostname** (`effective_backend_api_url`). The
//! request reuses the OpenHuman **session bearer** — the same auth every other
//! backend call carries; the backend authenticates that JWT, injects the
//! Langfuse project keys server-side, and forwards the batch to Langfuse's real
//! `/api/public/ingestion` (backend `src/services/langfuseProxy.ts`). Clients
//! never hold Langfuse keys and never hit `/api/public/ingestion` directly.
//!
//! Best-effort: any failure is logged and swallowed by the caller so tracing
//! never breaks a turn. Spans always carry metadata (names, kinds, timings,
//! and non-PII token/cost figures — the latter promoted into Langfuse's native
//! `usageDetails`/`costDetails`). Prompt/reply text and truncated tool I/O
//! ride along only while `observability.agent_tracing.capture_content` is on;
//! with the default off, content is withheld and export stays metadata-only.

use std::borrow::Cow;
use std::time::Duration;

use serde_json::{json, Map, Value};
use tinyagents::harness::events::AgentEvent;
use tinyagents::harness::observability::{AgentObservation, LangfuseClient, LangfuseTraceConfig};

use crate::api::config::effective_backend_api_url;
use crate::api::jwt::bearer_authorization_value;
use crate::openhuman::config::Config;
use crate::openhuman::credentials::session_support::require_live_session_token;
use crate::openhuman::session_db::run_ledger::RunTelemetry;

use super::{SpanStatus, TraceContext, TraceSpan};

const LOG_TARGET: &str = "agent-tracing::langfuse";
/// Backend proxy route for Langfuse ingestion (relative to the backend origin).
/// The backend authenticates the caller's session JWT, injects the Langfuse
/// project keys, and forwards to Langfuse's real `/api/public/ingestion` — so
/// clients POST here, NOT to `/api/public/ingestion` (which is unexposed and
/// carries no keys).
const INGESTION_PATH: &str = "/telemetry/langfuse/ingestion";
/// Cap the push so a slow/hung Langfuse never stalls run teardown.
const PUSH_TIMEOUT: Duration = Duration::from_secs(10);

/// Resolve the Langfuse ingestion URL from the current backend host. Joins the
/// proxy path onto [`effective_backend_api_url`] — the exact base-server
/// resolution every other backend call uses — via the canonical
/// [`crate::api::config::api_url`] helper, which replaces any path the base
/// carried with the given absolute path. So the host always matches wherever the
/// app's domain calls go (staging, prod, or a custom `api_url` override).
pub(crate) fn ingestion_url(config: &Config) -> String {
    let base = effective_backend_api_url(&config.api_url);
    crate::api::config::api_url(&base, INGESTION_PATH)
}

/// Epoch-milliseconds → RFC 3339 / ISO-8601 string (Langfuse requires ISO
/// timestamps, not epoch integers). Falls back to "now" only if the value is
/// somehow out of range — `start_unix_ms` comes from a monotonic wall clock so
/// this is defensive.
fn iso_millis(unix_ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(unix_ms as i64)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339()
}

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

/// Derive the Langfuse `environment` for a backend base URL. Chosen signal:
/// the resolved backend host is the single existing config-driven fact that
/// distinguishes deployments (there is no NODE_ENV-style flag in the core
/// config) — `staging` in the host → staging, loopback/local → development,
/// anything else → production.
pub(crate) fn environment_for_base(base: &str) -> &'static str {
    let lower = base.to_ascii_lowercase();
    if lower.contains("staging") {
        "staging"
    } else if lower.contains("localhost")
        || lower.contains("127.0.0.1")
        || lower.contains("0.0.0.0")
    {
        "development"
    } else {
        "production"
    }
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
        // `run:<type>` (interactive_chat / autonomous_task / agentbox /
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

/// Fresh per-event id. Langfuse dedupes ingestion events by this id, so it must
/// be unique per event (distinct from the observation/trace id in `body`).
fn new_event_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Enrich `trace_ctx` with the run lineage (`run_id` / `parent_run_id` /
/// `root_run_id`) carried by the run's journalled `observations` (#4657).
///
/// A single export corresponds to one run's observation stream (the journal is
/// read per run id), so every observation shares the same lineage and the first
/// is representative. For a spawned sub-agent that lineage points back at the
/// spawning turn, which is exactly what links the sub-agent's trace to its
/// parent. Returns the context unchanged when there are no observations.
fn trace_ctx_with_run_lineage(
    trace_ctx: &TraceContext,
    observations: &[AgentObservation],
) -> TraceContext {
    let Some(first) = observations.first() else {
        return trace_ctx.clone();
    };
    trace_ctx.clone().with_run_lineage(
        Some(first.run_id.as_str().to_string()),
        first
            .parent_run_id
            .as_ref()
            .map(|id| id.as_str().to_string()),
        Some(first.root_run_id.as_str().to_string()),
    )
}

fn trace_config_from_context(trace_ctx: &TraceContext, environment: &str) -> LangfuseTraceConfig {
    let mut metadata = Map::new();
    if let Some(client_id) = &trace_ctx.client_id {
        metadata.insert("client.id".into(), json!(client_id));
    }
    if let Some(agent_id) = &trace_ctx.agent_id {
        metadata.insert("agent.id".into(), json!(agent_id));
    }
    if let Some(source) = &trace_ctx.channel_source {
        metadata.insert("channel.source".into(), json!(source));
    }
    metadata.insert("run_type".into(), json!(trace_ctx.run_type.as_str()));
    metadata.insert("app.version".into(), json!(env!("CARGO_PKG_VERSION")));
    // Run lineage (#4657): stamp the run/parent/root ids so a spawned sub-agent's
    // trace is navigable from — and threadable under — its parent turn. Omitted
    // keys (e.g. `parent_run_id` for a top-level turn) simply stay absent.
    if let Some(run_id) = &trace_ctx.run_id {
        metadata.insert("run_id".into(), json!(run_id));
    }
    if let Some(parent_run_id) = &trace_ctx.parent_run_id {
        metadata.insert("parent_run_id".into(), json!(parent_run_id));
    }
    if let Some(root_run_id) = &trace_ctx.root_run_id {
        metadata.insert("root_run_id".into(), json!(root_run_id));
    }

    let mut tags = vec![format!("run:{}", trace_ctx.run_type.as_str())];
    if let Some(source) = &trace_ctx.channel_source {
        tags.push(format!("source:{source}"));
    }

    LangfuseTraceConfig {
        trace_id: Some(trace_ctx.session_id.clone()),
        name: Some(match &trace_ctx.agent_id {
            Some(agent_id) => format!("agent.turn:{agent_id}"),
            None => "agent.turn".to_string(),
        }),
        user_id: trace_ctx.user_id.clone(),
        session_id: trace_ctx
            .session_group
            .clone()
            .or_else(|| Some(trace_ctx.session_id.clone())),
        environment: Some(environment.to_string()),
        release: Some(env!("CARGO_PKG_VERSION").to_string()),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        tags,
        metadata: Value::Object(metadata),
    }
}

fn observations_for_export<'a>(
    trace_ctx: &TraceContext,
    observations: &'a [AgentObservation],
) -> Cow<'a, [AgentObservation]> {
    if trace_ctx.capture_content {
        return Cow::Borrowed(observations);
    }

    Cow::Owned(
        observations
            .iter()
            .cloned()
            .map(strip_observation_content)
            .collect(),
    )
}

fn strip_observation_content(mut observation: AgentObservation) -> AgentObservation {
    match &mut observation.event {
        AgentEvent::ModelCompleted { input, output, .. }
        | AgentEvent::ToolCompleted { input, output, .. } => {
            *input = None;
            *output = None;
        }
        _ => {}
    }
    observation
}

fn insert_run_telemetry_generation(payload: &mut Value, telemetry: Option<&RunTelemetry>) -> bool {
    let Some(telemetry) = telemetry else {
        return false;
    };
    if telemetry.input_tokens == 0 && telemetry.output_tokens == 0 && telemetry.cost_usd == 0.0 {
        return false;
    }

    let Some(batch) = payload.get_mut("batch").and_then(Value::as_array_mut) else {
        return false;
    };
    let Some(trace_id) = batch
        .first()
        .and_then(|event| event.get("body"))
        .and_then(|body| body.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return false;
    };
    let start_time = batch
        .first()
        .and_then(|event| event.get("timestamp"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let end_time = batch
        .last()
        .and_then(|event| event.get("timestamp"))
        .and_then(Value::as_str)
        .unwrap_or(start_time.as_str())
        .to_string();

    let non_cached_input = telemetry
        .input_tokens
        .saturating_sub(telemetry.cached_input_tokens);
    let mut body = json!({
        "id": format!("{trace_id}:openhuman-run-telemetry"),
        "traceId": trace_id,
        "name": "run.total",
        "startTime": start_time,
        "endTime": end_time,
        "usageDetails": {
            "input": non_cached_input,
            "output": telemetry.output_tokens,
            "total": telemetry.input_tokens.saturating_add(telemetry.output_tokens),
            "cache_read_input_tokens": telemetry.cached_input_tokens,
        },
        "costDetails": {
            "total": telemetry.cost_usd,
        },
        "metadata": {
            "source": "openhuman.run_telemetry",
            "run_id": telemetry.run_id.as_str(),
            "tool_count": telemetry.tool_count,
        },
    });
    if let Some(model) = &telemetry.model {
        body["model"] = json!(model);
    }
    if let Some(provider) = &telemetry.provider {
        body["metadata"]["provider"] = json!(provider);
    }
    if let Some(error) = &telemetry.error {
        body["level"] = json!("ERROR");
        body["statusMessage"] = json!(error);
    }

    batch.insert(
        1,
        json!({
            "id": new_event_id(),
            "type": "generation-create",
            "timestamp": body["startTime"].clone(),
            "body": body,
        }),
    );
    true
}

/// Push durable journal observations through the tinyagents crate Langfuse
/// exporter. The journal is already redacted before persistence, and this
/// exporter additionally strips model/tool payloads unless `capture_content`
/// is explicitly enabled.
/// Langfuse rejects an ingestion request whose `batch` holds more than 500
/// events (`400 "Langfuse ingestion batch cannot exceed 500 events"`). Large
/// turns — especially ones that spawn sub-agents — routinely exceed this.
const LANGFUSE_MAX_BATCH_EVENTS: usize = 500;

/// Split a `{"batch": [...]}` ingestion payload into multiple payloads, each
/// carrying at most `max` events and preserving any other top-level keys.
///
/// Langfuse dedupes ingestion events by id and resolves each observation to its
/// trace by `traceId`, so delivering one run's events across several requests is
/// safe (the `trace-create` event stays in the first chunk). A payload at or
/// under the limit — or without a `batch` array — passes through unchanged as a
/// single element.
fn split_ingestion_batch(payload: Value, max: usize) -> Vec<Value> {
    let events = match payload.get("batch").and_then(Value::as_array) {
        Some(events) if max > 0 && events.len() > max => events.clone(),
        _ => return vec![payload],
    };
    events
        .chunks(max)
        .map(|chunk| {
            let mut part = payload.clone();
            part["batch"] = Value::Array(chunk.to_vec());
            part
        })
        .collect()
}

pub(crate) async fn push_observations(
    config: &Config,
    trace_ctx: &TraceContext,
    observations: &[AgentObservation],
    run_telemetry: Option<&RunTelemetry>,
) -> Result<(), String> {
    if observations.is_empty() {
        return Ok(());
    }
    let url = ingestion_url(config);
    if !url.starts_with("http") {
        return Err(format!(
            "could not resolve Langfuse ingestion URL from backend host (got {url:?})"
        ));
    }
    let token = require_live_session_token(config)?;
    let environment = environment_for_base(&url);
    // Stamp the run lineage from the run's own observations so a spawned
    // sub-agent's trace links back to its parent turn (#4657).
    let trace_ctx = trace_ctx_with_run_lineage(trace_ctx, observations);
    let trace = trace_config_from_context(&trace_ctx, environment);
    let observation_count = observations.len();
    let observations = observations_for_export(&trace_ctx, observations);

    tracing::debug!(
        target: LOG_TARGET,
        "[agent-tracing] pushing {observation_count} journal observations to Langfuse at {url}"
    );

    let client = LangfuseClient::proxy(url, token)
        .map_err(|err| format!("Langfuse client setup failed: {err}"))?;
    let mut payload = client
        .build_ingestion_batch(trace, observations.as_ref())
        .map_err(|err| format!("Langfuse journal batch build failed: {err}"))?;
    if insert_run_telemetry_generation(&mut payload, run_telemetry) {
        tracing::debug!(
            target: LOG_TARGET,
            "[agent-tracing] added run telemetry aggregate to Langfuse journal batch"
        );
    } else {
        tracing::debug!(
            target: LOG_TARGET,
            "[agent-tracing] no run telemetry aggregate added to Langfuse journal batch"
        );
    }
    // Langfuse caps a single ingestion request at 500 events; a large run (e.g.
    // one that spawns sub-agents) can far exceed that and previously had its
    // ENTIRE trace rejected with a 400. Send in <=500-event chunks instead.
    for chunk in split_ingestion_batch(payload, LANGFUSE_MAX_BATCH_EVENTS) {
        tokio::time::timeout(PUSH_TIMEOUT, client.send_batch(chunk))
            .await
            .map_err(|_| format!("Langfuse journal push timed out after {PUSH_TIMEOUT:?}"))?
            .map_err(|err| format!("Langfuse journal ingestion failed: {err}"))?;
    }

    tracing::debug!(
        target: LOG_TARGET,
        "[agent-tracing] pushed {observation_count} journal observations to Langfuse"
    );
    Ok(())
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
    if !url.starts_with("http") {
        return Err(format!(
            "could not resolve Langfuse ingestion URL from backend host (got {url:?})"
        ));
    }
    let token = require_live_session_token(config)?;
    let include_content = config.observability.agent_tracing.capture_content;
    let environment = environment_for_base(&url);
    let batch = spans_to_langfuse_batch(spans, include_content, environment);
    let span_count = spans.len();

    tracing::debug!(
        target: LOG_TARGET,
        "[agent-tracing] pushing {span_count} spans to Langfuse at {url}"
    );

    let response = reqwest::Client::new()
        .post(&url)
        .header(
            reqwest::header::AUTHORIZATION,
            bearer_authorization_value(&token),
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn split_ingestion_batch_chunks_over_the_limit() {
        // 1201 events with max 500 -> chunks of 500, 500, 201.
        let events: Vec<Value> = (0..1201).map(|i| json!({ "id": i })).collect();
        let payload = json!({ "batch": events, "metadata": { "k": "v" } });

        let parts = split_ingestion_batch(payload, 500);
        assert_eq!(parts.len(), 3);
        let sizes: Vec<usize> = parts
            .iter()
            .map(|p| p["batch"].as_array().unwrap().len())
            .collect();
        assert_eq!(sizes, vec![500, 500, 201]);
        // Every chunk stays within the limit and preserves other top-level keys.
        for p in &parts {
            assert!(p["batch"].as_array().unwrap().len() <= 500);
            assert_eq!(p["metadata"]["k"], json!("v"));
        }
        // The first event of the run (e.g. the trace-create) lands in chunk 0.
        assert_eq!(parts[0]["batch"][0]["id"], json!(0));
        // Order is preserved across the split.
        assert_eq!(parts[2]["batch"][0]["id"], json!(1000));
    }

    #[test]
    fn split_ingestion_batch_passes_small_payloads_through() {
        let payload = json!({ "batch": [json!({ "id": 1 }), json!({ "id": 2 })] });
        let parts = split_ingestion_batch(payload.clone(), 500);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0], payload);
        // A payload without a `batch` array is returned untouched.
        let no_batch = json!({ "hello": "world" });
        assert_eq!(split_ingestion_batch(no_batch.clone(), 500), vec![no_batch]);
    }

    use crate::openhuman::agent::progress_tracing::SpanKind;
    use tinyagents::harness::ids::{CallId, EventId, RunId};
    use tinyagents::harness::usage::Usage;

    fn span(
        trace: &str,
        id: &str,
        parent: Option<&str>,
        name: &str,
        kind: SpanKind,
        status: SpanStatus,
        start: u64,
        end: Option<u64>,
    ) -> TraceSpan {
        let mut attributes = BTreeMap::new();
        attributes.insert("tokens".to_string(), json!(42));
        TraceSpan {
            trace_id: trace.to_string(),
            span_id: id.to_string(),
            parent_span_id: parent.map(str::to_string),
            name: name.to_string(),
            kind,
            start_unix_ms: start,
            end_unix_ms: end,
            status,
            attributes,
            input: None,
            output: None,
        }
    }

    fn obs(offset: u64, event: AgentEvent) -> AgentObservation {
        AgentObservation {
            event_id: EventId::new(format!("run-1-evt-{offset}")),
            run_id: RunId::new("run-1"),
            parent_run_id: None,
            root_run_id: RunId::new("run-1"),
            offset,
            ts_ms: 1_000 + offset,
            event,
        }
    }

    #[test]
    fn ingestion_url_uses_backend_origin_and_ingestion_path() {
        let mut config = Config::default();
        config.api_url = Some("https://staging-api.tinyhumans.ai/api/v1".to_string());
        assert_eq!(
            ingestion_url(&config),
            "https://staging-api.tinyhumans.ai/telemetry/langfuse/ingestion",
            "endpoint is the backend's Langfuse proxy route on the base server \
             host, replacing any inference path the base carried"
        );

        // A base carrying an inference path resolves to the proxy route on the
        // SAME host — the ingestion host tracks the base server URL, not a fixed
        // literal.
        let mut with_inference_path = Config::default();
        with_inference_path.api_url =
            Some("https://api.tinyhumans.ai/openai/v1/chat/completions".to_string());
        assert_eq!(
            ingestion_url(&with_inference_path),
            "https://api.tinyhumans.ai/telemetry/langfuse/ingestion"
        );
    }

    #[test]
    fn trace_config_from_context_matches_span_trace_attribution() {
        let ctx = TraceContext::new("trace:req-1", Some("user-1".to_string()))
            .with_session_group("thread-abc")
            .with_client_id("socket-abc")
            .with_agent_id("researcher")
            .with_channel_source("chat")
            .with_run_type(crate::openhuman::agent::progress_tracing::RunType::InteractiveChat);

        let trace = trace_config_from_context(&ctx, "staging");
        assert_eq!(trace.trace_id.as_deref(), Some("trace:req-1"));
        assert_eq!(trace.name.as_deref(), Some("agent.turn:researcher"));
        assert_eq!(trace.user_id.as_deref(), Some("user-1"));
        assert_eq!(trace.session_id.as_deref(), Some("thread-abc"));
        assert_eq!(trace.environment.as_deref(), Some("staging"));
        assert_eq!(trace.tags, vec!["run:interactive_chat", "source:chat"]);
        assert_eq!(trace.metadata["client.id"], "socket-abc");
        assert_eq!(trace.metadata["agent.id"], "researcher");
        assert_eq!(trace.metadata["channel.source"], "chat");
        assert_eq!(trace.metadata["run_type"], "interactive_chat");
        assert_eq!(trace.metadata["app.version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn trace_config_from_context_stamps_run_lineage() {
        // A spawned sub-agent: its run has a parent (the spawning turn) and a
        // root. Stamping these onto trace metadata is what links the sub-agent's
        // Langfuse trace back to the parent turn (#4657).
        let ctx = TraceContext::new("trace:req-1", None).with_run_lineage(
            Some("sub-run".to_string()),
            Some("parent-run".to_string()),
            Some("root-run".to_string()),
        );
        let trace = trace_config_from_context(&ctx, "staging");
        assert_eq!(trace.metadata["run_id"], "sub-run");
        assert_eq!(trace.metadata["parent_run_id"], "parent-run");
        assert_eq!(trace.metadata["root_run_id"], "root-run");
    }

    #[test]
    fn trace_config_omits_parent_run_id_for_top_level_turn() {
        // A top-level turn has no parent; the key must stay absent (root == run).
        let ctx = TraceContext::new("trace:req-1", None).with_run_lineage(
            Some("run-1".to_string()),
            None,
            Some("run-1".to_string()),
        );
        let trace = trace_config_from_context(&ctx, "staging");
        assert_eq!(trace.metadata["run_id"], "run-1");
        assert_eq!(trace.metadata["root_run_id"], "run-1");
        assert!(
            trace.metadata.get("parent_run_id").is_none(),
            "top-level turn must not carry a parent_run_id"
        );
    }

    #[test]
    fn trace_ctx_with_run_lineage_derives_from_subagent_observations() {
        // Sub-agent observations carry parent/root ids pointing at the spawning
        // turn; the derived trace context stamps them so the sub-agent's trace
        // links back instead of landing as a disconnected sibling (#4657).
        let observations = vec![AgentObservation {
            event_id: EventId::new("evt-1"),
            run_id: RunId::new("sub-run"),
            parent_run_id: Some(RunId::new("parent-run")),
            root_run_id: RunId::new("root-run"),
            offset: 1,
            ts_ms: 1_000,
            event: AgentEvent::ModelCompleted {
                call_id: CallId::new("model-1"),
                started_at_ms: Some(1_000),
                usage: Some(Usage::new(1, 1)),
                input: None,
                output: None,
            },
        }];
        let base = TraceContext::new("trace:req-1", None);

        let enriched = trace_ctx_with_run_lineage(&base, &observations);
        assert_eq!(enriched.run_id.as_deref(), Some("sub-run"));
        assert_eq!(enriched.parent_run_id.as_deref(), Some("parent-run"));
        assert_eq!(enriched.root_run_id.as_deref(), Some("root-run"));

        // An empty stream leaves the context untouched (no lineage invented).
        let untouched = trace_ctx_with_run_lineage(&base, &[]);
        assert!(untouched.run_id.is_none());
        assert!(untouched.parent_run_id.is_none());
        assert!(untouched.root_run_id.is_none());
    }

    #[test]
    fn journal_observation_content_follows_capture_gate() {
        let observations = vec![
            obs(
                1,
                AgentEvent::ModelCompleted {
                    call_id: CallId::new("model-1"),
                    started_at_ms: Some(1_000),
                    usage: Some(Usage::new(10, 3)),
                    input: Some(json!([{"role": "user", "content": "secret prompt"}])),
                    output: Some(json!({"role": "assistant", "content": "secret reply"})),
                },
            ),
            obs(
                2,
                AgentEvent::ToolCompleted {
                    call_id: CallId::new("tool-1"),
                    tool_name: "search".to_string(),
                    started_at_ms: Some(1_010),
                    input: Some(json!({"query": "secret"})),
                    output: Some(json!("secret result")),
                    duration_ms: Some(20),
                    output_bytes: Some(13),
                    error: None,
                },
            ),
        ];

        let off_ctx = TraceContext::new("trace:req-1", None);
        let filtered = observations_for_export(&off_ctx, &observations);
        assert!(matches!(filtered, Cow::Owned(_)));
        match &filtered[0].event {
            AgentEvent::ModelCompleted { input, output, .. } => {
                assert!(input.is_none());
                assert!(output.is_none());
            }
            other => panic!("unexpected event: {other:?}"),
        }
        match &filtered[1].event {
            AgentEvent::ToolCompleted { input, output, .. } => {
                assert!(input.is_none());
                assert!(output.is_none());
            }
            other => panic!("unexpected event: {other:?}"),
        }
        match &observations[0].event {
            AgentEvent::ModelCompleted { input, output, .. } => {
                assert!(input.is_some(), "source journal observation stays intact");
                assert!(output.is_some(), "source journal observation stays intact");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let on_ctx = TraceContext::new("trace:req-1", None).with_capture_content(true);
        let passthrough = observations_for_export(&on_ctx, &observations);
        assert!(matches!(passthrough, Cow::Borrowed(_)));
        match &passthrough[1].event {
            AgentEvent::ToolCompleted { input, output, .. } => {
                assert_eq!(input.as_ref(), Some(&json!({"query": "secret"})));
                assert_eq!(output.as_ref(), Some(&json!("secret result")));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn run_telemetry_inserts_aggregate_generation() {
        let observations = vec![obs(
            1,
            AgentEvent::ModelCompleted {
                call_id: CallId::new("model-1"),
                started_at_ms: Some(1_000),
                usage: Some(Usage::new(100, 20)),
                input: None,
                output: None,
            },
        )];
        let client =
            LangfuseClient::proxy("https://api.tinyhumans.ai", "token").expect("proxy client");
        let trace =
            trace_config_from_context(&TraceContext::new("trace:req-1", None), "production");
        let mut payload = client
            .build_ingestion_batch(trace, &observations)
            .expect("batch");
        let telemetry = RunTelemetry {
            run_id: "req-1".to_string(),
            input_tokens: 120,
            output_tokens: 30,
            cached_input_tokens: 40,
            cost_usd: 0.0123,
            elapsed_ms: Some(900),
            tool_count: 2,
            model: Some("managed.chat-v1".to_string()),
            provider: Some("managed".to_string()),
            error: None,
            updated_at: None,
        };

        assert!(insert_run_telemetry_generation(
            &mut payload,
            Some(&telemetry)
        ));
        let batch = payload["batch"].as_array().expect("batch array");
        assert_eq!(batch[1]["type"], "generation-create");
        let body = &batch[1]["body"];
        assert_eq!(body["id"], "trace:req-1:openhuman-run-telemetry");
        assert_eq!(body["name"], "run.total");
        assert_eq!(body["traceId"], "trace:req-1");
        assert_eq!(body["model"], "managed.chat-v1");
        assert_eq!(body["usageDetails"]["input"], 80);
        assert_eq!(body["usageDetails"]["output"], 30);
        assert_eq!(body["usageDetails"]["total"], 150);
        assert_eq!(body["usageDetails"]["cache_read_input_tokens"], 40);
        assert_eq!(body["costDetails"]["total"], 0.0123);
        assert_eq!(body["metadata"]["source"], "openhuman.run_telemetry");
        assert_eq!(body["metadata"]["run_id"], "req-1");
        assert_eq!(body["metadata"]["tool_count"], 2);
        assert_eq!(body["metadata"]["provider"], "managed");
    }

    #[test]
    fn iso_millis_formats_epoch_as_rfc3339() {
        // 2021-01-01T00:00:00Z = 1_609_459_200_000 ms.
        assert!(iso_millis(1_609_459_200_000).starts_with("2021-01-01T00:00:00"));
    }

    #[test]
    fn batch_emits_trace_create_then_one_span_create_each() {
        let spans = vec![
            span(
                "trace-1",
                "root",
                None,
                "agent.turn",
                SpanKind::Turn,
                SpanStatus::Ok,
                1_000,
                Some(2_000),
            ),
            span(
                "trace-1",
                "tool-1",
                Some("root"),
                "tool.web_search",
                SpanKind::Tool,
                SpanStatus::Error,
                1_100,
                Some(1_500),
            ),
        ];
        let payload = spans_to_langfuse_batch(&spans, false, "production");
        let batch = payload["batch"].as_array().expect("batch array");
        assert_eq!(batch.len(), 3, "one trace-create + two span-create");

        assert_eq!(batch[0]["type"], "trace-create");
        assert_eq!(batch[0]["body"]["id"], "trace-1");

        // Camel-case Langfuse fields, ISO timestamps, parent linkage, error level.
        let root = &batch[1];
        assert_eq!(root["type"], "span-create");
        assert_eq!(root["body"]["id"], "root");
        assert_eq!(root["body"]["traceId"], "trace-1");
        assert!(root["body"]["startTime"].as_str().unwrap().contains('T'));
        assert_eq!(root["body"]["level"], "DEFAULT");
        assert_eq!(root["body"]["metadata"]["kind"], "turn");
        assert!(root["body"].get("parentObservationId").is_none());

        let tool = &batch[2];
        assert_eq!(tool["body"]["parentObservationId"], "root");
        assert_eq!(tool["body"]["level"], "ERROR");
        assert!(tool["body"]["endTime"].as_str().unwrap().contains('T'));

        // Event ids are unique and distinct from the observation ids.
        assert_ne!(batch[1]["id"], batch[2]["id"]);
        assert_ne!(batch[1]["id"], batch[1]["body"]["id"]);
    }

    #[test]
    fn usage_span_becomes_generation_and_content_is_gated() {
        let mut turn = span(
            "trace-1",
            "root",
            None,
            "agent.turn",
            SpanKind::Turn,
            SpanStatus::Ok,
            1_000,
            Some(2_000),
        );
        turn.attributes.clear();
        turn.attributes
            .insert("gen_ai.request.model".into(), json!("claude-x"));
        turn.attributes
            .insert("gen_ai.usage.input_tokens".into(), json!(100));
        turn.attributes
            .insert("gen_ai.usage.output_tokens".into(), json!(20));
        turn.attributes
            .insert("gen_ai.usage.cost_usd".into(), json!(0.0123));
        turn.input = Some(json!("what is 2+2?"));
        turn.output = Some(json!("4"));
        let spans = vec![turn];

        // Content OFF (default): span is promoted to a generation with native
        // usage + cost, but prompt/reply are withheld.
        let off = spans_to_langfuse_batch(&spans, false, "production");
        let obs = &off["batch"][1];
        assert_eq!(obs["type"], "generation-create");
        assert_eq!(obs["body"]["model"], "claude-x");
        assert_eq!(obs["body"]["usageDetails"]["input"], 100);
        assert_eq!(obs["body"]["usageDetails"]["output"], 20);
        assert_eq!(obs["body"]["usageDetails"]["total"], 120);
        assert_eq!(obs["body"]["costDetails"]["total"], 0.0123);
        assert!(
            obs["body"].get("input").is_none(),
            "prompt must be withheld when capture_content is off"
        );
        assert!(obs["body"].get("output").is_none());

        // Content ON: prompt/reply included, usage/cost unchanged.
        let on = spans_to_langfuse_batch(&spans, true, "production");
        let obs = &on["batch"][1];
        assert_eq!(obs["type"], "generation-create");
        assert_eq!(obs["body"]["input"], "what is 2+2?");
        assert_eq!(obs["body"]["output"], "4");
        assert_eq!(obs["body"]["costDetails"]["total"], 0.0123);
    }

    #[test]
    fn trace_create_carries_user_and_session_grouping() {
        // The turn span's user.id / thread.id attributes are promoted onto the
        // trace-create as Langfuse userId / sessionId so per-turn traces group
        // under one conversation and attribute to a user.
        let mut turn = span(
            "trace:req-1",
            "root",
            None,
            "agent.turn",
            SpanKind::Turn,
            SpanStatus::Ok,
            1_000,
            Some(2_000),
        );
        turn.attributes.insert("user.id".into(), json!("client-7"));
        turn.attributes
            .insert("thread.id".into(), json!("thread-abc"));
        let payload = spans_to_langfuse_batch(&[turn], false, "production");
        let trace = &payload["batch"][0];
        assert_eq!(trace["type"], "trace-create");
        assert_eq!(trace["body"]["userId"], "client-7");
        assert_eq!(trace["body"]["sessionId"], "thread-abc");
    }

    #[test]
    fn trace_create_session_id_falls_back_to_trace_id() {
        // No thread.id attribute → the trace id itself becomes the sessionId,
        // so every trace lands with a session in Langfuse.
        let turn = span(
            "trace:req-2",
            "root",
            None,
            "agent.turn",
            SpanKind::Turn,
            SpanStatus::Ok,
            1_000,
            Some(2_000),
        );
        let payload = spans_to_langfuse_batch(&[turn], false, "production");
        assert_eq!(payload["batch"][0]["body"]["sessionId"], "trace:req-2");
    }

    #[test]
    fn trace_create_metadata_carries_attribution_and_version() {
        let mut turn = span(
            "trace-1",
            "root",
            None,
            "agent.turn:researcher",
            SpanKind::Turn,
            SpanStatus::Ok,
            1_000,
            Some(2_000),
        );
        turn.attributes
            .insert("client.id".into(), json!("socket-abc"));
        turn.attributes
            .insert("agent.id".into(), json!("researcher"));
        turn.attributes
            .insert("channel.source".into(), json!("chat"));
        let payload = spans_to_langfuse_batch(&[turn], false, "production");
        let trace = &payload["batch"][0]["body"];
        assert_eq!(trace["name"], "agent.turn:researcher");
        let meta = &trace["metadata"];
        assert_eq!(meta["client.id"], "socket-abc");
        assert_eq!(meta["agent.id"], "researcher");
        assert_eq!(meta["channel.source"], "chat");
        assert_eq!(meta["app.version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn trace_create_input_output_follow_content_gate() {
        let mut turn = span(
            "trace-1",
            "root",
            None,
            "agent.turn",
            SpanKind::Turn,
            SpanStatus::Ok,
            1_000,
            Some(2_000),
        );
        turn.input = Some(json!("the prompt"));
        turn.output = Some(json!("the reply"));
        let spans = vec![turn];

        let on = spans_to_langfuse_batch(&spans, true, "production");
        assert_eq!(on["batch"][0]["body"]["input"], "the prompt");
        assert_eq!(on["batch"][0]["body"]["output"], "the reply");

        let off = spans_to_langfuse_batch(&spans, false, "production");
        assert!(off["batch"][0]["body"].get("input").is_none());
        assert!(off["batch"][0]["body"].get("output").is_none());
    }

    #[test]
    fn environment_derivation_from_backend_base() {
        assert_eq!(
            environment_for_base("https://staging-api.tinyhumans.ai"),
            "staging"
        );
        assert_eq!(environment_for_base("http://localhost:5000"), "development");
        assert_eq!(environment_for_base("http://127.0.0.1:5000"), "development");
        assert_eq!(
            environment_for_base("https://api.tinyhumans.ai"),
            "production"
        );
    }

    #[test]
    fn trace_create_carries_environment_release_and_run_tags() {
        let mut turn = span(
            "trace-1",
            "root",
            None,
            "agent.turn",
            SpanKind::Turn,
            SpanStatus::Ok,
            1_000,
            Some(2_000),
        );
        turn.attributes
            .insert("run.type".into(), json!("autonomous_task"));
        turn.attributes
            .insert("channel.source".into(), json!("autonomous"));
        let payload = spans_to_langfuse_batch(&[turn], false, "staging");
        let trace = &payload["batch"][0]["body"];
        // Top-level Langfuse trace fields, not metadata.
        assert_eq!(trace["environment"], "staging");
        assert_eq!(trace["release"], env!("CARGO_PKG_VERSION"));
        // Filterable run tags + run_type metadata.
        assert_eq!(
            trace["tags"],
            json!(["run:autonomous_task", "source:autonomous"])
        );
        assert_eq!(trace["metadata"]["run_type"], "autonomous_task");
    }

    #[test]
    fn interactive_chat_trace_gets_interactive_run_tag() {
        let mut turn = span(
            "trace-1",
            "root",
            None,
            "agent.turn",
            SpanKind::Turn,
            SpanStatus::Ok,
            1_000,
            Some(2_000),
        );
        turn.attributes
            .insert("run.type".into(), json!("interactive_chat"));
        turn.attributes
            .insert("channel.source".into(), json!("chat"));
        let payload = spans_to_langfuse_batch(&[turn], false, "production");
        let trace = &payload["batch"][0]["body"];
        assert_eq!(
            trace["tags"],
            json!(["run:interactive_chat", "source:chat"])
        );
        assert_eq!(trace["metadata"]["run_type"], "interactive_chat");
    }

    #[test]
    fn generation_usage_details_map_reasoning_and_cache_tokens() {
        let mut gen = span(
            "trace-1",
            "gen-1",
            Some("root"),
            "llm.agentic-v1",
            SpanKind::Generation,
            SpanStatus::Ok,
            1_000,
            Some(1_500),
        );
        gen.attributes.clear();
        gen.attributes
            .insert("gen_ai.request.model".into(), json!("agentic-v1"));
        gen.attributes
            .insert("gen_ai.usage.input_tokens".into(), json!(1_000));
        gen.attributes
            .insert("gen_ai.usage.output_tokens".into(), json!(200));
        gen.attributes
            .insert("gen_ai.usage.cached_input_tokens".into(), json!(0));
        gen.attributes
            .insert("gen_ai.usage.reasoning_tokens".into(), json!(128));
        gen.attributes
            .insert("gen_ai.usage.cache_creation_tokens".into(), json!(64));
        gen.attributes
            .insert("gen_ai.usage.cost_usd".into(), json!(0.0042));
        gen.attributes
            .insert("gen_ai.provider".into(), json!("managed"));

        let payload = spans_to_langfuse_batch(&[gen], false, "production");
        let obs = &payload["batch"][1];
        assert_eq!(obs["type"], "generation-create");
        let usage = &obs["body"]["usageDetails"];
        assert_eq!(usage["input"], 1_000);
        assert_eq!(usage["output"], 200);
        // Cache reads always flow, even at 0.
        assert_eq!(usage["cache_read_input_tokens"], 0);
        assert_eq!(usage["reasoning_tokens"], 128);
        assert_eq!(usage["cache_creation_input_tokens"], 64);
        assert_eq!(obs["body"]["costDetails"]["total"], 0.0042);
        // Provenance rides in observation metadata.
        assert_eq!(obs["body"]["metadata"]["gen_ai.provider"], "managed");
    }

    #[test]
    fn generation_without_reasoning_or_cache_write_omits_those_usage_keys() {
        let mut gen = span(
            "trace-1",
            "gen-1",
            Some("root"),
            "llm.agentic-v1",
            SpanKind::Generation,
            SpanStatus::Ok,
            1_000,
            Some(1_500),
        );
        gen.attributes.clear();
        gen.attributes
            .insert("gen_ai.usage.input_tokens".into(), json!(10));
        gen.attributes
            .insert("gen_ai.usage.output_tokens".into(), json!(5));
        let payload = spans_to_langfuse_batch(&[gen], false, "production");
        let usage = &payload["batch"][1]["body"]["usageDetails"];
        assert_eq!(
            usage["cache_read_input_tokens"], 0,
            "cache reads always present"
        );
        assert!(usage.get("reasoning_tokens").is_none());
        assert!(usage.get("cache_creation_input_tokens").is_none());
    }

    #[test]
    fn error_span_gets_error_level_and_status_message() {
        let mut tool = span(
            "trace-1",
            "tool-1",
            Some("root"),
            "tool.shell",
            SpanKind::Tool,
            SpanStatus::Error,
            1_000,
            Some(1_200),
        );
        tool.attributes
            .insert("error.message".into(), json!("The command timed out"));
        let payload = spans_to_langfuse_batch(&[tool], false, "production");
        let obs = &payload["batch"][1]["body"];
        assert_eq!(obs["level"], "ERROR");
        assert_eq!(obs["statusMessage"], "The command timed out");

        // Without a captured message: ERROR level, no statusMessage.
        let bare = span(
            "trace-1",
            "tool-2",
            Some("root"),
            "tool.shell",
            SpanKind::Tool,
            SpanStatus::Error,
            1_000,
            Some(1_200),
        );
        let payload = spans_to_langfuse_batch(&[bare], false, "production");
        let obs = &payload["batch"][1]["body"];
        assert_eq!(obs["level"], "ERROR");
        assert!(obs.get("statusMessage").is_none());
    }

    #[tokio::test]
    async fn empty_spans_push_is_ok_noop() {
        let config = Config::default();
        // Empty batch short-circuits before any host/token resolution or network.
        assert!(push_spans(&config, &[]).await.is_ok());
    }
}
