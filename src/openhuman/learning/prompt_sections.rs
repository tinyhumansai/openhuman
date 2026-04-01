//! Prompt sections that inject learned context into the agent's system prompt.
//!
//! These sections read from memory at prompt-build time and format observations,
//! patterns, and user profile data for the LLM context.

use crate::openhuman::agent::prompt::{PromptContext, PromptSection};
use crate::openhuman::memory::{Memory, MemoryCategory};
use anyhow::Result;
use std::sync::Arc;

/// Injects recent observations and patterns from the learning subsystem.
pub struct LearnedContextSection {
    memory: Arc<dyn Memory>,
}

impl LearnedContextSection {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    /// Load learned context without blocking the runtime.
    /// Note: This is called during prompt building (sync context).
    /// To avoid blocking, we return empty and rely on pre-cached data in future iterations.
    fn load_learned_context(&self) -> String {
        // TODO: Pre-fetch and cache this data during Agent initialization or turn start.
        // For now, we return empty to avoid blocking the Tokio runtime.
        tracing::debug!(
            "[learning] LearnedContextSection skipped during prompt build (sync context). \
             Consider pre-fetching in Agent::turn() or init."
        );
        String::new()
    }
}

impl PromptSection for LearnedContextSection {
    fn name(&self) -> &str {
        "learned_context"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok(self.load_learned_context())
    }
}

/// Injects the learned user profile into the system prompt.
pub struct UserProfileSection {
    memory: Arc<dyn Memory>,
}

impl UserProfileSection {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    /// Load user profile without blocking the runtime.
    /// Note: This is called during prompt building (sync context).
    /// To avoid blocking, we return empty and rely on pre-cached data in future iterations.
    fn load_user_profile(&self) -> String {
        // TODO: Pre-fetch and cache this data during Agent initialization or turn start.
        // For now, we return empty to avoid blocking the Tokio runtime.
        tracing::debug!(
            "[learning] UserProfileSection skipped during prompt build (sync context). \
             Consider pre-fetching in Agent::turn() or init."
        );
        String::new()
    }
}

impl PromptSection for UserProfileSection {
    fn name(&self) -> &str {
        "user_profile"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok(self.load_user_profile())
    }
}