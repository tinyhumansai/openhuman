use std::borrow::Cow;
use std::time::Duration;

use serde_json::{json, Map, Value};
use tinyagents_harness::events::AgentEvent;
use tinyagents_harness::observability::{AgentObservation, LangfuseClient, LangfuseTraceConfig};

use crate::api::config::effective_backend_api_url;
use crate::api::jwt::bearer_authorization_value;
use crate::openhuman::config::Config;
use crate::openhuman::security::credentials::session_support::require_live_session_token;
use tinyagents_session::run_ledger::RunTelemetry;

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

/// The domain the deployed backends live under. A host outside it cannot be
/// one of ours, so it cannot be staging however it is spelled.
const DEPLOYMENT_DOMAIN: &str = "tinyhumans.ai";

/// Derive the Langfuse `environment` for a backend base URL. Chosen signal:
/// the resolved backend host is the single existing config-driven fact that
/// distinguishes deployments (there is no NODE_ENV-style flag in the core
/// config) — loopback/local → development, a staging host under
/// [`DEPLOYMENT_DOMAIN`] → staging, anything else → production.
///
/// # Why the host is parsed rather than substring-matched
///
/// This used to test `base.contains("staging")` and
/// `base.contains("localhost" | "127.0.0.1" | "0.0.0.0")` against the whole
/// URL, which was tolerable while the answer only labelled a payload. It is
/// not tolerable now that [`skip_push`] decides whether to push at all, so
/// both directions of the sloppiness became load-bearing:
///
/// - **Too broad on staging.** `https://staging-attacker.invalid` contains
///   `staging`, so it classified as staging and passed the push gate — a host
///   nothing in this tree owns, reached with a live session token. Anchoring
///   to [`DEPLOYMENT_DOMAIN`] is what makes the classifier fail *closed*: a
///   host that is not ours is production, and production does not push.
/// - **Too narrow on local.** An IPv6 loopback backend (`http://[::1]:7788`)
///   or a private LAN address matched none of the three literals and
///   classified as production, so a working development setup would have
///   silently stopped exporting the moment the gate landed.
///
/// Host classification is delegated to [`crate::api::config::host_is_local`],
/// which already parses the URL and handles IPv4 loopback/unspecified/private,
/// IPv6 loopback/unspecified, and `localhost` / `*.localhost`. Keeping one
/// definition matters more than the few lines it saves: two local-host
/// predicates that disagree is how a gate lets through exactly the case the
/// other one blocks.
///
/// An unparseable URL is production — the fail-closed default. `ingestion_url`
/// can return a non-URL placeholder when no backend host resolves, and the
/// caller checks `starts_with("http")` separately; classifying that as
/// anything pushable would defeat the gate.
pub(crate) fn environment_for_base(base: &str) -> &'static str {
    let Ok(parsed) = url::Url::parse(base) else {
        return "production";
    };
    if crate::api::config::host_is_local(&parsed) {
        return "development";
    }
    let Some(url::Host::Domain(host)) = parsed.host() else {
        // A public IP literal is not a deployment of ours.
        return "production";
    };
    let host = host.to_ascii_lowercase();
    let under_deployment_domain =
        host == DEPLOYMENT_DOMAIN || host.ends_with(&format!(".{DEPLOYMENT_DOMAIN}"));
    // The label test is on the leftmost label only, so `staging-api…` and
    // `staging…` match while `api-staging-mirror…` does not sneak in on a
    // substring.
    let leftmost_is_staging = host
        .split('.')
        .next()
        .is_some_and(|label| label == "staging" || label.starts_with("staging-"));
    if under_deployment_domain && leftmost_is_staging {
        "staging"
    } else {
        "production"
    }
}

/// The environments this client may push to Langfuse from.
///
/// An allowlist, not `!= "production"`, and it mirrors the backend's rule
/// (`backend:src/config/langfuseEnvironment.ts`) deliberately: the two gates
/// have to agree, and two negations drift more easily than two lists. Stating
/// the permitted set also makes the fail-closed property structural — if
/// [`environment_for_base`] ever grows a fourth bucket, that bucket does not
/// push until someone adds it here on purpose.
///
/// `test` appears in the backend's list but not here because there is no such
/// bucket on this side: [`environment_for_base`] maps loopback hosts to
/// `development`, and that is what the Rust suite resolves to.
const LANGFUSE_PUSH_ENVIRONMENTS: &[&str] = &["staging", "development"];

/// Whether a push is permitted for a resolved environment.
fn push_allowed(environment: &str) -> bool {
    LANGFUSE_PUSH_ENVIRONMENTS.contains(&environment)
}

/// Emitted at most once per process by [`skip_push`].
static SKIP_LOGGED: std::sync::Once = std::sync::Once::new();

/// Whether this push should be dropped before any work, logging the reason
/// once per process.
///
/// # Why skip at all, when the backend already refuses
///
/// Defence in depth, and latency. The backend answers `403 FEATURE_DISABLED`
/// outside staging (backend#1291), so nothing reaches Langfuse either way —
/// but a client that still asks pays a full authenticated round-trip on every
/// agent turn to be told no, and [`PUSH_TIMEOUT`] bounds that at ten seconds
/// when the host is slow. #5602 is that stall. The cheapest request is the one
/// not made.
///
/// # Why once per process, and at info
///
/// This is on the path of every completed run. A warning per turn would move
/// the noise rather than remove it, and a skip in production is the configured
/// outcome, not a fault — so it is `info`, said once, and then silence. The
/// caller receives `Ok(())`: skipping is a successful no-op, and returning
/// `Err` would make the caller log the same line on every turn, which is the
/// thing being avoided.
fn skip_push(environment: &str) -> bool {
    if push_allowed(environment) {
        return false;
    }
    SKIP_LOGGED.call_once(|| {
        tracing::info!(
            target: LOG_TARGET,
            "[agent-tracing] Langfuse push disabled for environment {environment:?} \
             (enabled in: {}) — traces stay local for the rest of this process",
            LANGFUSE_PUSH_ENVIRONMENTS.join(", ")
        );
    });
    true
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
