//! Langfuse export for `flows::` graph runs.
//!
//! After a `flows_run` / `flows_resume` settles, the `flows::` domain hands
//! this module the run's durable [`GraphObservation`] slice (captured by the
//! per-run in-memory journal that `tinyflows`' journaled entry points fill)
//! and it exports the run as one Langfuse trace via the backend's Langfuse
//! proxy route, `/telemetry/langfuse/ingestion`.
//!
//! The batch is built by `tinyagents`' [`GraphLangfuseExporter`], which turns
//! each superstep and node into a timed span and stamps the Langfuse **Agent
//! Graph view** keys (`langgraph_node` / `langgraph_step`) on node spans, so
//! the Langfuse UI can render the flow run as a graph. A host span-metadata
//! injector additionally stamps `flow_id` on every span for filtering.
//!
//! Transport mirrors the agent-turn tracing path
//! (`agent::progress_tracing::langfuse::push_spans`): the endpoint is derived
//! from the **current backend hostname** (`effective_backend_api_url`), auth
//! is the live OpenHuman session bearer (the backend injects the real
//! Langfuse keys server-side), the send is capped at 10s, `207 Multi-Status`
//! is tolerated, and every failure is logged and swallowed — exporting is
//! best-effort and never fails the run. Gated on
//! `observability.share_usage_data`.

use std::time::Duration;

use serde_json::{json, Map};
use tinyagents_graph::{GraphLangfuseExporter, GraphObservation};
use tinyagents_harness::{LangfuseAuth, LangfuseClient, LangfuseTraceConfig};
use tinyflows::engine::GraphObservation as FlowObservation;

use crate::api::config::effective_backend_api_url;
use crate::openhuman::config::Config;
use crate::openhuman::flows::FlowRunTrigger;
use crate::openhuman::security::credentials::session_support::require_live_session_token;

const LOG_TARGET: &str = "flows::langfuse";
/// Backend proxy route for Langfuse ingestion (relative to the backend
/// origin). The backend authenticates the session JWT, injects the Langfuse
/// project keys, and forwards to Langfuse's real `/api/public/ingestion`.
const INGESTION_PATH: &str = "/telemetry/langfuse/ingestion";
/// Cap the push so a slow/hung Langfuse never stalls run teardown (same
/// posture as the agent-turn exporter).
const PUSH_TIMEOUT: Duration = Duration::from_secs(10);

/// Resolve the Langfuse ingestion URL from the current backend host — the
/// exact base-server resolution every other backend call uses, so the host
/// always matches wherever the app's domain calls go (staging, prod, or a
/// custom `api_url` override).
fn ingestion_url(config: &Config) -> String {
    let base = effective_backend_api_url(&config.api_url);
    crate::api::config::api_url(&base, INGESTION_PATH)
}

/// The OpenHuman core crate version (e.g. `0.58.0`), stamped onto every flow
/// trace as the Langfuse `release` field plus an `app_version` metadata key so
/// traces can be correlated with the app build that produced them.
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Builds the [`LangfuseTraceConfig`] for one flow run: the trace id **and**
/// session id are the run's `thread_id` (`flow:{flow_id}:{uuid}`), the trace
/// is named `flow.run:{flow_name}`, run-type tags (`run:flow` +
/// `trigger:<kind>`) mark how the run started, and flow coordinates plus the
/// app version ride on the trace metadata. No content — ids, name, status,
/// trigger, and version only.
fn build_flow_trace_config(
    flow_name: &str,
    flow_id: &str,
    thread_id: &str,
    status: &str,
    trigger: FlowRunTrigger,
) -> LangfuseTraceConfig {
    LangfuseTraceConfig {
        trace_id: Some(thread_id.to_string()),
        name: Some(format!("flow.run:{flow_name}")),
        session_id: Some(thread_id.to_string()),
        release: Some(APP_VERSION.to_string()),
        tags: vec![
            "run:flow".to_string(),
            format!("trigger:{}", trigger.as_str()),
        ],
        metadata: json!({
            "flow_id": flow_id,
            "status": status,
            "source": "flows",
            "run_type": "flow",
            "trigger": trigger.as_str(),
            "app_version": APP_VERSION,
        }),
        ..Default::default()
    }
}

/// Builds the graph exporter for a flow run: the batch builder plus a host
/// span-metadata injector that stamps `flow_id` on every span (node spans
/// already carry `langgraph_node`/`langgraph_step` from `tinyagents`).
fn build_flow_exporter(client: LangfuseClient, flow_id: &str) -> GraphLangfuseExporter {
    let flow_id = flow_id.to_string();
    GraphLangfuseExporter::new(client).with_span_metadata_fn(move |_obs| {
        let mut extra = Map::new();
        extra.insert("flow_id".to_string(), json!(flow_id));
        Some(extra)
    })
}

