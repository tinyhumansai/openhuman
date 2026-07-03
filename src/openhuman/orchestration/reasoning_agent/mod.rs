//! The `reasoning_agent` built-in: the deep-thinking reasoning core (`execute`
//! node) of the orchestration wake graph. Registered in the built-in loader
//! ([`crate::openhuman::agent_registry::agents::loader`]).
//!
//! The current subconscious steering directive for a cycle is carried into the
//! agent's system prompt via a task-local ([`ORCHESTRATION_STEERING`]) that the
//! `execute` node scopes around the turn (see [`with_steering`]); `prompt::build`
//! reads it (or falls back to [`DEFAULT_STEERING`]).

pub mod graph;
pub mod prompt;

use std::future::Future;

tokio::task_local! {
    /// The active subconscious steering directive for the current wake cycle,
    /// scoped by the `execute` node around the reasoning agent's turn.
    static ORCHESTRATION_STEERING: String;
}

/// Default alignment directive used when no steering directive is active.
pub const DEFAULT_STEERING: &str = "No active steering directive. Stay aligned with the user's \
stated goals and prior context; prefer correctness and safety over speed.";

/// Scope `steering` for the duration of `fut` (the reasoning agent's turn), so
/// the prompt builder reads it. Box-pins the inner future to keep the combined
/// task-local + agent-loop future heap-allocated (same rationale as
/// [`crate::openhuman::agent::turn_origin::with_origin`]).
pub async fn with_steering<F: Future>(steering: String, fut: F) -> F::Output {
    ORCHESTRATION_STEERING.scope(steering, Box::pin(fut)).await
}

/// The current steering directive, or `None` when no cycle scoped one.
pub fn current_steering() -> Option<String> {
    ORCHESTRATION_STEERING.try_with(|s| s.clone()).ok()
}
