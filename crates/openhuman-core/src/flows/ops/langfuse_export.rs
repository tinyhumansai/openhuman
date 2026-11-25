//! Langfuse export for `flows::` graph runs.
//!
//! After a `flows_run` / `flows_resume` settles, the `flows::` domain hands
//! this module the run's [`GraphObservation`] slice collected from the Runs
//! bus event stream, and it exports the run as one Langfuse trace via the backend's Langfuse
//! OTLP proxy route (Langfuse v4; the legacy batch ingestion path is not used
//! for traces).
//!
//! The trace mapper turns TinyFlows run observations into timed generic
//! harness spans, stamps the Langfuse **Agent Graph view** keys
//! (`langgraph_node` / `langgraph_step`) on node spans, and projects them to
//! v4 OTLP: trace name, session, tags, release and flow metadata ride on every
//! span, since v4 filters observations directly. A host span-metadata
//! injector additionally stamps `flow_id` on every span for filtering.
//!
//! Transport mirrors the agent-turn tracing path
//! (`core::observability::progress_tracing::otlp::push_spans`): the endpoint is derived
//! from the **current backend hostname** (`effective_backend_api_url`), auth
//! is the live OpenHuman session bearer (the backend injects the real
//! Langfuse keys server-side, stamps the authenticated user on every span and
//! adds `x-langfuse-ingestion-version: 4`), the send is capped at 10s, a
//! partial OTLP rejection counts as a failure, and every failure is logged and
//! swallowed — exporting is best-effort and never fails the run. Gated on
//! `observability.share_usage_data`.

use std::time::Duration;

use std::collections::{HashMap, VecDeque};

use serde_json::{json, Value};
use tinyagents_harness::observability::spans::{Span, SpanExporter, SpanState};
use tinyagents_harness::{LangfuseAuth, LangfuseClient, LangfuseTraceConfig};
use tinyflows::engine::GraphObservation;
use tinyflows::graph::stream::GraphEvent;
use tinyflows_bus::runs::FlowRunTrigger;

use crate::config::Config;
use crate::credentials::backend_credential::direct_backend_credential;

const LOG_TARGET: &str = "flows::langfuse";
/// Backend proxy route for legacy Langfuse ingestion (relative to the backend
/// origin). Only the client's base: [`LangfuseClient::otlp_endpoint`] derives
/// the OTLP route beside it, which is
/// where flow traces go. The backend authenticates the session JWT, injects
/// the Langfuse project keys, and forwards to Langfuse's
/// `/api/public/otel/v1/traces`.
/// Cap the push so a slow/hung Langfuse never stalls run teardown (same
/// posture as the agent-turn exporter).
const PUSH_TIMEOUT: Duration = Duration::from_secs(10);

fn trigger_name(trigger: FlowRunTrigger) -> &'static str {
    match trigger {
        FlowRunTrigger::Rpc => "rpc",
        FlowRunTrigger::Schedule => "schedule",
        FlowRunTrigger::AppEvent => "app_event",
        FlowRunTrigger::Resume => "resume",
    }
}

/// Resolve the Langfuse ingestion URL from the current backend host — the
/// exact base-server resolution every other backend call uses, so the host
/// always matches wherever the app's domain calls go (staging, prod, or a
/// custom `api_url` override).
fn ingestion_url(config: &Config) -> String {
    // Empty without a resolved ingestion URL; callers treat a non-`http` URL
    // as "proxy unavailable" and skip the push.
    config
        .resolved_endpoints
        .tracing_ingest_url
        .clone()
        .unwrap_or_default()
}

/// The OpenHuman core crate version (e.g. `0.58.0`), stamped onto every flow
/// trace as the Langfuse `release` field plus an `app_version` metadata key so
/// traces can be correlated with the app build that produced them.
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Builds the [`LangfuseTraceConfig`] for one flow run: the trace id is the
/// run's `thread_id` (`flow:{flow_id}:{uuid}`), while the session id is the
/// stable `flow:{flow_id}` so repeated runs group together. The trace
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
    let trigger = trigger_name(trigger);
    LangfuseTraceConfig {
        trace_id: Some(thread_id.to_string()),
        name: Some(format!("flow.run:{flow_name}")),
        session_id: Some(format!("flow:{flow_id}")),
        release: Some(APP_VERSION.to_string()),
        tags: vec!["run:flow".to_string(), format!("trigger:{trigger}")],
        metadata: json!({
            "flow_id": flow_id,
            "status": status,
            "source": "flows",
            "run_type": "flow",
            "trigger": trigger,
            "app_version": APP_VERSION,
        }),
        ..Default::default()
    }
}

