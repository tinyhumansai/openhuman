//! System-prompt helpers for sub-agent typed mode.
//!
//! Includes the artifact-offload contract used before sending specs to the
//! provider.

use crate::agent::harness::artifact_offload::OFFLOAD_WRITE_TOOL;
use std::collections::HashSet;
use tinyagents_harness::artifacts::{
    render_artifact_offload_contract, should_render_offload_contract, ARTIFACT_OFFLOAD_HEADING,
};

/// Compose child role framing with the host's artifact-tool prompt policy.
pub(crate) fn compose_subagent_system_prompt(
    base_prompt: String,
    agent_id: &str,
    visible_tool_names: &HashSet<String>,
    task_prompt: &str,
    spawn_depth: usize,
) -> String {
    let role = if visible_tool_names
        .iter()
        .any(|name| crate::agent::subagent_host::tool_prep::is_subagent_spawn_tool(name))
    {
        tinyagents_orchestration::subagent::SubagentRole::Orchestrator
    } else {
        tinyagents_orchestration::subagent::SubagentRole::Leaf
    };
    let framed = format!(
        "{}\n\n{}",
        base_prompt.trim_end(),
        tinyagents_orchestration::subagent::subagent_framing(role, spawn_depth, task_prompt,)
    );
    append_artifact_offload_contract(framed, agent_id, visible_tool_names)
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;

/// Append the filesystem-offload contract (#3883), but only for a sub-agent
/// that actually holds a file-write tool.
///
/// A sub-agent's system prompt may only name tools it can really
/// call — a skill-filtered specialist sees just its own toolkit. Telling it to write a file yields
/// hallucinated calls that fail, and two guards enforce it
/// (`ops_tests::typed_mode_filters_tools_by_skill_filter`).
///
/// Agents skipped here are still covered: `offload_oversized_result` runs on
/// every sub-agent outcome and needs no cooperation from the model.
///
/// The contract text is rendered from the artifact module so the directory
/// names the model is told about can never drift from the ones
/// `resolve_artifact_path` will actually accept. The heading check makes it
/// idempotent, and lets an archetype prompt spell the convention out itself.
pub(crate) fn append_artifact_offload_contract(
    base_prompt: String,
    agent_id: &str,
    visible_tool_names: &HashSet<String>,
) -> String {
    if !should_render_offload_contract(visible_tool_names, OFFLOAD_WRITE_TOOL) {
        tracing::debug!(
            agent_id = %agent_id,
            "[artifact] sub-agent holds no file-write tool — skipping offload contract (harness-side offload still applies)"
        );
        return base_prompt;
    }
    if base_prompt.contains(ARTIFACT_OFFLOAD_HEADING) {
        return base_prompt;
    }

    let mut prompt = base_prompt;
    if !prompt.ends_with('\n') {
        prompt.push('\n');
    }
    prompt.push('\n');
    prompt.push_str(&render_artifact_offload_contract(OFFLOAD_WRITE_TOOL));

    tracing::debug!(
        agent_id = %agent_id,
        final_chars = prompt.chars().count(),
        "[artifact] appended offload contract to sub-agent system prompt"
    );

    prompt
}
