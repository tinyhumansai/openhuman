//! The agent memory lifecycle: what the host calls around every agent turn.
//!
//! TinyMemory owns the lifecycle (`tinymemory_tools::AgentMemory`); this module
//! binds it to OpenHuman — the configured engine, the acting identity
//! ([`super::scope`]) and the `[memory.recall]` policy — and keeps it off the
//! turn's critical path:
//!
//! | Hook | When | Call |
//! | --- | --- | --- |
//! | [`prefetch`] | before the model runs, once per turn | takes completed cached memory and queues user logging and refresh |
//! | [`hooks::post_turn`] | after the turn is committed | logs the reply and queues the belief builds it hands back |
//! | [`prefetch`] | when the transcript is compacted | enriches from completed memory and queues recall of the folded span |
//! | [`jobs`] | the `memory_background` cron job | runs queued belief builds and deferred ingests |
//!
//! Automatic hooks never await engine I/O. The explicit/manual hook primitives
//! remain timeout-bounded; an engine outage degrades memory without blocking
//! the live agent.

pub mod date_hint;
pub mod hooks;
pub mod jobs;
pub(crate) mod prefetch;
pub mod sender;
pub mod views;
pub mod window;

use std::sync::Arc;

use tinymemory_api::MemoryEngine;
use tinymemory_tools::{AgentMemory, RecallPolicy};

use crate::config::schema::MemoryRecallConfig;
use crate::config::Config;

use super::engine;
use super::error::{MemoryError, MemoryResult};
use super::scope::ResolvedIdentity;

/// TinyMemory's recall policy from `[memory.recall]`.
#[must_use]
pub fn policy(recall: &MemoryRecallConfig) -> RecallPolicy {
    let limit = |value: u32| usize::try_from(value).unwrap_or(usize::MAX);
    RecallPolicy {
        budget_tokens: limit(recall.budget_tokens).max(1),
        learnings_limit: limit(recall.learnings_limit),
        brain_limit: limit(recall.brain_limit),
        history_limit: limit(recall.history_limit),
        team_limit: limit(recall.team_limit),
        build_beliefs_every: (recall.build_beliefs_every > 0).then_some(recall.build_beliefs_every),
    }
}

/// A policy that recalls nothing: the turn is logged, no section is read.
#[must_use]
pub fn log_only(policy: RecallPolicy) -> RecallPolicy {
    RecallPolicy {
        learnings_limit: 0,
        brain_limit: 0,
        history_limit: 0,
        team_limit: 0,
        ..policy
    }
}

/// `identity`'s memory on `engine` under `config`'s policy.
pub fn agent_memory_on(
    engine: Arc<dyn MemoryEngine>,
    config: &Config,
    identity: &ResolvedIdentity,
) -> MemoryResult<AgentMemory> {
    Ok(
        AgentMemory::new(engine, identity.layout.clone(), &identity.agent_id)?
            .with_policy(policy(&config.memory.recall)),
    )
}

/// `identity`'s memory on the configured engine; [`MemoryError::Off`] when no
/// engine is bound.
pub fn agent_memory(config: &Config, identity: &ResolvedIdentity) -> MemoryResult<AgentMemory> {
    let bound = engine::resolve(config).engine()?;
    agent_memory_on(bound.engine, config, identity)
}

/// The memory of `identity` (the default agent for the root identity).
pub fn current_agent_memory(
    config: &Config,
    identity: &super::scope::MemoryIdentity,
) -> MemoryResult<(AgentMemory, ResolvedIdentity)> {
    let identity = identity.resolve(config);
    let memory = agent_memory(config, &identity)?;
    Ok((memory, identity))
}

/// An agent id the caller named, resolved under `identity`'s root; `identity`
/// itself when none is named.
pub fn named_agent_memory(
    config: &Config,
    identity: &super::scope::MemoryIdentity,
    agent_id: Option<&str>,
) -> MemoryResult<(AgentMemory, ResolvedIdentity)> {
    let mut identity = identity.resolve(config);
    if let Some(agent_id) = agent_id.map(str::trim).filter(|id| !id.is_empty()) {
        identity.agent_id = agent_id.to_string();
    }
    let memory = agent_memory(config, &identity).map_err(|error| match error {
        MemoryError::InvalidRequest(message) => {
            MemoryError::invalid(format!("agent `{}`: {message}", identity.agent_id))
        }
        other => other,
    })?;
    Ok((memory, identity))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
