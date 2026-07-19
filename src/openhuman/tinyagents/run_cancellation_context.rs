//! Task-local carrier for the current TinyAgents run cancellation token.
//!
//! TinyAgents keeps cooperative cancellation on `RunContext`, while the
//! tool-visible `ToolExecutionContext` intentionally exposes only stable run
//! metadata. OpenHuman tools that need to fan out nested graph work can read this
//! scoped token and pass the same live cancellation signal into graph helpers.

use tinyagents::CancellationToken;

tokio::task_local! {
    static CURRENT_RUN_CANCELLATION: CancellationToken;
}

/// Cancellation token for the current TinyAgents run, when executing inside the
/// shared harness drive.
pub(crate) fn current_run_cancellation() -> Option<CancellationToken> {
    CURRENT_RUN_CANCELLATION.try_with(Clone::clone).ok()
}

/// Run `future` with `token` installed as the current TinyAgents run
/// cancellation signal.
pub(crate) async fn with_run_cancellation<F, R>(token: CancellationToken, future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    CURRENT_RUN_CANCELLATION.scope(token, future).await
}
