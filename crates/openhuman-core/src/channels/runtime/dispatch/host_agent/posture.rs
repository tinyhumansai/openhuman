//! What a host agent may do on a turn an outside sender started.
//!
//! A channel turn is [`AgentTurnOrigin::ExternalChannel`]: whoever can message
//! the bot wrote its text. On the orchestrator path an acting tool parks an
//! approval and, with no one to answer it, is denied when the park expires. A
//! host agent bound to a public channel must not wait on that: the sender is
//! the only person in the chat, and they are the one who must not approve it.
//! So the turn is capped at [`PermissionLevel::ReadOnly`] and anything above
//! that, or anything with an external effect, is refused on the spot with a
//! tool error the model can relay.

use tinytools::{PermissionLevel, Tool};

use crate::agent::HostTools;
use crate::config::Config;
use crate::core::runtime::turn_origin::AgentTurnOrigin;

/// The most a host agent may do on a turn from `origin`; `None` leaves the
/// agent's own limits alone.
pub(crate) fn tool_ceiling(origin: &AgentTurnOrigin) -> Option<PermissionLevel> {
    match origin {
        AgentTurnOrigin::ExternalChannel { .. } => Some(PermissionLevel::ReadOnly),
        _ => None,
    }
}

/// Cap `channel` at `ceiling` in this turn's `config`.
///
/// The session's tool-policy snapshot reads this map, so every tool on the
/// belt -- built-in or host -- above the ceiling is denied before it runs,
/// per call. Only ever narrows: an operator's `none` is kept.
pub(crate) fn cap_channel(config: &mut Config, channel: &str, ceiling: PermissionLevel) {
    let entry = config
        .agent
        .channel_permissions
        .entry(channel.to_string())
        .or_default();
    if entry.trim().eq_ignore_ascii_case("none") {
        return;
    }
    *entry = permission_token(ceiling).to_string();
}

/// The `channel_permissions` spelling of `level`.
fn permission_token(level: PermissionLevel) -> &'static str {
    match level {
        PermissionLevel::None => "none",
        PermissionLevel::ReadOnly => "readonly",
        PermissionLevel::Write => "write",
        PermissionLevel::Execute => "execute",
        PermissionLevel::Dangerous => "dangerous",
    }
}

/// `host` with every tool it builds refusing calls above `ceiling`.
pub(crate) fn guard_host_tools(host: HostTools, ceiling: PermissionLevel) -> HostTools {
    std::sync::Arc::new(move |turn| {
        let host = host.clone();
        Box::pin(async move {
            let mut belt = host(turn).await;
            belt.tools = std::mem::take(&mut belt.tools)
                .into_iter()
                .map(|tool| Box::new(CeilingGuard::new(tool, ceiling)) as Box<dyn Tool>)
                .collect();
            belt
        })
    })
}

/// A tool that refuses calls above its ceiling instead of running them.
pub(crate) struct CeilingGuard {
    inner: Box<dyn Tool>,
    ceiling: PermissionLevel,
}

impl CeilingGuard {
    pub(crate) fn new(inner: Box<dyn Tool>, ceiling: PermissionLevel) -> Self {
        Self { inner, ceiling }
    }

    /// The refusal for a call with `args`, or `None` when it may run.
    fn refusal(&self, args: &serde_json::Value) -> Option<tinytools::ToolResult> {
        let required = self.inner.permission_level_with_args(args);
        let effect = self.inner.external_effect_with_args(args);
        if required <= self.ceiling && !effect {
            return None;
        }
        let name = self.inner.name();
        tracing::info!(
            tool = name,
            %required,
            external_effect = effect,
            ceiling = %self.ceiling,
            "[channels::dispatch::host_agent] refused tool call on an external channel turn"
        );
        Some(tinytools::ToolResult::error(format!(
            "Tool '{name}' is not available in this chat: messages here come from people \
             outside this deployment, so actions that change anything (required: {required}, \
             allowed: {}) are refused. Answer with what you can do without it.",
            self.ceiling
        )))
    }
}

#[path = "guard_tool.rs"]
mod guard_tool;

#[cfg(test)]
#[path = "posture_tests.rs"]
mod tests;
