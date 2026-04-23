use crate::openhuman::agent::prompts::types::{PromptContext, PromptSection};
use crate::openhuman::agent::prompts::workspace_files::{inject_workspace_file, sync_workspace_file};
use anyhow::Result;

pub struct IdentitySection;

impl PromptSection for IdentitySection {
    fn name(&self) -> &str {
        "identity"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        let mut prompt = String::from("## Project Context\n\n");
        prompt.push_str(
            "The following workspace files define your identity, behavior, and context.\n\n",
        );
        // When the visible-tool filter is active the main agent is a pure
        // orchestrator: it routes via spawn_subagent, synthesises results,
        // and talks to the user. It does NOT need the periodic-task config
        // (HEARTBEAT.md) — subagents handle their own concerns.
        let is_orchestrator = !ctx.visible_tool_names.is_empty();
        let all_files: &[&str] = &["SOUL.md", "IDENTITY.md", "HEARTBEAT.md"];
        // Orchestrator skips these from the prompt but we still sync them
        // to disk so they stay current.
        let skip_in_prompt: &[&str] = if is_orchestrator {
            &["HEARTBEAT.md"]
        } else {
            &[]
        };
        for file in all_files {
            // Always sync to disk so builtin updates ship.
            sync_workspace_file(ctx.workspace_dir, file);
            if !skip_in_prompt.contains(file) {
                inject_workspace_file(&mut prompt, ctx.workspace_dir, file);
            }
        }

        // PROFILE.md / MEMORY.md injection lives in the dedicated
        // `UserFilesSection` (below) so agents that strip the identity
        // preamble (`omit_identity = true`) — welcome, orchestrator, the
        // trigger pair — still get their user files at runtime via
        // `SystemPromptBuilder::for_subagent`, which omits
        // `IdentitySection` entirely when `omit_identity` is set.

        Ok(prompt)
    }
}

/// Render the `## Project Context` identity block
/// (`SOUL.md` / `IDENTITY.md` / optionally `HEARTBEAT.md`).
pub fn render_identity(ctx: &PromptContext<'_>) -> Result<String> {
    IdentitySection.build(ctx)
}