/// Converts TinyFlows' durable events to the generic harness span contract.
/// Pairing is FIFO so repeated node activations keep their event order.
fn flow_spans(flow_id: &str, observations: &[GraphObservation]) -> Vec<Span> {
    let Some(first) = observations.first() else {
        return Vec::new();
    };
    let trace_id = first
        .thread_id
        .as_ref()
        .map(|thread| thread.as_str())
        .unwrap_or_else(|| first.root_run_id.as_str());
    let run_id = first.run_id.as_str();
    let root_run_id = first.root_run_id.as_str();
    let run_span_id = format!("{trace_id}:run:{root_run_id}");
    let terminal = observations.iter().rev().find(|obs| {
        matches!(
            obs.event,
            GraphEvent::RunCompleted { .. } | GraphEvent::RunFailed { .. }
        )
    });
    let mut run_span = Span::new(
        run_span_id.clone(),
        run_id,
        first.graph_id.as_str(),
        first.ts_ms,
    )
    .with_root_run_id(root_run_id)
    .with_event_id(terminal.unwrap_or(first).event_id.as_str());
    if let Some(end) = terminal.or_else(|| observations.last()) {
        run_span = run_span.finish(
            end.ts_ms,
            if matches!(end.event, GraphEvent::RunFailed { .. }) {
                SpanState::Error
            } else {
                SpanState::Ok
            },
        );
    }
    add_observation_metadata(&mut run_span, first, flow_id);

    let mut output = vec![run_span];
    let mut consumed = vec![false; observations.len()];
    let mut step_starts: HashMap<usize, VecDeque<usize>> = HashMap::new();
    let mut node_starts: HashMap<(String, usize), VecDeque<usize>> = HashMap::new();
    let mut subgraph_starts: HashMap<(String, Vec<String>), VecDeque<usize>> = HashMap::new();

    for (index, observation) in observations.iter().enumerate() {
        match &observation.event {
            GraphEvent::StepStarted { step, .. } => {
                step_starts.entry(*step).or_default().push_back(index);
                consumed[index] = true;
            }
            GraphEvent::StepCompleted { step } => {
                if let Some(start) = pop_start(&mut step_starts, step) {
                    consumed[index] = true;
                    output.push(step_span(
                        flow_id,
                        trace_id,
                        &run_span_id,
                        *step,
                        observations,
                        start,
                        index,
                    ));
                }
            }
            GraphEvent::NodeStarted { node, step } => {
                node_starts
                    .entry((node.as_str().to_owned(), *step))
                    .or_default()
                    .push_back(index);
                consumed[index] = true;
            }
            GraphEvent::NodeCompleted { node, step } => {
                if let Some(start) = pop_start(&mut node_starts, &(node.as_str().to_owned(), *step))
                {
                    consumed[index] = true;
                    output.push(node_span(
                        flow_id,
                        trace_id,
                        node.as_str(),
                        *step,
                        observations,
                        start,
                        index,
                        None,
                    ));
                }
            }
            GraphEvent::NodeFailed { node, step, error } => {
                if let Some(start) = pop_start(&mut node_starts, &(node.as_str().to_owned(), *step))
                {
                    consumed[index] = true;
                    output.push(node_span(
                        flow_id,
                        trace_id,
                        node.as_str(),
                        *step,
                        observations,
                        start,
                        index,
                        Some(error),
                    ));
                }
            }
            GraphEvent::SubgraphStarted { node, namespace } => {
                subgraph_starts
                    .entry((node.as_str().to_owned(), namespace.clone()))
                    .or_default()
                    .push_back(index);
                consumed[index] = true;
            }
            GraphEvent::SubgraphCompleted { node, namespace } => {
                if let Some(start) = pop_start(
                    &mut subgraph_starts,
                    &(node.as_str().to_owned(), namespace.clone()),
                ) {
                    consumed[index] = true;
                    output.push(subgraph_span(
                        flow_id,
                        trace_id,
                        &run_span_id,
                        node.as_str(),
                        namespace,
                        observations,
                        start,
                        index,
                    ));
                }
            }
            _ => {}
        }
    }
    for (step, starts) in step_starts {
        for start in starts {
            output.push(step_span(
                flow_id,
                trace_id,
                &run_span_id,
                step,
                observations,
                start,
                start,
            ));
            output.last_mut().expect("just pushed").end_ms = None;
        }
    }
    for ((node, step), starts) in node_starts {
        for start in starts {
            output.push(node_span(
                flow_id,
                trace_id,
                &node,
                step,
                observations,
                start,
                start,
                None,
            ));
            output.last_mut().expect("just pushed").end_ms = None;
        }
    }
    for ((node, namespace), starts) in subgraph_starts {
        for start in starts {
            output.push(subgraph_span(
                flow_id,
                trace_id,
                &run_span_id,
                &node,
                &namespace,
                observations,
                start,
                start,
            ));
            output.last_mut().expect("just pushed").end_ms = None;
        }
    }
    for (index, observation) in observations.iter().enumerate() {
        if consumed[index] || matches!(observation.event, GraphEvent::RunStarted { .. }) {
            continue;
        }
        let failed = matches!(observation.event, GraphEvent::RunFailed { .. });
        let error = match &observation.event {
            GraphEvent::RunFailed { error, .. } | GraphEvent::NodeFailed { error, .. } => {
                Some(error.as_str())
            }
            _ => None,
        };
        let mut span = Span::new(
            observation.event_id.as_str(),
            observation.run_id.as_str(),
            observation.event.kind(),
            observation.ts_ms,
        )
        .with_root_run_id(observation.root_run_id.as_str())
        .with_parent(&run_span_id)
        .with_event_id(observation.event_id.as_str());
        span.kind = "event-create".to_owned();
        span.status = if failed {
            SpanState::Error
        } else {
            SpanState::Unset
        };
        span.status_message = error.map(str::to_owned);
        add_observation_metadata(&mut span, observation, flow_id);
        output.push(span);
    }
    output
}

