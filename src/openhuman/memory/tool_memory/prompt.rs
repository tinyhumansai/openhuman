//! Prompt section that injects tool-scoped memory rules into the system
//! prompt (W7).
//!
//! ## Why a prompt section
//!
//! Mid-session compression rewrites the rolling chat buffer but never the
//! system prompt — that prompt is frozen for the whole session by design (so the
//! inference backend's prefix cache stays warm; see
//! [`crate::openhuman::agent::prompts::SystemPromptBuilder::build`]). Anything we
//! want to be **compression-resistant** therefore has to live in the system
//! prompt — exactly where Critical and High priority [`ToolMemoryRule`]s belong.
//!
//! ## What this module owns
//!
//! All of it, and the doc above this line used to say otherwise. The rendering
//! ([`render_tool_memory_rules`]) and the section type
//! ([`ToolMemoryRulesSection`], a byte-stable at-construction snapshot) were
//! described as "the crate's, re-exported here" back when they were
//! `tinycortex::memory::tool_memory::render`; they have been defined below for
//! some time. What was always host-retained is the [`PromptSection`] impl that
//! plugs the section into the host system-prompt builder.
//!
//! The one contract dependency is the rule vocabulary itself
//! ([`ToolMemoryRule`], [`ToolMemoryPriority`]), named at
//! [`memory::api::tool_memory`](crate::openhuman::memory::api::tool_memory)
//! because these are the types the module serialises across the bus.

use anyhow::Result;

use crate::openhuman::agent::context::prompt::{PromptContext, PromptSection};

use crate::openhuman::memory::api::tool_memory::{ToolMemoryPriority, ToolMemoryRule};

pub const TOOL_MEMORY_HEADING: &str = "## Tool-scoped rules";
pub struct ToolMemoryRulesSection {
    rendered: String,
}
impl ToolMemoryRulesSection {
    pub fn new<T: serde::Serialize>(rules: Vec<T>) -> Self {
        let rules: Vec<ToolMemoryRule> = rules
            .into_iter()
            .filter_map(|rule| {
                serde_json::to_value(rule)
                    .ok()
                    .and_then(|value| serde_json::from_value(value).ok())
            })
            .collect();
        Self {
            rendered: render_tool_memory_rules(&rules),
        }
    }
    pub fn empty() -> Self {
        Self {
            rendered: String::new(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.rendered.trim().is_empty()
    }
    pub fn rendered(&self) -> &str {
        &self.rendered
    }
}
pub fn render_tool_memory_rules(rules: &[ToolMemoryRule]) -> String {
    if rules.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<_> = rules.iter().collect();
    sorted.sort_by(|a, b| {
        a.tool_name
            .cmp(&b.tool_name)
            .then_with(|| b.priority.cmp(&a.priority))
            .then_with(|| a.rule.cmp(&b.rule))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut out = format!("{TOOL_MEMORY_HEADING}\n\nThese rules are pinned by the user or by the safety pipeline. Treat every entry as a hard constraint when considering the matching tool — do not override them silently. Lower-priority guidance lives in the `tool-{{name}}` memory namespace and can be queried via `memory_recall` if needed.\n\n");
    let mut current = None;
    for rule in sorted {
        if current != Some(rule.tool_name.as_str()) {
            if current.is_some() {
                out.push('\n');
            }
            out.push_str(&format!(
                "### `{}`\n",
                prompt_line(&rule.tool_name).replace('`', "'")
            ));
            current = Some(rule.tool_name.as_str());
        }
        let priority = match rule.priority {
            ToolMemoryPriority::Critical => "**[critical]**",
            ToolMemoryPriority::High => "**[high]**",
            ToolMemoryPriority::Normal => "**[normal]**",
        };
        out.push_str(&format!("- {priority} {}\n", prompt_line(&rule.rule)));
    }
    out
}
fn prompt_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl PromptSection for ToolMemoryRulesSection {
    fn name(&self) -> &str {
        "tool_memory_rules"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        // build() must not depend on PromptContext fields — it returns the
        // at-construction snapshot verbatim so the inference prefix cache stays warm.
        Ok(self.rendered().to_string())
    }
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
