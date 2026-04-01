//! Prompt sections that inject learned context into the agent's system prompt.
//!
//! These sections read pre-fetched data from `PromptContext.learned` — no async
//! or blocking I/O happens during prompt building.

use crate::openhuman::agent::prompt::{PromptContext, PromptSection};
use anyhow::Result;

/// Injects recent observations and patterns from the learning subsystem.
pub struct LearnedContextSection;

impl LearnedContextSection {
    pub fn new(_memory: std::sync::Arc<dyn crate::openhuman::memory::Memory>) -> Self {
        // Memory parameter kept for API compatibility but data comes from PromptContext.learned
        Self
    }
}

impl PromptSection for LearnedContextSection {
    fn name(&self) -> &str {
        "learned_context"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        if ctx.learned.observations.is_empty() && ctx.learned.patterns.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("## Learned Context\n\n");

        if !ctx.learned.observations.is_empty() {
            out.push_str("### Recent Observations\n");
            for obs in &ctx.learned.observations {
                out.push_str("- ");
                out.push_str(obs);
                out.push('\n');
            }
            out.push('\n');
        }

        if !ctx.learned.patterns.is_empty() {
            out.push_str("### Recognized Patterns\n");
            for pat in &ctx.learned.patterns {
                out.push_str("- ");
                out.push_str(pat);
                out.push('\n');
            }
            out.push('\n');
        }

        Ok(out)
    }
}

/// Injects the learned user profile into the system prompt.
pub struct UserProfileSection;

impl UserProfileSection {
    pub fn new(_memory: std::sync::Arc<dyn crate::openhuman::memory::Memory>) -> Self {
        Self
    }
}

impl PromptSection for UserProfileSection {
    fn name(&self) -> &str {
        "user_profile"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        if ctx.learned.user_profile.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("## User Profile (Learned)\n\n");
        for entry in &ctx.learned.user_profile {
            out.push_str("- ");
            out.push_str(entry);
            out.push('\n');
        }
        out.push('\n');
        Ok(out)
    }
}
