//! Structured tracing export for one bridged turn (#3886).
//!
//! When enabled, the bridge folds the turn's progress stream into
//! OTel/Langfuse-style spans correlated by session id (falling back to the
//! thread id for headless/autonomous runs). Disabled (both the remote and the
//! local exporter off), it costs nothing.

use crate::agent::progress_tracing::SpanCollector;
use tinyagents_harness::observability::trace_export as te;

use super::super::types::ChatRequestMetadata;

/// The span collector for the turn, and the trace context the journal export
/// reuses after the turn ends. `None` when tracing is disabled.
pub(super) fn span_collector(
    config: &crate::config::Config,
    metadata: &ChatRequestMetadata,
    client_id: &str,
    thread_id: &str,
    request_id: &str,
) -> Option<(SpanCollector, te::TraceContext)> {
    if !(config.observability.share_usage_data || config.observability.agent_tracing.enabled) {
        return None;
    }
    // One trace per turn: the trace id is unique per request, while the
    // thread id rides along as the Langfuse `sessionId` so a
    // conversation's per-turn traces still group under one session.
    let base = te::trace_session_id(metadata.session_id, thread_id);
    let trace_id = format!("{base}:{request_id}");
    // Attribute the trace to the *real* authenticated user (cached
    // stored credential identity: id, else email) — the transport client
    // id (socket client / "system") is NOT a user; it rides along as
    // the separate `client.id` metadata attribute. The backend stamps
    // the authenticated JWT user on every accepted trace.
    let identity = crate::security::credentials::identity::peek_credential_user_identity();
    let user_id = identity
        .and_then(|i| i.id.or(i.email))
        .or_else(|| super::session_profile_user_attribution(config));
    let user_attributed = user_id.is_some();
    // Run origin for trace metadata: the request's source tag
    // ("ptt"/"dictation"/"type"/"autonomous"/…), else a
    // plain interactive chat turn.
    let run_type = te::RunType::from_source(metadata.source.as_deref());
    let channel_source = metadata
        .source
        .clone()
        .unwrap_or_else(|| "chat".to_string());
    // Storage-level privacy gate (#4454): capture_content (off by
    // default) rides on the TraceContext so the collector only attaches
    // prompt/reply content to spans when the operator opted in — no
    // exporter can serialize prompt/reply text otherwise.
    let capture_content = config.observability.agent_tracing.capture_content;
    log::debug!(
        "[web_channel][bridge] trace context trace_id={} user_attributed={} \
         agent_id={:?} channel_source={} run_type={} capture_content={} request_id={}",
        trace_id,
        user_attributed,
        metadata.agent_id,
        channel_source,
        run_type.as_str(),
        capture_content,
        request_id,
    );
    let mut trace_ctx = te::TraceContext::new(trace_id, user_id)
        .with_session_group(thread_id.to_string())
        .with_client_id(client_id.to_string())
        .with_channel_source(channel_source)
        .with_run_type(run_type)
        .with_capture_content(capture_content);
    if let Some(agent_id) = metadata.agent_id.clone() {
        trace_ctx = trace_ctx.with_agent_id(agent_id);
    }
    Some((SpanCollector::new(trace_ctx.clone()), trace_ctx))
}
