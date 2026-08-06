//! System-prompt helpers for sub-agent typed mode.
//!
//! Includes the role-contract suffix, its injector, and the tool-spec
//! deduplication helper used before sending specs to the provider.

use crate::openhuman::agent::harness::artifact_offload::{
    render_artifact_offload_contract, should_render_offload_contract, ARTIFACT_OFFLOAD_HEADING,
};
use crate::openhuman::tools::ToolSpec;
use std::collections::HashSet;

// ─────────────────────────────────────────────────────────────────────────────
// Sub-agent role contract
// ─────────────────────────────────────────────────────────────────────────────

/// Prompt suffix injected into every typed sub-agent run.
///
/// Purpose:
/// - make the child explicitly aware it is acting as a sub-agent
/// - keep delegated outputs concise so parent-context growth stays bounded
/// - discourage verbose restatement of the delegated task/context
pub(super) const SUBAGENT_ROLE_CONTRACT_SUFFIX: &str = "## Sub-agent Role Contract\n\n\
You are a sub-agent working for a parent OpenHuman agent, not a direct end-user assistant.\n\
- Stay tightly scoped to the delegated task.\n\
- Keep tool arguments and follow-up prompts compact, include only required fields/context.\n\
- Keep your final response concise and synthesis-ready for the parent, prefer short bullets or short paragraphs.\n\
- Do not restate the full task/context unless strictly required for correctness.\n\
\n\
## Sub-agent Result Contract\n\n\
Return a compact result with these headings:\n\
- Answer\n\
- Evidence used\n\
- Actions taken\n\
- Open uncertainties\n\
- Failed tool calls\n\
- Recommended next step\n\
\n\
Do not include facts in Answer that are not supported by Evidence used or Actions taken.\n\
If a tool result was truncated, partial, or too large to inspect fully, say so under Open uncertainties and do not treat it as complete.\n";

pub(crate) fn append_subagent_role_contract(base_prompt: String, agent_id: &str) -> String {
    // `context_scout` defines its own strict output contract (emit a single
    // `[context_bundle]` and nothing else). The generic Result Contract here
    // (Answer / Evidence used / Actions taken / …) directly conflicts with
    // that and can make the scout emit the generic headings instead of the
    // bundle — leaving the orchestrator without `has_enough_context` /
    // `recommended_tool_calls`. Skip the suffix for the scout; its prompt.md
    // already carries the sub-agent framing it needs.
    if agent_id == "context_scout" {
        tracing::debug!(
            agent_id = %agent_id,
            "[subagent_runner] skipping role-contract suffix — agent defines its own output contract"
        );
        return base_prompt;
    }

    if base_prompt.contains(SUBAGENT_ROLE_CONTRACT_SUFFIX.trim()) {
        tracing::debug!(
            agent_id = %agent_id,
            base_chars = base_prompt.chars().count(),
            "[subagent_runner] sub-agent role contract already present in system prompt"
        );
        return base_prompt;
    }

    let mut prompt = base_prompt;
    if !prompt.ends_with('\n') {
        prompt.push('\n');
    }
    prompt.push('\n');
    prompt.push_str(SUBAGENT_ROLE_CONTRACT_SUFFIX);

    tracing::debug!(
        agent_id = %agent_id,
        suffix_chars = SUBAGENT_ROLE_CONTRACT_SUFFIX.chars().count(),
        final_chars = prompt.chars().count(),
        "[subagent_runner] appended sub-agent role contract to system prompt"
    );

    prompt
}

/// Append the filesystem-offload contract (#3883), but only for a sub-agent
/// that actually holds a file-write tool.
///
/// Kept separate from [`append_subagent_role_contract`] because it is **not**
/// universal. A sub-agent's system prompt may only name tools it can really
/// call — `researcher` is search + fetch only, and a skill-filtered specialist
/// sees just its own toolkit. Telling either to write a file yields
/// hallucinated calls that fail, and two guards enforce it
/// (`researcher::prompt::tests::build_returns_nonempty_body`,
/// `ops_tests::typed_mode_filters_tools_by_skill_filter`).
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
    if !should_render_offload_contract(visible_tool_names) {
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
    prompt.push_str(&render_artifact_offload_contract());

    tracing::debug!(
        agent_id = %agent_id,
        final_chars = prompt.chars().count(),
        "[artifact] appended offload contract to sub-agent system prompt"
    );

    prompt
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool-spec deduplication
// ─────────────────────────────────────────────────────────────────────────────

/// Deduplicate assembled tool specs by name, keeping the first occurrence.
///
/// The sub-agent's `filtered_specs` is a `Vec` assembled from
/// `parent.all_tool_specs` indices plus dynamic tools, so a delegation tool can
/// shadow a same-named skill/integration tool (common for the wide-set
/// `tools_agent`), leaving two specs with the same name. Strict providers reject
/// such a request with `400 "Tool names must be unique."` The main-agent path
/// dedups via [`session::builder::dedup_visible_tool_specs`]; this separate
/// sub-agent assembly must do the same.
///
/// First occurrence wins so registration-order semantics are preserved (tool
/// dispatch still resolves by name). Dropped duplicates are logged at `debug`
/// (diagnostic instrumentation, per the repo Rust logging guideline).
///
/// Extracted as a free function so the regression suite can exercise the dedup
/// without standing up the full `run_typed_mode` plumbing.
pub(super) fn dedup_tool_specs_by_name(agent_id: &str, specs: Vec<ToolSpec>) -> Vec<ToolSpec> {
    let mut seen: HashSet<String> = HashSet::with_capacity(specs.len());
    let mut deduped: Vec<ToolSpec> = Vec::with_capacity(specs.len());
    let mut dropped: Vec<String> = Vec::new();
    for spec in specs {
        if seen.insert(spec.name.clone()) {
            deduped.push(spec);
        } else {
            dropped.push(spec.name);
        }
    }
    if !dropped.is_empty() {
        tracing::debug!(
            agent_id = %agent_id,
            "[subagent_runner] dropped {} duplicate tool spec(s) before sending to provider: {:?}",
            dropped.len(),
            dropped
        );
    }
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_scout_skips_role_contract_suffix() {
        // The scout owns its own [context_bundle] output contract; the generic
        // result contract must not be appended (it conflicts).
        let base = "scout prompt body".to_string();
        let out = append_subagent_role_contract(base.clone(), "context_scout");
        assert_eq!(out, base);
        assert!(!out.contains("Sub-agent Result Contract"));
    }

    #[test]
    fn other_agents_get_role_contract_suffix() {
        let out = append_subagent_role_contract("body".to_string(), "researcher");
        assert!(out.contains("Sub-agent Result Contract"));
        assert!(out.contains("Recommended next step"));
    }
}