fn pop_start<K: std::hash::Hash + Eq>(
    starts: &mut HashMap<K, VecDeque<usize>>,
    key: &K,
) -> Option<usize> {
    let queue = starts.get_mut(key)?;
    let start = queue.pop_front();
    if queue.is_empty() {
        starts.remove(key);
    }
    start
}

fn add_observation_metadata(span: &mut Span, observation: &GraphObservation, flow_id: &str) {
    span.metadata.insert("flow_id".into(), json!(flow_id));
    span.metadata
        .insert("graph_id".into(), json!(observation.graph_id.as_str()));
    span.metadata
        .insert("run_id".into(), json!(observation.run_id.as_str()));
    span.metadata.insert(
        "root_run_id".into(),
        json!(observation.root_run_id.as_str()),
    );
    span.metadata.insert(
        "parent_run_id".into(),
        json!(observation.parent_run_id.as_ref().map(|id| id.as_str())),
    );
    span.metadata.insert(
        "checkpoint_id".into(),
        json!(observation.checkpoint_id.as_ref().map(|id| id.as_str())),
    );
    span.metadata
        .insert("namespace".into(), json!(observation.namespace));
    span.metadata.insert("step".into(), json!(observation.step));
    span.metadata
        .insert("offset".into(), json!(observation.offset));
    span.metadata
        .insert("event".into(), json!(observation.event));
}

fn step_span(
    flow_id: &str,
    trace_id: &str,
    run_span_id: &str,
    step: usize,
    observations: &[GraphObservation],
    start: usize,
    end: usize,
) -> Span {
    let first = &observations[start];
    let last = &observations[end];
    let mut span = Span::new(
        format!("{trace_id}:step:{step}"),
        first.run_id.as_str(),
        format!("step {step}"),
        first.ts_ms,
    )
    .with_root_run_id(first.root_run_id.as_str())
    .with_parent(run_span_id)
    .with_event_id(last.event_id.as_str());
    if end != start {
        span = span.finish(last.ts_ms, SpanState::Ok);
    }
    add_observation_metadata(&mut span, first, flow_id);
    span.metadata.insert("langgraph_step".into(), json!(step));
    span
}

