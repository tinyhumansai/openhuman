//! Langfuse ingestion exporter for agent trace spans (issue #4249 follow-up).
//!
//! When `observability.share_usage_data` is enabled, a completed run's spans are POSTed to the OpenHuman
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
//! disabling that flag withholds content and leaves metadata-only export.

use std::time::Duration;

mod environment;
#[allow(dead_code)] // Legacy batch exporter remains for compatibility tests.
mod ingestion_batch;
#[allow(dead_code)] // Child-run rooting is still shared with the OTLP path.
mod journal_export;
#[allow(dead_code)] // Legacy batch exporter remains for compatibility tests.
mod span_export;

pub(crate) use environment::{environment_for_base, ingestion_url, skip_push};
pub(crate) use journal_export::journal_push_ready;
#[cfg(test)]
pub(crate) use journal_export::push_observations;
pub(crate) use journal_export::root_subagent_observations;
#[cfg(test)]
pub(crate) use span_export::push_spans;

use super::{SpanStatus, TraceContext, TraceSpan};

#[cfg(test)]
use crate::config::Config;
#[cfg(test)]
use environment::push_allowed;
#[cfg(test)]
use ingestion_batch::{iso_millis, split_ingestion_batch};
#[cfg(test)]
use journal_export::{
    insert_run_telemetry_generation, observations_for_export, trace_config_from_context,
    trace_ctx_with_run_lineage,
};
#[cfg(test)]
use serde_json::{json, Value};
#[cfg(test)]
use std::borrow::Cow;
#[cfg(test)]
use tinyagents_harness::events::AgentEvent;
#[cfg(test)]
use tinyagents_harness::observability::{AgentObservation, LangfuseClient};
#[cfg(test)]
use tinyagents_session::run_ledger::RunTelemetry;

const LOG_TARGET: &str = "agent-tracing::langfuse";
/// Cap the push so a slow/hung Langfuse never stalls run teardown.
const PUSH_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(test)]
#[path = "langfuse_tests.rs"]
mod tests;
