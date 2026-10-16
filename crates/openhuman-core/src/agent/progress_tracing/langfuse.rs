//! Langfuse proxy URL and environment resolution for agent trace export
//! (issue #4249 follow-up).
//!
//! When `observability.share_usage_data` is enabled, a completed run's spans
//! are POSTed as OTLP/HTTP JSON (see [`super::otlp`]) to the tracing proxy of
//! the host's backend, at the URL the host resolved onto the config
//! (`resolved_endpoints.tracing_otlp_url`, see [`otlp_url`]) so it always
//! follows the current backend hostname. The request reuses the session
//! bearer; the backend authenticates that JWT, stamps the user on every span
//! and holds the tracing project keys itself, so clients never hold them.
//!
//! Best-effort: any failure is logged and swallowed by the caller so tracing
//! never breaks a turn. Spans always carry metadata (names, kinds, timings,
//! and non-PII token/cost figures, promoted into Langfuse's native
//! usage/cost details). Prompt/reply text and truncated tool I/O ride along
//! only while `observability.agent_tracing.capture_content` is on; disabling
//! that flag withholds content and leaves metadata-only export.

mod environment;
mod journal_export;

pub(crate) use environment::{environment_for_base, otlp_url, skip_push};
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
