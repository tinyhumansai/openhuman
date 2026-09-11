//! Build the turn, dispatch `agent.run_turn`, parse the reply.
//!
//! This is the core of the triage pipeline. It implements a tiered
//! fallback chain (issue #1257):
//!
//! ```text
//! cloud (initial)
//!   ├── 429 / transient (5xx / timeout / connection) ──► retry once
//!   │       └── still failing ──► local fallback
//!   └── ok ──► resolution_path = Cloud | CloudAfterRetry
//!
//! local fallback
//!   ├── ok ──► resolution_path = LocalFallback
//!   └── failed ──► TriageOutcome::Deferred { until_ms, reason }
//! ```
//!
//! Non-transient cloud failures (auth, malformed prompt, model not
//! found) bubble up immediately — there's no point retrying them and
//! the local arm wouldn't help either. Malformed classifier replies
//! are treated like retryable cloud failures: retry once, then fall
//! through to local / Deferred.
//!
//! ## Why the turn path doesn't care about `tools_registry = []`
//!
//! The triage agent has `named = []` in its TOML (zero tools). The
//! tinyagents-backed turn path (`run_turn_via_tinyagents_shared` in
//! `src/openhuman/agent/tinyagents/mod.rs`) handles an empty registry by simply
//! sending no tool schemas to the backend — the turn degrades to a plain
//! chat completion.

#[cfg(test)]
#[path = "evaluator_tests.rs"]
mod tests;
include!("evaluator_part_01.rs");
include!("evaluator_part_02.rs");
