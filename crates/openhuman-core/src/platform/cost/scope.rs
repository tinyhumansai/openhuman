//! Building a [`UsageScope`] from what the recording task knows.
//!
//! Usage is recorded deep in the agent loop, where no caller passes thread or
//! agent ids down. They are already in scope as task-locals, though: the turn
//! origin (thread, channel or job, bound on the turn's `CoreContext`) and the
//! [`CoreContext`] (the embedded or SaaS user agent). The recording site adds
//! what only it knows — the provider, the sub-agent when a delegated child made
//! the call, and the agent definition acting when it holds the run
//! ([`UsageScope::with_definition`]); the memory identity is not ambient.
//!
//! [`CoreContext`]: crate::core::runtime::CoreContext

use crate::agent::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};
use crate::core::runtime::DispatchCtx;

use super::types::UsageScope;

impl UsageScope {
    /// The scope of a call running under `ctx`, with `provider` and the
    /// sub-agent (its definition and task id) supplied by the recording site.
    /// The user agent comes from `ctx`'s tenant, not from task state.
    pub fn in_ctx(
        ctx: &DispatchCtx,
        provider: Option<&str>,
        subagent: Option<(&str, &str)>,
    ) -> Self {
        let origin = ctx.turn_origin_or_scoped();
        let (thread_id, origin) = describe_origin(origin.as_ref());
        let session_agent = ctx.tenant().ok().and_then(|tenant| tenant.agent);
        let (agent_id, subagent_task_id) = match subagent {
            Some((agent, task)) => (Some(agent.to_string()), Some(task.to_string())),
            None => (None, None),
        };
        Self {
            thread_id,
            origin,
            agent_id,
            subagent_task_id,
            session_agent,
            provider: provider.filter(|p| !p.is_empty()).map(str::to_owned),
        }
    }
}

impl UsageScope {
    /// This scope attributed to the agent definition `definition` acting,
    /// unless a sub-agent already names the agent.
    #[must_use]
    pub fn with_definition(mut self, definition: Option<&str>) -> Self {
        if self.agent_id.is_none() {
            self.agent_id = definition
                .filter(|definition| !definition.is_empty())
                .map(str::to_owned);
        }
        self
    }
}

/// The thread a turn origin names and its short label.
pub(crate) fn describe_origin(
    origin: Option<&AgentTurnOrigin>,
) -> (Option<String>, Option<String>) {
    match origin {
        None | Some(AgentTurnOrigin::Unknown) => (None, None),
        Some(AgentTurnOrigin::WebChat { thread_id, .. }) => {
            (Some(thread_id.clone()), Some("web_chat".to_string()))
        }
        // A channel conversation's history key is its stable identity.
        Some(AgentTurnOrigin::ExternalChannel {
            channel,
            history_key,
            ..
        }) => (
            history_key.clone().filter(|key| !key.trim().is_empty()),
            Some(format!("channel:{channel}")),
        ),
        Some(AgentTurnOrigin::TrustedAutomation { source, .. }) => {
            let label = match source {
                TrustedAutomationSource::Cron => "cron",
                TrustedAutomationSource::Background => "background",
                TrustedAutomationSource::Workflow { .. } => "workflow",
            };
            (None, Some(label.to_string()))
        }
        Some(AgentTurnOrigin::Cli) => (None, Some("cli".to_string())),
        Some(AgentTurnOrigin::DirectChat) => (None, Some("direct_chat".to_string())),
    }
}

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;
