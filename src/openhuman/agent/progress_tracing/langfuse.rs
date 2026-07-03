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
//! never breaks a turn. Spans carry only metadata (names, kinds, timings,
//! counts) — never prompt text, tool arguments, or PII — honoring the project's
//! "never log secrets or full PII" rule.

use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::api::config::effective_backend_api_url;
use crate::api::jwt::bearer_authorization_value;
use crate::openhuman::config::Config;
use crate::openhuman::credentials::session_support::require_live_session_token;

use super::{SpanStatus, TraceSpan};

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

/// Convert finished spans into a Langfuse `/api/public/ingestion` batch payload:
/// a single `trace-create` for the shared trace id followed by one
/// `span-create` observation per span. Field names are Langfuse's camelCase
/// (`traceId`, `startTime`, `parentObservationId`); timestamps are ISO strings.
pub(crate) fn spans_to_langfuse_batch(spans: &[TraceSpan]) -> Value {
    let mut batch: Vec<Value> = Vec::with_capacity(spans.len() + 1);

    // One trace-create for the run, keyed by the shared trace id. Prefer the
    // root (parentless) span for the trace name/start; fall back to the first.
    if let Some(root) = spans
        .iter()
        .find(|s| s.parent_span_id.is_none())
        .or_else(|| spans.first())
    {
        batch.push(json!({
            "id": new_event_id(),
            "type": "trace-create",
            "timestamp": iso_millis(root.start_unix_ms),
            "body": {
                "id": root.trace_id,
                "name": root.name,
                "timestamp": iso_millis(root.start_unix_ms),
            },
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
        batch.push(json!({
            "id": new_event_id(),
            "type": "span-create",
            "timestamp": iso_millis(span.start_unix_ms),
            "body": body,
        }));
    }

    json!({ "batch": batch })
}

/// Fresh per-event id. Langfuse dedupes ingestion events by this id, so it must
/// be unique per event (distinct from the observation/trace id in `body`).
fn new_event_id() -> String {
    uuid::Uuid::new_v4().to_string()
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
    let batch = spans_to_langfuse_batch(spans);
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
    // Langfuse returns 207 Multi-Status on partial success; `is_success()`
    // covers the whole 2xx range including that.
    if status.is_success() {
        tracing::debug!(
            target: LOG_TARGET,
            "[agent-tracing] pushed {span_count} spans to Langfuse ({status})"
        );
        Ok(())
    } else {
        let body = response.text().await.unwrap_or_default();
        let excerpt: String = body.chars().take(200).collect();
        Err(format!("Langfuse ingestion returned {status}: {excerpt}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::openhuman::agent::progress_tracing::SpanKind;

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
        let payload = spans_to_langfuse_batch(&spans);
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

    #[tokio::test]
    async fn empty_spans_push_is_ok_noop() {
        let config = Config::default();
        // Empty batch short-circuits before any host/token resolution or network.
        assert!(push_spans(&config, &[]).await.is_ok());
    }
}
