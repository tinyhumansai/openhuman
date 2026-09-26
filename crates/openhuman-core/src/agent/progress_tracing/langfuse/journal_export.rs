//! Journal export: pushes a run's durable tinyagents observations through the
//! tinyagents Langfuse exporter, stamping run lineage and trace attribution,
//! gating content on `capture_content`, and folding the run-ledger telemetry
//! aggregate into the batch.

use std::borrow::Cow;

use serde_json::{json, Map, Value};
use tinyagents_harness::events::AgentEvent;
use tinyagents_harness::observability::{AgentObservation, LangfuseClient, LangfuseTraceConfig};
use tinyagents_session::run_ledger::RunTelemetry;

use crate::config::Config;
use crate::security::credentials::session_support::direct_backend_credential;

use super::ingestion_batch::{new_event_id, split_ingestion_batch, LANGFUSE_MAX_BATCH_EVENTS};
use super::TraceContext;
use super::{environment_for_base, ingestion_url, skip_push, LOG_TARGET, PUSH_TIMEOUT};

/// Enrich `trace_ctx` with the run lineage (`run_id` / `parent_run_id` /
/// `root_run_id`) carried by the run's journalled `observations` (#4657).
///
/// A single export corresponds to one run's observation stream (the journal is
/// read per run id), so every observation shares the same lineage and the first
/// is representative. For a spawned sub-agent that lineage points back at the
/// spawning turn, which is exactly what links the sub-agent's trace to its
/// parent. Returns the context unchanged when there are no observations.
pub(super) fn trace_ctx_with_run_lineage(
    trace_ctx: &TraceContext,
    observations: &[AgentObservation],
) -> TraceContext {
    let Some(first) = observations.first() else {
        return trace_ctx.clone();
    };
    trace_ctx.clone().with_run_lineage(
        trace_ctx
            .run_id
            .clone()
            .or_else(|| Some(first.run_id.as_str().to_string())),
        trace_ctx.parent_run_id.clone().or_else(|| {
            first
                .parent_run_id
                .as_ref()
                .map(|id| id.as_str().to_string())
        }),
        trace_ctx
            .root_run_id
            .clone()
            .or_else(|| Some(first.root_run_id.as_str().to_string())),
    )
}

/// A child is a root observation within its own trace. Preserve its original
/// run lineage on the trace metadata before calling this projection.
pub(crate) fn root_subagent_observations(
    observations: &[AgentObservation],
) -> Vec<AgentObservation> {
    observations
        .iter()
        .cloned()
        .map(|mut observation| {
            observation.parent_run_id = None;
            observation.root_run_id = observation.run_id.clone();
            observation
        })
        .collect()
}

pub(super) fn trace_config_from_context(
    trace_ctx: &TraceContext,
    environment: &str,
) -> LangfuseTraceConfig {
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

pub(super) fn observations_for_export<'a>(
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

pub(super) fn insert_run_telemetry_generation(
    payload: &mut Value,
    telemetry: Option<&RunTelemetry>,
) -> bool {
    let Some(telemetry) = telemetry else {
        return false;
    };
    if telemetry.input_tokens == 0 && telemetry.output_tokens == 0 && telemetry.cost_usd == 0.0 {
        return false;
    }

    let Some(batch) = payload.get_mut("batch").and_then(Value::as_array_mut) else {
        return false;
    };
    // Native per-call charges are already counted by Langfuse. Adding the
    // run aggregate as another generation would double-count the same spend.
    if batch.iter().any(|event| {
        event["type"] == "generation-create"
            && event["body"]["costDetails"]["total"].as_f64().is_some()
    }) {
        return false;
    }
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
/// Whether [`push_observations`] would actually send for `config`: the same
/// gates it checks, for a caller that must read a journal and build
/// observations first. Without a live session (unit tests, a signed-out or
/// embedder host) or on a skipped environment, that work is discarded
/// anyway — and reading a whole child journal is not free.
pub(crate) fn journal_push_ready(config: &Config) -> bool {
    let url = ingestion_url(config);
    !skip_push(environment_for_base(&url))
        && url.starts_with("http")
        && direct_backend_credential(config, "langfuse journal push").is_some()
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
    // Same gate, same reasons as `push_spans` — both entry points are on the
    // per-turn path, so both skip before doing any work.
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
    let Some(credential) = direct_backend_credential(config, "langfuse journal push") else {
        return Ok(());
    };
    let token = credential.into_secret();
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