fn node_span(
    flow_id: &str,
    trace_id: &str,
    node: &str,
    step: usize,
    observations: &[GraphObservation],
    start: usize,
    end: usize,
    error: Option<&str>,
) -> Span {
    let first = &observations[start];
    let last = &observations[end];
    let mut span = Span::new(
        format!("{trace_id}:node:{node}:{step}"),
        first.run_id.as_str(),
        node,
        first.ts_ms,
    )
    .with_root_run_id(first.root_run_id.as_str())
    .with_parent(format!("{trace_id}:step:{step}"))
    .with_event_id(last.event_id.as_str());
    if end != start {
        span = span.finish(
            last.ts_ms,
            if error.is_some() {
                SpanState::Error
            } else {
                SpanState::Ok
            },
        );
    }
    span.status_message = error.map(str::to_owned);
    add_observation_metadata(&mut span, last, flow_id);
    span.metadata.insert("langgraph_node".into(), json!(node));
    span.metadata.insert("langgraph_step".into(), json!(step));
    span
}

fn subgraph_span(
    flow_id: &str,
    trace_id: &str,
    run_span_id: &str,
    node: &str,
    namespace: &[String],
    observations: &[GraphObservation],
    start: usize,
    end: usize,
) -> Span {
    let first = &observations[start];
    let last = &observations[end];
    let mut span = Span::new(
        format!("{trace_id}:subgraph:{}", namespace.join("/")),
        first.run_id.as_str(),
        format!("subgraph {node}"),
        first.ts_ms,
    )
    .with_root_run_id(first.root_run_id.as_str())
    .with_parent(run_span_id)
    .with_event_id(last.event_id.as_str());
    if end != start {
        span = span.finish(last.ts_ms, SpanState::Ok);
    }
    add_observation_metadata(&mut span, last, flow_id);
    span
}

fn flow_otlp_requests(
    exporter: &SpanExporter,
    trace: LangfuseTraceConfig,
    flow_id: &str,
    observations: &[GraphObservation],
) -> tinyagents_harness::error::Result<Vec<Value>> {
    exporter.build_otlp_requests(
        trace,
        &flow_spans(flow_id, observations),
        "openhuman",
        "openhuman.graph",
    )
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
    observations: &[GraphObservation],
) {
    if !config.observability.share_usage_data {
        tracing::debug!(
            target: LOG_TARGET,
            flow_id = %flow_id,
            "[flows] langfuse export skipped: observability.share_usage_data is off"
        );
        return;
    }
    if observations.is_empty() {
        tracing::debug!(
            target: LOG_TARGET,
            flow_id = %flow_id,
            thread_id = %thread_id,
            "[flows] langfuse export skipped: PollRun observations are empty"
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
    // No TinyHumans connection, or no usable credential (signed out, offline
    // local session): a configured state — `direct_backend_credential` logs
    // the reason at debug, and the export is skipped without a request.
    let token = match direct_backend_credential(config, "flows langfuse export").await {
        Some(crate::credentials::backend_credential::BackendCredential::Session(token)) => token,
        _ => {
            tracing::debug!(
                target: LOG_TARGET,
                flow_id = %flow_id,
                "[flows] langfuse export skipped: hosted backend not available"
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

    let exporter = SpanExporter::new(client);
    let trace = build_flow_trace_config(flow_name, flow_id, thread_id, status, trigger);
    let observation_count = observations.len();
    tracing::debug!(
        target: LOG_TARGET,
        flow_id = %flow_id,
        thread_id = %thread_id,
        status = %status,
        trigger = %trigger_name(trigger),
        observation_count,
        endpoint = %exporter.client().otlp_endpoint(),
        "[flows] pushing flow run trace to Langfuse"
    );

    let requests = match flow_otlp_requests(&exporter, trace, flow_id, observations) {
        Ok(requests) => requests,
        Err(err) => {
            tracing::warn!(
                target: LOG_TARGET,
                flow_id = %flow_id,
                thread_id = %thread_id,
                error = %err,
                "[flows] langfuse export skipped: could not build OTLP requests"
            );
            return;
        }
    };
    // The generic exporter owns OTLP conversion and the authenticated client;
    // this adapter supplies flow-specific spans and trace metadata.
    let send = async {
        for request in requests {
            exporter.client().send_otlp(request).await?;
        }
        Ok::<(), tinyagents_harness::error::TinyAgentsError>(())
    };
    match tokio::time::timeout(PUSH_TIMEOUT, send).await {
        Ok(Ok(())) => {
            tracing::debug!(
                target: LOG_TARGET,
                flow_id = %flow_id,
                thread_id = %thread_id,
                observation_count,
                request_count = 1,
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
