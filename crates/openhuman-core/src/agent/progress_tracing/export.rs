//! Handing finished spans to their sink(s): the local NDJSON/log exporter and
//! the remote Langfuse usage-data push.

use crate::config::schema::AgentTracingConfig;
use crate::config::Config;

use super::langfuse;
use super::otlp;
use super::serialize::spans_to_ndjson;
use super::types::{TraceContext, TraceSpan};

/// Export finished spans per the [`AgentTracingConfig`]: append NDJSON to the
/// configured file, or emit to the application log when no path is set.
/// Best-effort — a failed write is logged and swallowed so tracing never
/// breaks an agent run. A no-op when tracing is disabled or there are no spans.
pub(crate) fn export_spans(config: &AgentTracingConfig, spans: &[TraceSpan]) {
    if !config.enabled || spans.is_empty() {
        return;
    }
    let payload = spans_to_ndjson(config.backend, spans);
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
///    push the run's spans to the backend Langfuse proxy — endpoint derived from
///    the current backend host, authed with the session bearer (see
///    [`otlp::push_spans`]). A failure (no live session, network, rejected
///    payload) just logs; there is no local fallback, since sharing and local
///    export are distinct opt-ins.
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
        if let Err(err) = otlp::push_spans(config, spans).await {
            log::warn!("[agent-tracing] Langfuse usage-data push failed ({err})");
        }
    }

    if observability.agent_tracing.enabled {
        export_spans(&observability.agent_tracing, spans);
    }
}

/// Export a completed run when durable tinyagents observations are available.
/// Remote usage-data sharing exports the live span tree as OTLP. The journal
/// is still read for local parity checks and future recovery work.
pub(crate) async fn export_run_trace_from_journal(
    config: &Config,
    _trace_ctx: &TraceContext,
    _observations: &[tinyagents_harness::observability::AgentObservation],
    _run_telemetry: Option<&tinyagents_session::run_ledger::RunTelemetry>,
    live_spans: &[TraceSpan],
) {
    if live_spans.is_empty() {
        return;
    }
    let observability = &config.observability;

    if observability.share_usage_data && !live_spans.is_empty() {
        if let Err(err) = otlp::push_spans(config, live_spans).await {
            log::warn!("[agent-tracing] Langfuse OTLP push failed ({err})");
        }
    }

    if observability.agent_tracing.enabled && !live_spans.is_empty() {
        export_spans(&observability.agent_tracing, live_spans);
    }
}

/// Export one completed child turn as a separate trace in its conversation.
/// The backend stamps the authenticated user onto the trace, while the child
/// run's original parent/root ids remain in trace metadata for navigation.
pub(crate) async fn export_subagent_journal_trace(
    config: &Config,
    journal_run_id: &str,
    thread_id: Option<&str>,
    agent_id: &str,
    task_id: &str,
) {
    if !config.observability.share_usage_data {
        return;
    }
    // Check the push gates before reading the child's journal: without a live
    // session the push refuses anyway, and the read and observation build were
    // pure cost — on every delegated turn, since usage sharing defaults on.
    if !langfuse::journal_push_ready(config) {
        log::debug!(
            "[agent-tracing] child trace export skipped: push not possible run_id={journal_run_id}"
        );
        return;
    }
    let observations = match crate::agent::tinyagents::journal::read_run_events(journal_run_id, 0)
        .await
    {
        Ok(observations) if !observations.is_empty() => observations,
        Ok(_) => return,
        Err(err) => {
            log::warn!("[agent-tracing] child journal read failed run_id={journal_run_id}: {err}");
            return;
        }
    };
    let first = &observations[0];
    let user_id = crate::security::credentials::identity::peek_credential_user_identity()
        .and_then(|identity| identity.id.or(identity.email));
    let trace_ctx = TraceContext::new(format!("subagent:{journal_run_id}"), user_id)
        .with_session_group(thread_id.unwrap_or(task_id).to_string())
        .with_agent_id(agent_id.to_string())
        .with_channel_source("subagent".to_string())
        .with_run_type(super::types::RunType::Subagent)
        .with_capture_content(config.observability.agent_tracing.capture_content)
        .with_run_lineage(
            Some(first.run_id.as_str().to_string()),
            first
                .parent_run_id
                .as_ref()
                .map(|id| id.as_str().to_string()),
            Some(first.root_run_id.as_str().to_string()),
        );
    let rooted = langfuse::root_subagent_observations(&observations);
    let mut spans =
        super::journal_projection::spans_from_observations(trace_ctx.clone(), 0, &rooted);
    if let Some(root) = spans.iter_mut().find(|span| span.parent_span_id.is_none()) {
        for (key, value) in [
            ("run_id", trace_ctx.run_id.as_deref()),
            ("parent_run_id", trace_ctx.parent_run_id.as_deref()),
            ("root_run_id", trace_ctx.root_run_id.as_deref()),
        ] {
            if let Some(value) = value {
                root.attributes
                    .insert(key.to_string(), serde_json::json!(value));
            }
        }
    }
    otlp::prepare_subagent_root(&mut spans);
    if let Err(err) = otlp::push_spans(config, &spans).await {
        log::warn!(
            "[agent-tracing] child Langfuse OTLP push failed run_id={journal_run_id}: {err}"
        );
    }
}
