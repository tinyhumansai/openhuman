//! Task-local carrier for the **calling agent's `sandbox_mode`** so tool
//! implementations can enforce sandbox semantics at execution time without
//! widening the [`crate::openhuman::tools::Tool`] trait signature.
//!
//! Sibling of the existing [`super::fork_context`] task-locals but serves
//! a different concept: `PARENT_CONTEXT` / `FORK_CONTEXT` carry the
//! *parent agent's* runtime context so that `spawn_subagent` can inherit
//! it, whereas [`CURRENT_AGENT_SANDBOX_MODE`] carries the *currently-
//! executing agent's* sandbox mode so that any tool it invokes can gate
//! on that mode.
//!
//! Why a task-local instead of an argument on [`Tool::execute`]: the tool
//! trait is called from many places (CLI, JSON-RPC, tests, agent loops).
//! Threading an optional context argument through every call site would
//! touch every tool implementation and every caller. A task-local keeps
//! the additive path scoped to the agent runtime that actually needs it.
//!
//! Tools read the current mode via [`current_sandbox_mode`]. When the
//! task-local isn't set (direct CLI / JSON-RPC / unit-test invocation),
//! the function returns `None` and tools fall through to their default
//! pre-sandbox behavior, so this change is strictly additive.

use super::definition::SandboxMode;

tokio::task_local! {
    /// Sandbox mode declared in the currently-executing agent's
    /// `agent.toml`. Scoped per agent turn by the tool loop so any tool
    /// executed inside that turn can read it. `None` when unset (direct
    /// tool invocation outside an agent turn).
    pub static CURRENT_AGENT_SANDBOX_MODE: SandboxMode;
}

/// Returns the current agent's `sandbox_mode`, if the scope is active.
///
/// Returns `None` when called from outside
/// [`with_current_sandbox_mode`] — e.g. CLI tool invocation, JSON-RPC
/// tool dispatch, or unit tests that call a [`Tool`] directly.
pub fn current_sandbox_mode() -> Option<SandboxMode> {
    CURRENT_AGENT_SANDBOX_MODE.try_with(|mode| *mode).ok()
}

/// Run `future` with `mode` installed as the current sandbox mode.
///
/// Intended call site is the tool loop (and subagent runner) immediately
/// around each `tool.execute(args)` invocation so every tool the agent
/// calls observes the correct mode. The scope does not leak into any
/// detached task spawned inside `future` — that is standard
/// [`tokio::task_local!`] semantics.
pub async fn with_current_sandbox_mode<F, R>(mode: SandboxMode, future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    CURRENT_AGENT_SANDBOX_MODE.scope(mode, future).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn current_sandbox_mode_returns_none_outside_scope() {
        assert_eq!(current_sandbox_mode(), None);
    }

    #[tokio::test]
    async fn with_current_sandbox_mode_installs_read_only() {
        let observed =
            with_current_sandbox_mode(SandboxMode::ReadOnly, async { current_sandbox_mode() })
                .await;
        assert_eq!(observed, Some(SandboxMode::ReadOnly));
    }

    #[tokio::test]
    async fn with_current_sandbox_mode_does_not_leak_across_scopes() {
        with_current_sandbox_mode(SandboxMode::ReadOnly, async {
            assert_eq!(current_sandbox_mode(), Some(SandboxMode::ReadOnly));
        })
        .await;
        assert_eq!(current_sandbox_mode(), None);
    }

    #[tokio::test]
    async fn nested_scope_overrides_outer() {
        with_current_sandbox_mode(SandboxMode::ReadOnly, async {
            assert_eq!(current_sandbox_mode(), Some(SandboxMode::ReadOnly));
            with_current_sandbox_mode(SandboxMode::Sandboxed, async {
                assert_eq!(current_sandbox_mode(), Some(SandboxMode::Sandboxed));
            })
            .await;
            assert_eq!(current_sandbox_mode(), Some(SandboxMode::ReadOnly));
        })
        .await;
    }
}
