//! Task-local carrier for a **task-recency window** so the Composio tool
//! surface can restrict task-fetch results to "recently created/changed"
//! without widening the [`tinytools::Tool`] trait signature.
//!
//! This Composio product policy is installed by the morning briefing runner
//! and applied to curated task-fetch actions through the connector bus.
//!
//! Why a task-local instead of a `Tool::execute` argument: the tool trait is
//! invoked from many call sites (CLI, JSON-RPC, tests, agent loops). A
//! task-local keeps the additive path scoped to the one agent runtime that
//! needs it — today only the `morning_briefing` cron agent installs a window.
//!
//! When the task-local isn't set (normal chat, CLI, JSON-RPC, unit tests),
//! [`current_task_recency_window`] returns `None` and the tool surface keeps
//! its default unbounded behavior. This is strictly additive: a user asking
//! "show all my Linear issues" in chat is never silently 24h-filtered.

use std::time::Duration;

tokio::task_local! {
    /// Recency window installed for the currently-executing agent turn.
    /// Scoped per turn by the cron agent runner; any tool executed inside
    /// that turn can read it. Unset (→ `None`) everywhere else.
    pub static TASK_RECENCY_WINDOW: Duration;
}

/// Returns the active task-recency window, if the scope is installed.
///
/// `None` outside [`with_task_recency_window`] — i.e. normal chat turns,
/// direct CLI/JSON-RPC tool dispatch, or unit tests calling a [`tinytools::Tool`]
/// directly. Callers treat `None` as "no recency restriction".
pub fn current_task_recency_window() -> Option<Duration> {
    let window = TASK_RECENCY_WINDOW.try_with(|w| *w).ok();
    tracing::trace!(
        has_window = window.is_some(),
        window_secs = window.map(|w| w.as_secs()),
        "[composio][task-window] read current window"
    );
    window
}

/// Run `future` with `window` installed as the current task-recency window.
///
/// Intended call site is the cron agent runner, wrapped around the turn for
/// agents (today: `morning_briefing`) that should only see recently-touched
/// task data. The scope does not leak into detached tasks spawned inside
/// `future` — standard [`tokio::task_local!`] semantics.
pub async fn with_task_recency_window<F, R>(window: Duration, future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    tracing::trace!(
        window_secs = window.as_secs(),
        "[composio][task-window] scope enter"
    );
    let out = TASK_RECENCY_WINDOW.scope(window, future).await;
    tracing::trace!("[composio][task-window] scope exit");
    out
}

#[cfg(test)]
#[path = "task_recency_context_tests.rs"]
mod tests;
