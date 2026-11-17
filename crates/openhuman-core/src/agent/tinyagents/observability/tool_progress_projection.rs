//! Projection of `AgentEvent::ToolProgressDetail` onto
//! [`AgentProgress::ToolCallProgress`].

use crate::progress::AgentProgress;

use super::event_bridge::OpenhumanEventBridge;

impl OpenhumanEventBridge {
    /// Forward a running tool's progress report for a parent-scope call.
    ///
    /// A sub-agent's tool progress stays inside its own run: on the parent
    /// turn, the delegation call that runs the sub-agent is the open call. An
    /// update that carries no message (a bare fraction or partial output) has
    /// nothing to show and is not forwarded.
    pub(super) fn forward_tool_progress(&self, call_id: &str, message: &str) {
        if self.scope.is_some() || message.trim().is_empty() {
            return;
        }
        self.send(AgentProgress::ToolCallProgress {
            call_id: call_id.to_string(),
            message: message.to_string(),
        });
    }
}
