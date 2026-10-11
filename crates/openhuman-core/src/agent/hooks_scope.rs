//! Owned hooks carried by one dispatch task, never installed process-wide.

use std::future::Future;
use std::sync::Arc;

use super::{PostTurnHook, ToolHook};

tokio::task_local! {
    static ACTIVE: HookScope;
}

/// Agent/turn callbacks added after the runtime's global callbacks.
///
/// Sessions snapshot these while they are built; asynchronous post-turn
/// callbacks own that snapshot. Independently spawned tasks must explicitly
/// inherit the scope if they build further sessions.
#[derive(Clone, Default)]
pub struct HookScope {
    tools: Vec<Arc<dyn ToolHook>>,
    post_turn: Vec<Arc<dyn PostTurnHook>>,
    stop: Vec<Arc<dyn tinyagents_harness::middleware::StopHook<crate::agent::cost::TurnCost>>>,
}

impl HookScope {
    /// Append a tool callback, preserving registration order.
    pub fn push_tool(&mut self, hook: Arc<dyn ToolHook>) {
        self.tools.push(hook);
    }

    /// Append a completed-turn callback, preserving registration order.
    pub fn push_post_turn(&mut self, hook: Arc<dyn PostTurnHook>) {
        self.post_turn.push(hook);
    }

    /// Append a usage observer/budget policy checked after each model call.
    pub fn push_stop(
        &mut self,
        hook: Arc<dyn tinyagents_harness::middleware::StopHook<crate::agent::cost::TurnCost>>,
    ) {
        self.stop.push(hook);
    }

    /// Build a session with scoped tool/post-turn callbacks. Stop policies
    /// enter through the asynchronous dispatch scope when the turn runs.
    pub fn sync_scope<T>(self, build: impl FnOnce() -> T) -> T {
        ACTIVE.sync_scope(self, build)
    }

    /// Run a dispatch with this scope. Nested scopes replace tool/post-turn
    /// callbacks and append their stop policies to the ambient policies.
    pub async fn scope<F: Future>(self, future: F) -> F::Output {
        let mut stop = crate::agent::stop_hooks::current_stop_hooks();
        stop.extend(self.stop.iter().cloned());
        crate::agent::stop_hooks::with_stop_hooks(stop, ACTIVE.scope(self, future)).await
    }
}

/// Session snapshot: process-wide tool callbacks, then the dispatch's callbacks.
pub fn turn_tool_hooks() -> Vec<Arc<dyn ToolHook>> {
    let mut hooks = super::embedder_tool_hooks();
    let _ = ACTIVE.try_with(|scope| hooks.extend(scope.tools.iter().cloned()));
    hooks
}

/// Session snapshot: process-wide post-turn callbacks, then dispatch callbacks.
pub fn turn_post_turn_hooks() -> Vec<Arc<dyn PostTurnHook>> {
    let mut hooks = super::embedder_post_turn_hooks();
    let _ = ACTIVE.try_with(|scope| hooks.extend(scope.post_turn.iter().cloned()));
    hooks
}
