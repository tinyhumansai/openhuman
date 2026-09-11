//! Task-local carrier for the currently-executing agent's identity so
//! file tools can attribute reads/writes without widening the `Tool` trait.
//!
//! Follows the same pattern as `sandbox_context.rs`. Set by the agent
//! harness around tool execution; tools read via [`current_file_state_agent_id`].

tokio::task_local! {
    static FILE_STATE_AGENT_ID: String;
}

/// Returns the current agent's identity for file-state tracking, if set.
///
/// Returns `None` outside an agent turn (CLI, JSON-RPC direct, unit tests).
pub fn current_file_state_agent_id() -> Option<String> {
    FILE_STATE_AGENT_ID.try_with(|id| id.clone()).ok()
}

/// Run `future` with `agent_id` installed as the file-state identity.
pub async fn with_file_state_agent_id<F, R>(agent_id: String, future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    FILE_STATE_AGENT_ID.scope(agent_id, future).await
}

#[cfg(test)]
#[path = "agent_context_tests.rs"]
mod tests;
