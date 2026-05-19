//! Task-local spawn-depth tracking for nested sub-agent delegation.
//!
//! Loader-time tier validation prevents built-in and workspace agent
//! definitions from declaring recursive hierarchies, but runtime calls can
//! still arrive through tools and MCP surfaces. This task-local is the
//! defence-in-depth layer that caps the active `run_subagent` chain.

/// Maximum number of nested `run_subagent` scopes allowed in one task.
///
/// Depth counts sub-agent runs, not the root user-facing agent turn, so the
/// intended deepest path is `chat -> reasoning -> worker`:
///
/// * reasoning sub-agent: depth 1
/// * worker spawned by reasoning: depth 2
/// * one final worker handoff: depth 3
pub const MAX_SPAWN_DEPTH: usize = 3;

tokio::task_local! {
    /// Current active `run_subagent` nesting depth for this task.
    static CURRENT_SPAWN_DEPTH: usize;
}

/// Return the active sub-agent nesting depth for this task.
///
/// Direct callers outside [`with_spawn_depth`] are at depth 0.
pub fn current_spawn_depth() -> usize {
    CURRENT_SPAWN_DEPTH.try_with(|depth| *depth).unwrap_or(0)
}

/// Run `future` with a specific active sub-agent depth.
pub async fn with_spawn_depth<F, R>(depth: usize, future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    CURRENT_SPAWN_DEPTH.scope(depth, future).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn current_spawn_depth_defaults_to_zero() {
        assert_eq!(current_spawn_depth(), 0);
    }

    #[tokio::test]
    async fn with_spawn_depth_scopes_value_to_future() {
        let observed = with_spawn_depth(2, async { current_spawn_depth() }).await;
        assert_eq!(observed, 2);
        assert_eq!(current_spawn_depth(), 0);
    }

    #[tokio::test]
    async fn nested_spawn_depth_scope_restores_outer_value() {
        with_spawn_depth(1, async {
            assert_eq!(current_spawn_depth(), 1);
            with_spawn_depth(2, async {
                assert_eq!(current_spawn_depth(), 2);
            })
            .await;
            assert_eq!(current_spawn_depth(), 1);
        })
        .await;
    }
}
