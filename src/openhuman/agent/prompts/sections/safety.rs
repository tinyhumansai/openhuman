use crate::openhuman::agent::prompts::types::{PromptContext, PromptSection};
use anyhow::Result;

pub struct SafetySection;

impl PromptSection for SafetySection {
    fn name(&self) -> &str {
        "safety"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok("## Safety\n\n- Do not exfiltrate private data.\n- Do not run destructive commands without asking.\n- Do not bypass oversight or approval mechanisms.\n- Prefer `trash` over `rm`.\n- When in doubt, ask before acting externally.".into())
    }
}

/// Render the static `## Safety` block.
pub fn render_safety() -> String {
    SafetySection
        .build(&super::empty_prompt_context_for_static_sections())
        .expect("SafetySection::build is infallible")
}
