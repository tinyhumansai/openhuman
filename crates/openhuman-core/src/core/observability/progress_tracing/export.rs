//! Handing finished spans to their sink(s): the local NDJSON/log exporter and
//! the usage-data [`TraceSink`](super::TraceSink)s a host registered.

use crate::config::schema::AgentTracingConfig;
use crate::config::Config;

use crate::config::schema::AgentTracingBackend;
use tinyagents_harness::observability::trace_export::serialize::{spans_to_ndjson, SpanEnvelope};
use tinyagents_harness::observability::trace_export::{TraceContext, TraceSpan};
use tinyagents_harness::observability::AgentObservation;

/// Hand `spans` to `sinks`. Best-effort: a failure is logged per sink and never
/// reaches the run.
async fn push_to_sinks(
    sinks: Vec<(String, std::sync::Arc<dyn super::TraceSink>)>,
    config: &Config,
    spans: &[TraceSpan],
    label: &str,
) {
    if spans.is_empty() {
        return;
    }
    if sinks.is_empty() {
        log::debug!("[agent-tracing] {label}: no usage-data trace sink registered");
        return;
    }
    for (id, sink) in sinks {
        if let Err(err) = sink.push(config, spans).await {
            log::warn!("[agent-tracing] {label} push to sink {id} failed ({err})");
        }
    }
}

/// Map the configured backend onto the upstream NDJSON envelope.
pub(crate) fn envelope_for(backend: AgentTracingBackend) -> SpanEnvelope {
    match backend {
        AgentTracingBackend::Otel => SpanEnvelope::Otel,
        AgentTracingBackend::Langfuse => SpanEnvelope::Langfuse,
    }
}

/// Export finished spans per the [`AgentTracingConfig`]: append NDJSON to the
/// configured file, or emit to the application log when no path is set.
/// Best-effort — a failed write is logged and swallowed so tracing never
/// breaks an agent run. A no-op when tracing is disabled or there are no spans.
pub(crate) fn export_spans(config: &AgentTracingConfig, spans: &[TraceSpan]) {
    if !config.enabled || spans.is_empty() {
        return;
    }
    let payload = spans_to_ndjson(envelope_for(config.backend), spans);
    match &config.export_path {
        Some(path) => {
            use std::io::Write as _;
            let opened = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path);
            match opened {
                Ok(mut file) => {
                    if let Err(err) = file.write_all(payload.as_bytes()) {
                        log::warn!(
                            "[agent-tracing] failed to append {} spans to {path}: {err}",
                            spans.len()
                        );
                    } else {
                        log::debug!("[agent-tracing] exported {} spans to {path}", spans.len());
                    }
                }
                Err(err) => log::warn!("[agent-tracing] failed to open {path}: {err}"),
            }
        }
        None => {
            // No path configured. Surface only metadata (count + trace id) at
            // `info` so the export is visible on read-only / sandboxed
            // deployments WITHOUT ever printing span content at `info` (#4454).
            // The NDJSON body — which may carry prompt/reply text when
            // `capture_content` is opted in — goes to `debug` only. With the
            // default gate off, the storage layer already strips content, so
            // `payload` is metadata-only regardless.
            log::info!(
                "[agent-tracing] {} spans (trace_id={}) — set observability.agent_tracing.export_path to persist",
                spans.len(),
                spans.first().map(|s| s.trace_id.as_str()).unwrap_or(""),
            );
            log::debug!(
                target: "agent-tracing",
                "[agent-tracing] span NDJSON ({} spans):\n{}",
                spans.len(),
                payload.trim_end()
            );
        }
    }
}

/// Hand a completed run's spans to the configured tracing sink(s).
///
/// Two independent paths, both best-effort and never fatal to a turn:
///
/// 1. **Usage-data sharing** (`observability.share_usage_data`, on by default):
///    hand the run's spans to every registered [`TraceSink`](super::TraceSink)
///    (the TinyHumans layer registers its Langfuse proxy push). A failure (no
///    live session, network, rejected payload) just logs; there is no local
///    fallback, since sharing and local export are distinct opt-ins.
/// 2. **Local exporter** (`observability.agent_tracing.enabled`, opt-in): append
///    OTel/Langfuse-format NDJSON to the configured file or the app log via
///    [`export_spans`].
///
/// A no-op when there are no spans or both paths are off.
pub(crate) async fn export_run_trace(config: &Config, spans: &[TraceSpan]) {
    if spans.is_empty() {
        return;
    }
    let observability = &config.observability;

    if observability.share_usage_data {
        push_to_sinks(super::sinks::sinks(), config, spans, "usage-data").await;
    }

    if observability.agent_tracing.enabled {
        export_spans(&observability.agent_tracing, spans);
    }
}

