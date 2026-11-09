//! System prompt builder for the `flow_discovery` built-in agent (the "Flow
//! Scout").
//!
//! Composes the module-owned discovery archetype with OpenHuman's scoped
//! runtime sections. The caller fetches the archetype through TinyFlows Copilot
//! before invoking this synchronous builder.

use crate::agent::prompts::PromptContext;
use anyhow::{anyhow, Result};

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    let archetype = ctx
        .flow_archetype_text
        .ok_or_else(|| anyhow!("flow_discovery requires a scoped TinyFlows GetPrompt response"))?;
    super::super::flow_prompts::build_with_archetype(ctx, archetype)
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
