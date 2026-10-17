//! Channel turns answered by a host-registered agent.
//!
//! `config.agent.channel_agents` binds a channel to an agent id
//! (`telegram = "teeny-chat"`). A message on a bound channel is resolved
//! through the host agent resolver ([`crate::agent::host_agents`]) and run as
//! that agent -- its definition, its host tools, its context -- instead of
//! going over the bus to the orchestrator. Everything else about the channel
//! stays: per-chat history, typing, drafts, the reply back to the chat.
//!
//! A binding the resolver cannot satisfy (the embedder has not created the
//! agent yet, or dropped it) is **refused**, with a short reply to the chat
//! and an error log. It is never widened to the orchestrator: the binding
//! exists because the channel is meant to reach one particular agent, and
//! the orchestrator holds the operator's full tool belt.
//!
//! The turn runs under the channel's `ExternalChannel` origin, passed down as
//! a parameter, and [`posture`] caps what the agent may do on it.

use std::sync::Arc;

use crate::agent::host_agents::HostAgent;
use crate::channels::context::ChannelRuntimeContext;
use crate::channels::{ChannelSendExt, SendMessage};
use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use tinychannels_bus as traits;

pub(crate) mod posture;
mod turn;

pub(crate) use turn::{run_host_agent_turn, seed_rows, HostChannelTurn};

/// What a channel's binding resolves to for one message.
pub(crate) enum HostRoute {
    /// No binding: the orchestrator answers, as before.
    Unbound,
    /// The bound agent, resolved.
    Agent(Box<HostAgent>),
    /// Bound to an agent id nobody answers for.
    Missing(String),
}

/// The agent `channel` is bound to in `config.agent.channel_agents`, if any.
pub(crate) fn bound_agent_id<'a>(config: &'a Config, channel: &str) -> Option<&'a str> {
    config
        .agent
        .channel_agents
        .get(channel)
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
}

/// Resolve `channel`'s binding against the installed host agents.
pub(crate) fn route(ctx: &ChannelRuntimeContext, channel: &str) -> HostRoute {
    let dispatch = ctx.dispatch_ctx();
    if let Some(capabilities) =
        crate::profiles::capabilities::ProfileCapabilities::in_ctx(&dispatch)
    {
        if let (Some(context), Some(config), Some(definitions)) = (
            dispatch.core().cloned(),
            ctx.config.as_ref(),
            crate::agent::harness::host_definition::AgentDefinitionRegistry::current(),
        ) {
            if let Some(definition) = definitions.get("orchestrator") {
                let mut definition = definition.clone();
                capabilities.apply_system_prompt(&mut definition);
                return HostRoute::Agent(Box::new(HostAgent {
                    definition,
                    config: config.as_ref().clone(),
                    host_tools: Some(capabilities.factory(config)),
                    context,
                    hooks: Default::default(),
                }));
            }
        }
        return HostRoute::Missing("profile agent".into());
    }
    let Some(agent_id) = ctx
        .config
        .as_deref()
        .and_then(|config| bound_agent_id(config, channel))
    else {
        return HostRoute::Unbound;
    };
    match crate::agent::host_agents::resolve(agent_id) {
        Some(agent) => {
            tracing::debug!(
                channel,
                agent_id,
                "[channels::dispatch::host_agent] channel bound to host agent"
            );
            HostRoute::Agent(Box::new(agent))
        }
        None => {
            tracing::error!(
                channel,
                agent_id,
                "[channels::dispatch::host_agent] channel is bound to an agent no host has \
                 registered; refusing the message instead of routing it to the orchestrator"
            );
            HostRoute::Missing(agent_id.to_string())
        }
    }
}

/// The reply a bound channel gets when its agent is missing.
pub(crate) const AGENT_UNAVAILABLE_REPLY: &str =
    "⚠️ This chat's assistant is not available right now. Please try again later.";

/// Answer `msg` with [`AGENT_UNAVAILABLE_REPLY`] and record the refusal.
pub(crate) async fn refuse_missing_agent(
    ctx: &ChannelRuntimeContext,
    msg: &traits::ChannelMessage,
    target_channel: Option<&Arc<dyn crate::channels::Channel>>,
    agent_id: &str,
) {
    if let Some(channel) = target_channel {
        let reply = SendMessage::new(AGENT_UNAVAILABLE_REPLY, &msg.reply_target)
            .in_thread(msg.thread_ts.clone());
        if let Err(error) = channel.send_with_outbound_intent(&reply).await {
            tracing::warn!(
                channel = %msg.channel,
                %error,
                "[channels::dispatch::host_agent] failed to send the unavailable reply"
            );
        }
    }
    BUS.publish(DomainEvent::ChannelMessageProcessed {
        channel: msg.channel.clone(),
        message_id: msg.id.clone(),
        sender: msg.sender.clone(),
        reply_target: msg.reply_target.clone(),
        content: msg.content.clone(),
        thread_ts: msg.thread_ts.clone(),
        response: AGENT_UNAVAILABLE_REPLY.to_string(),
        provider: format!("agent:{agent_id}"),
        model: String::new(),
        elapsed_ms: 0,
        success: false,
        workspace_dir: ctx.workspace_dir.as_ref().clone(),
    });
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod dispatch_tests;
