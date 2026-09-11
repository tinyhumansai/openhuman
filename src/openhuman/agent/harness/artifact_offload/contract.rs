//! The prompt-side half of the artifact-offload convention (#3883).
//!
//! Single source of truth for the directory names the model is told about, so
//! the prompt and [`super::paths::resolve_artifact_path`] can never drift.

use std::collections::HashSet;

use tinyagents_harness::artifacts::{OUTPUTS_DIR, SCRATCH_DIR};

/// Approximate token budget quoted to the model as the offload trigger.
/// Mirrors [`super::types::DEFAULT_OFFLOAD_THRESHOLD_BYTES`] at the harness-wide
/// 4-chars-per-token estimate.
const CONTRACT_THRESHOLD_TOKENS: usize = 2_000;

/// Heading the contract is rendered under. Used by the idempotence check in
/// `subagent_runner::ops::prompt` so a re-rendered prompt never stacks two
/// copies.
pub const ARTIFACT_OFFLOAD_HEADING: &str = "## Long-horizon Artifact Offload";

/// Tool a sub-agent must actually hold before the contract is worth rendering.
///
/// A prompt may only name tools the agent can really call: advertising one it
/// lacks produces hallucinated calls that fail. `researcher` (search + fetch
/// only) and every skill-filtered specialist have no filesystem tools, and
/// dedicated guards assert their prompts never mention one — see
/// `agent_registry::agents::researcher::prompt::tests::build_returns_nonempty_body`
/// and `subagent_runner::ops_tests::typed_mode_filters_tools_by_skill_filter`.
pub const OFFLOAD_WRITE_TOOL: &str = "file_write";

/// Whether the offload contract should be rendered for an agent whose visible
/// tools are `visible_tool_names`.
///
/// Only agents that can actually write a file are told to. Everyone else stays
/// covered by the harness-side automatic offload in
/// [`super::offload_oversized_result`], which needs no cooperation from the
/// model — which is exactly why that half exists.
pub fn should_render_offload_contract(visible_tool_names: &HashSet<String>) -> bool {
    visible_tool_names.contains(OFFLOAD_WRITE_TOOL)
}

/// Render the offload contract for a sub-agent that holds a file-write tool.
///
/// Deliberately parameter-free and byte-stable: the sub-agent system prompt is
/// prefix-cached, so this must render identically on every run. Absolute paths
/// are never interpolated for the same reason (and because a worktree-isolated
/// worker resolves its own action root).
///
/// Names no tool other than [`OFFLOAD_WRITE_TOOL`]. In particular it does not
/// name the parent's *reading* tool: how the parent opens the file is not this
/// agent's business, and mentioning it would leak a tool name into prompts of
/// agents that do not hold it.
pub fn render_artifact_offload_contract() -> String {
    format!(
        "{ARTIFACT_OFFLOAD_HEADING}\n\n\
Large results belong on disk, not in your reply. Two directories sit under your action directory:\n\
- `{OUTPUTS_DIR}/` for deliverables, anything the parent or a later step needs to read.\n\
- `{SCRATCH_DIR}/` for scratch, intermediate files you do not intend to hand back.\n\
\n\
When a result would run past roughly {CONTRACT_THRESHOLD_TOKENS} tokens:\n\
1. Write the full content to a file under `{OUTPUTS_DIR}/` with `{OFFLOAD_WRITE_TOOL}`.\n\
2. Reply with that relative path plus a short abstract, not the full payload.\n\
3. Quote the path exactly, so the parent can open it verbatim.\n\
\n\
Rules:\n\
- Offload paths are always relative to your action directory. Never write outside it, and never target the core's internal workspace state.\n\
- Keep the abstract honest: say what the file holds and what is still open. Never present it as the complete result.\n\
- Small results stay inline. A pointer to a two-line file costs the parent more than the two lines.\n\
- If you inline an oversized result anyway, the harness persists it under `{OUTPUTS_DIR}/` and hands the parent the path instead.\n"
    )
}