/// Export a completed run by collecting its durable native observations.
pub(crate) async fn export_run_trace_from_journal(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    request_id: &str,
    trace_ctx: TraceContext,
) {
    let observability = &config.observability;
    if !observability.share_usage_data && !observability.agent_tracing.enabled {
        return;
    }
    let Some(root_run_id) =
        crate::agent::tinyagents::journal::take_request_journal_run(ctx, request_id)
    else {
        log::debug!("[agent-tracing] no durable root run registered request_id={request_id}");
        return;
    };
    let observations =
        match crate::agent::tinyagents::journal::read_run_tree_events(ctx, &root_run_id).await {
            Ok(observations) if !observations.is_empty() => observations,
            Ok(_) => return,
            Err(err) => {
                log::warn!("[agent-tracing] trace journal read failed run_id={root_run_id}: {err}");
                return;
            }
        };
    let Some(root) = observations.iter().find(|observation| {
        observation.run_id.as_str() == root_run_id && observation.parent_run_id.is_none()
    }) else {
        log::warn!("[agent-tracing] trace root observation missing run_id={root_run_id}");
        return;
    };
    let trace_ctx = trace_ctx.with_run_lineage(
        Some(root_run_id.clone()),
        None,
        Some(root.root_run_id.as_str().to_owned()),
    );
    let mut collector =
        tinyagents_harness::observability::trace_export::collector::SpanCollector::new(trace_ctx);
    for observation in &observations {
        collector.record(observation);
    }
    let finished_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    collector.finish(finished_at_ms);
    let spans = collector.spans().to_vec();
    export_run_trace(config, &spans).await;
}

/// Export one completed child turn as a distinct trace in its conversation.
/// The child journal retains its original parent/root lineage in trace metadata
/// while its observations are rooted locally for span construction.
pub(crate) async fn export_subagent_journal_trace(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    journal_run_id: &str,
    thread_id: Option<&str>,
    agent_id: &str,
    task_id: &str,
) {
    if !config.observability.share_usage_data {
        return;
    }
    let ready = super::sinks::ready_sinks(config).await;
    if ready.is_empty() {
        log::debug!(
            "[agent-tracing] child trace export skipped: no ready sink run_id={journal_run_id}"
        );
        return;
    }
    let observations =
        match crate::agent::tinyagents::journal::read_run_events(ctx, journal_run_id, 0).await {
            Ok(observations) if !observations.is_empty() => observations,
            Ok(_) => return,
            Err(err) => {
                log::warn!(
                    "[agent-tracing] child journal read failed run_id={journal_run_id}: {err}"
                );
                return;
            }
        };
    let spans = build_subagent_trace(
        &observations,
        thread_id,
        agent_id,
        task_id,
        config.observability.agent_tracing.capture_content,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64,
    );
    push_to_sinks(ready, config, &spans, "child trace").await;
}

pub(crate) fn build_subagent_trace(
    observations: &[AgentObservation],
    thread_id: Option<&str>,
    agent_id: &str,
    task_id: &str,
    capture_content: bool,
    finished_at_ms: u64,
) -> Vec<TraceSpan> {
    let Some(first) = observations.first() else {
        return Vec::new();
    };
    let user_id = crate::credentials::identity::peek_credential_user_identity()
        .and_then(|identity| identity.id.or(identity.email));
    let trace_ctx = TraceContext::new(format!("subagent:{}", first.run_id.as_str()), user_id)
        .with_session_group(thread_id.unwrap_or(task_id).to_string())
        .with_agent_id(agent_id.to_string())
        .with_channel_source("subagent".to_string())
        .with_run_type(tinyagents_harness::observability::trace_export::RunType::Subagent)
        .with_capture_content(capture_content)
        .with_run_lineage(
            Some(first.run_id.as_str().to_string()),
            first
                .parent_run_id
                .as_ref()
                .map(|id| id.as_str().to_string()),
            Some(first.root_run_id.as_str().to_string()),
        );
    let rooted =
        tinyagents_harness::observability::trace_export::journal_export::root_subagent_observations(
            observations,
        );
    let observation_root = first.run_id.clone();
    let mut collector =
        tinyagents_harness::observability::trace_export::collector::SpanCollector::new(trace_ctx)
            .with_observation_root(observation_root);
    for observation in &rooted {
        collector.record(observation);
    }
    collector.finish(finished_at_ms);
    let mut spans = collector.spans().to_vec();
    tinyagents_harness::observability::trace_export::otlp::prepare_subagent_root(&mut spans);
    spans
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
