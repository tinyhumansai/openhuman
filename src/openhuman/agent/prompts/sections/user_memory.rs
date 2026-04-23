use crate::openhuman::agent::prompts::types::{PromptContext, PromptSection};
use anyhow::Result;
use std::fmt::Write;

pub struct UserMemorySection;

impl PromptSection for UserMemorySection {
    fn name(&self) -> &str {
        "user_memory"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        if ctx.learned.tree_root_summaries.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("## User Memory\n\n");
        out.push_str(
            "Long-term memory distilled by the tree summarizer. \
             Each section is the root summary for a memory namespace, \
             representing everything we've learned about that domain over time. \
             Treat this as durable context: the model has seen these facts before, \
             they should not need to be re-discovered.\n\n",
        );

        for (namespace, body) in &ctx.learned.tree_root_summaries {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                continue;
            }
            let _ = writeln!(out, "### {namespace}\n");
            out.push_str(trimmed);
            out.push_str("\n\n");
        }

        Ok(out)
    }
}

/// Render the tree-summariser user-memory block.
pub fn render_user_memory(ctx: &PromptContext<'_>) -> Result<String> {
    UserMemorySection.build(ctx)
}
