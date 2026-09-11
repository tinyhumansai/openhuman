//! Tool: `agent_prepare_context` — "plan mode as a subagent".
//!
//! When a parent agent explicitly needs an ad hoc context pass, it can call
//! `agent_prepare_context`. This runs the read-only `context_scout` sub-agent
//! inline (blocking), which gathers context from memory, the user's
//! goals/profile, connected integrations, and the web, then returns a tight
//! `[context_bundle]` envelope: whether there's enough context to act, a
//! compact context summary, and an ordered set of recommended next tool calls
//! drawn from the *parent's own* tool catalogue.
//!
//! The scout's output is bounded by `context_scout`'s `max_result_chars`
//! (≈1000 tokens) so the parent's context only grows by a bounded amount.

#[cfg(test)]
#[path = "agent_prepare_context_tests.rs"]
mod tests;
include!("agent_prepare_context_part_01.rs");
include!("agent_prepare_context_part_02.rs");
