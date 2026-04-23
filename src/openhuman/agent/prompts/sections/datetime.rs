use crate::openhuman::agent::prompts::types::{PromptContext, PromptSection};
use anyhow::Result;
use chrono::Local;

pub struct DateTimeSection;

impl PromptSection for DateTimeSection {
    fn name(&self) -> &str {
        "datetime"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        let now = Local::now();
        Ok(format!(
            "## Current Date & Time\n\n{} ({})",
            now.format("%Y-%m-%d %H:%M:%S"),
            now.format("%Z")
        ))
    }
}

/// Render the `## Current Date & Time` block. Intentionally **not**
/// included in byte-stable sub-agent prompts (`for_subagent`) because
/// injecting `Local::now()` defeats prefix caching. Exposed so full-
/// assembly main-agent builders can opt in.
pub fn render_datetime(ctx: &PromptContext<'_>) -> Result<String> {
    DateTimeSection.build(ctx)
}
