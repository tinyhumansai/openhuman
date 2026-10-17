//! Owned hooks carried by one session or turn, never installed process-wide
//! and never recovered from an ambient scope.

use std::sync::Arc;

use super::{PostTurnHook, ToolHook};

/// Agent/turn callbacks added after the runtime's global callbacks.
///
/// A scope is plain data: the entry point that owns the turn hands it to the
/// session it builds ([`OpenHumanSessionHost::set_host_env`]), which copies the
/// tool and stop callbacks onto the turn's explicit
/// [`OpenHumanRunContext`](crate::agent::tinyagents::host::OpenHumanRunContext)
/// and keeps the post-turn callbacks for the learning sink. A child run
/// inherits them through `OpenHumanRunContext::child`.
///
/// [`OpenHumanSessionHost::set_host_env`]: crate::agent::OpenHumanSessionHost::set_host_env
#[derive(Clone, Default)]
pub struct HookScope {
    tools: Vec<Arc<dyn ToolHook>>,
    post_turn: Vec<Arc<dyn PostTurnHook>>,
    stop:
        Vec<Arc<dyn tinyagents_harness::middleware::StopHook<tinyagents_harness::cost::TurnCost>>>,
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
        hook: Arc<dyn tinyagents_harness::middleware::StopHook<tinyagents_harness::cost::TurnCost>>,
    ) {
        self.stop.push(hook);
    }

    /// The scope's own tool callbacks, in registration order.
    pub fn tool_hooks(&self) -> &[Arc<dyn ToolHook>] {
        &self.tools
    }

    /// The scope's own completed-turn callbacks, in registration order.
    pub fn post_turn_hooks(&self) -> &[Arc<dyn PostTurnHook>] {
        &self.post_turn
    }

    /// The scope's own stop policies, in registration order.
    pub fn stop_hooks(
        &self,
    ) -> &[Arc<dyn tinyagents_harness::middleware::StopHook<tinyagents_harness::cost::TurnCost>>]
    {
        &self.stop
    }

    /// Whether the scope carries no callback of any kind.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty() && self.post_turn.is_empty() && self.stop.is_empty()
    }
}
