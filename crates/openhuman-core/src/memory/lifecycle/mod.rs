//! The agent memory lifecycle: what the host calls around every agent turn.
//!
//! TinyMemory owns the lifecycle (`tinymemory_tools::AgentMemory`); this module
//! binds it to OpenHuman — the configured engine, the acting identity
//! ([`super::scope`]) and the `[memory.recall]` policy — and keeps it off the
//! turn's critical path:
//!
//! | Hook | When | Call |
//! | --- | --- | --- |
//! | [`prefetch`] | before the model runs, once per turn | takes completed cached memory and queues user logging and refresh; with `[memory] recall_each_turn_ms`, waits that long for the current message's pack instead |
//! | [`hooks::post_turn`] | after the turn is committed | logs the reply and queues the belief builds it hands back |
//! | [`prefetch`] | when the transcript is compacted | enriches from completed memory and queues recall of the folded span |
//! | [`jobs`] | the `memory_background` cron job | runs queued belief builds and deferred ingests |
//!
//! Automatic hooks never await engine I/O beyond a deadline the host chose: by
//! default not at all, with `recall_each_turn_ms` at most that long. An engine
//! outage degrades memory without blocking the live agent.

pub mod date_hint;
pub mod hooks;
pub mod jobs;
pub(crate) mod memory_block;
pub mod plan_notes;
pub(crate) mod prefetch;
pub mod sender;
pub mod views;

use std::sync::Arc;

use tinymemory_api::MemoryEngine;
use tinymemory_tools::{AgentMemory, RecallPolicy};

use crate::config::schema::RecallSettings;
use crate::config::Config;

use super::engine;
use tinymemory_api::{Error, Result};
use tinymemory_tools::layout::identity::ResolvedIdentity;

/// TinyMemory's recall policy from `[memory.recall]`.
#[must_use]
pub fn policy(recall: &RecallSettings) -> RecallPolicy {
    recall.into()
}

/// `identity`'s memory on `engine` under `config`'s policy.
pub fn agent_memory_on(
    engine: Arc<dyn MemoryEngine>,
    config: &Config,
    identity: &ResolvedIdentity,
) -> Result<AgentMemory> {
    Ok(
        AgentMemory::new(engine, identity.layout.clone(), &identity.agent_id)?
            .with_policy(policy(&config.memory.recall)),
    )
}

/// `identity`'s memory on the configured engine; [`Error::Off`] when no
/// engine is bound.
pub async fn agent_memory(config: &Config, identity: &ResolvedIdentity) -> Result<AgentMemory> {
    let bound = engine::resolve(config).await.engine()?;
    agent_memory_on(bound.engine, config, identity)
}

/// The memory of `identity` (the default agent for the root identity).
pub async fn current_agent_memory(
    config: &Config,
    identity: &tinymemory_tools::layout::identity::Identity,
) -> Result<(AgentMemory, ResolvedIdentity)> {
    let identity = identity.resolve(&crate::memory::scope::identity_pins(config));
    let memory = agent_memory(config, &identity).await?;
    Ok((memory, identity))
}

/// An agent id the caller named, resolved under `identity`'s root; `identity`
/// itself when none is named.
pub async fn named_agent_memory(
    config: &Config,
    identity: &tinymemory_tools::layout::identity::Identity,
    agent_id: Option<&str>,
) -> Result<(AgentMemory, ResolvedIdentity)> {
    let mut identity = identity.resolve(&crate::memory::scope::identity_pins(config));
    if let Some(agent_id) = agent_id.map(str::trim).filter(|id| !id.is_empty()) {
        identity.agent_id = agent_id.to_string();
    }
    let memory = agent_memory(config, &identity)
        .await
        .map_err(|error| match error {
            Error::InvalidRequest(message) => {
                Error::InvalidRequest(format!("agent `{}`: {message}", identity.agent_id))
            }
            other => other,
        })?;
    Ok((memory, identity))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
