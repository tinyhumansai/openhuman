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
//!   └── failed ──► TriageOutcome::Terminal { reason }
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
//! `crates/openhuman-core/src/agent/tinyagents/mod.rs`) handles an empty registry by simply
//! sending no tool schemas to the backend — the turn degrades to a plain
//! chat completion.

mod arm;
mod chain;
mod outcome;
mod prompt;

pub use arm::TRIGGER_TRIAGE_AGENT_ID;
#[cfg(test)]
pub(crate) use chain::begin_outage_attempt;
#[cfg(test)]
pub(crate) use chain::record_outage;
#[cfg(test)]
pub(crate) use chain::run_triage_with_arms_for_test_with_state;
pub use chain::{run_triage, run_triage_with_arms};
pub use outcome::{TriageOutcome, TriageResolutionPath, TriageRun};

#[cfg(test)]
use super::envelope::TriggerEnvelope;
#[cfg(test)]
use super::routing::ResolvedProvider;
#[cfg(test)]
use crate::agent::harness::definition::{AgentDefinition, PromptSource};
#[cfg(test)]
pub(crate) use arm::{classify_error, ArmError};
#[cfg(test)]
pub(crate) use chain::run_triage_with_arms_for_test;
#[cfg(test)]
pub(crate) use prompt::{extract_inline_prompt, render_user_message, truncate_payload};

#[cfg(test)]
#[path = "evaluator_tests.rs"]
mod tests;
