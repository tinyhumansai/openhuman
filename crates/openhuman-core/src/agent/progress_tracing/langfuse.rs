//! Langfuse proxy URL and environment resolution for agent trace export
//! (issue #4249 follow-up).
//!
//! When `observability.share_usage_data` is enabled, a completed run's spans
//! are POSTed as OTLP/HTTP JSON (see [`super::otlp`]) to the OpenHuman
//! backend's Langfuse **proxy**, `/telemetry/langfuse/otel/v1/traces`, which
//! sits beside the legacy `/telemetry/langfuse/ingestion` route
//! [`ingestion_url`] resolves from the **current backend hostname**
//! (`effective_backend_api_url`). The request reuses the OpenHuman **session
//! bearer**; the backend authenticates that JWT, stamps the user on every span,
//! injects the Langfuse project keys server-side, and forwards to Langfuse
//! v4's `/api/public/otel/v1/traces` (backend `src/services/langfuseProxy.ts`).
//! Clients never hold Langfuse keys. The legacy batch route is no longer used
//! for traces: Langfuse Cloud removes it on 2026-11-16.
//!
//! Best-effort: any failure is logged and swallowed by the caller so tracing
//! never breaks a turn. Spans always carry metadata (names, kinds, timings,
//! and non-PII token/cost figures, promoted into Langfuse's native
//! usage/cost details). Prompt/reply text and truncated tool I/O ride along
//! only while `observability.agent_tracing.capture_content` is on; disabling
//! that flag withholds content and leaves metadata-only export.

mod environment;
mod journal_export;

pub(crate) use environment::{environment_for_base, ingestion_url, skip_push};
pub(crate) use journal_export::journal_push_ready;

#[cfg(test)]
use crate::config::Config;
#[cfg(test)]
use environment::push_allowed;
#[cfg(test)]
use serde_json::json;
#[cfg(test)]
use tinyagents_harness::events::AgentEvent;
#[cfg(test)]
use tinyagents_harness::observability::trace_export::{SpanStatus, TraceSpan};
#[cfg(test)]
use tinyagents_harness::observability::AgentObservation;

const LOG_TARGET: &str = "agent-tracing::langfuse";

#[cfg(test)]
#[path = "langfuse_tests.rs"]
mod tests;