/// Re-types the engine's observations for the exporter.
///
/// The engine now emits `tinyflows::engine::GraphObservation` — tinyflows
/// vendored its state-graph runtime in PR #43 — while the Langfuse batch is
/// still built by `tinyagents`' exporter, which names its own. The two types
/// are the same shape (`tinyagents` is where tinyflows' copy came from) down
/// to the `GraphEvent` payload, so this re-types through their shared serde
/// representation rather than duplicating a 20-field mapping that would then
/// have to be kept in step by hand.
///
/// An observation that fails to convert is **dropped, not fatal**: exporting
/// is best-effort throughout this module, and losing one span from a trace is
/// a better outcome than losing the trace. A drop is logged at `warn` because
/// the only way it can happen is the two types drifting apart, which is worth
/// hearing about — `re_typing_preserves_every_field` is the guard that should
/// catch it first.
fn to_exporter_observations(observations: &[FlowObservation]) -> Vec<GraphObservation> {
    observations
        .iter()
        .filter_map(|obs| {
            match serde_json::to_value(obs).and_then(serde_json::from_value::<GraphObservation>) {
                Ok(converted) => Some(converted),
                Err(err) => {
                    tracing::warn!(
                        target: LOG_TARGET,
                        error = %err,
                        "[flows] dropped an observation the exporter could not read — \
                         tinyflows and tinyagents GraphObservation have drifted"
                    );
                    None
                }
            }
        })
        .collect()
}

/// Exports one settled flow run to Langfuse as a single trace. Best-effort:
/// every failure path logs a `[flows]`-prefixed warning and returns — a
/// Langfuse outage can never fail or delay-fail the run. No-op when
/// `observability.share_usage_data` is off or there is nothing to send.
pub async fn export_flow_run_trace(
    config: &Config,
    flow_name: &str,
    flow_id: &str,
    thread_id: &str,
    status: &str,
    trigger: FlowRunTrigger,
    journal_observations: &[FlowObservation],
) {
    if !config.observability.share_usage_data {
        tracing::debug!(
            target: LOG_TARGET,
            flow_id = %flow_id,
            "[flows] langfuse export skipped: observability.share_usage_data is off"
        );
        return;
    }
    if journal_observations.is_empty() {
        tracing::debug!(
            target: LOG_TARGET,
            flow_id = %flow_id,
            thread_id = %thread_id,
            "[flows] langfuse export skipped: run journal is empty"
        );
        return;
    }

    let url = ingestion_url(config);
    if !url.starts_with("http") {
        tracing::warn!(
            target: LOG_TARGET,
            flow_id = %flow_id,
            "[flows] langfuse export skipped: could not resolve ingestion URL from backend host (got {url:?})"
        );
        return;
    }
    let token = match require_live_session_token(config) {
        Ok(token) => token,
        Err(err) => {
            tracing::warn!(
                target: LOG_TARGET,
                flow_id = %flow_id,
                error = %err,
                "[flows] langfuse export skipped: no live session token"
            );
            return;
        }
    };
    let client = match LangfuseClient::new(url.clone(), LangfuseAuth::Bearer { token }) {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(
                target: LOG_TARGET,
                flow_id = %flow_id,
                error = %err,
                "[flows] langfuse export skipped: could not build client for {url}"
            );
            return;
        }
    };

    let observations = to_exporter_observations(journal_observations);
    if observations.is_empty() {
        tracing::warn!(
            target: LOG_TARGET,
            flow_id = %flow_id,
            thread_id = %thread_id,
            "[flows] langfuse export skipped: no observation survived re-typing"
        );
        return;
    }

    let exporter = build_flow_exporter(client, flow_id);
    let trace = build_flow_trace_config(flow_name, flow_id, thread_id, status, trigger);
    let observation_count = observations.len();
    tracing::debug!(
        target: LOG_TARGET,
        flow_id = %flow_id,
        thread_id = %thread_id,
        status = %status,
        trigger = %trigger.as_str(),
        observation_count,
        endpoint = %exporter.endpoint(),
        "[flows] pushing flow run trace to Langfuse"
    );

    // `send_observations` already tolerates 207 Multi-Status; the outer
    // timeout caps a hung connection the same way the agent exporter does.
    match tokio::time::timeout(
        PUSH_TIMEOUT,
        exporter.send_observations(trace, &observations),
    )
    .await
    {
        Ok(Ok(_)) => {
            tracing::debug!(
                target: LOG_TARGET,
                flow_id = %flow_id,
                thread_id = %thread_id,
                observation_count,
                "[flows] pushed flow run trace to Langfuse"
            );
        }
        Ok(Err(err)) => {
            tracing::warn!(
                target: LOG_TARGET,
                flow_id = %flow_id,
                thread_id = %thread_id,
                error = %err,
                "[flows] langfuse export failed (run unaffected)"
            );
        }
        Err(_elapsed) => {
            tracing::warn!(
                target: LOG_TARGET,
                flow_id = %flow_id,
                thread_id = %thread_id,
                timeout_secs = PUSH_TIMEOUT.as_secs(),
                "[flows] langfuse export timed out (run unaffected)"
            );
        }
    }
}

#[cfg(test)]
#[path = "langfuse_export_tests.rs"]
mod tests;
