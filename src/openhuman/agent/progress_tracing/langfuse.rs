//! Langfuse ingestion exporter for agent trace spans (issue #4249 follow-up).
//!
//! When `[observability.agent_tracing]` has `enabled = true` and
//! `backend = "langfuse"`, a completed run's spans are POSTed to the OpenHuman
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
//! with the default off, content is withheld and export stays metadata-only.

#[cfg(test)]
#[path = "langfuse_tests.rs"]
mod tests;
include!("langfuse_part_01.rs");
include!("langfuse_part_02.rs");
