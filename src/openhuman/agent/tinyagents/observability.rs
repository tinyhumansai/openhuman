//! Bridge the `tinyagents` harness event stream onto openhuman's
//! [`AgentProgress`] + cost tracker (issue #4249).
//!
//! tinyagents emits a typed [`AgentEvent`] stream (model started/delta/completed,
//! tool started/completed, usage) through an [`EventSink`] that callers attach
//! to a [`RunContext`]. This listener translates those into the same
//! `AgentProgress` events the legacy `run_turn_engine` produced — restoring the
//! live tool timeline, streaming text, and the cost/token footer on the
//! tinyagents path — and feeds per-call usage into the global cost tracker.

#[cfg(test)]
#[path = "observability_tests.rs"]
mod tests;
include!("observability_part_01.rs");
include!("observability_part_02.rs");
