//! Helpers that prepare the sub-agent's tool surface and system prompt
//! body before [`super::run_typed_mode`] spins up its tool-loop.
//!
//! Kept together because they share a theme (what does the sub-agent
//! actually see?) and because several of them are exposed `pub(crate)`
//! so the debug-dump path in
//! [`crate::openhuman::agent::debug`] can mirror the live runner
//! byte-for-byte instead of carrying its own drifting copies.

use super::super::definition::{PromptSource, ToolScope};
use super::types::SubagentRunError;
use crate::openhuman::context::prompt::PromptContext;
use crate::openhuman::tools::{Tool, ToolSpec};

// ── Heavy-schema toolkit accounting ─────────────────────────────────────

/// Tight top-K ceiling for toolkits whose per-action JSON schemas are
/// dense enough to blow through either Fireworks' 65 535-rule grammar
/// cap (native mode) or the 196 607-token context cap (text mode) even
/// before any tool results land in history. Determined empirically from
/// the fixture dumps under `tests/fixtures/composio_*.json` and real
/// staging failures — see the trace where Gmail at top-K=25 produced
/// a 276k-token iter-1 prompt.
const HEAVY_SCHEMA_TOOLKITS: &[&str] = &[
    "gmail",
    "notion",
    "github",
    "salesforce",
    "hubspot",
    "googledrive",
    "googlesheets",
    "googledocs",
    "microsoftteams",
];

const TOOL_FILTER_TOP_K_DEFAULT: usize = 25;
const TOOL_FILTER_TOP_K_HEAVY: usize = 12;

/// Pick a top-K budget for the fuzzy filter based on how dense the
/// toolkit's action schemas tend to be. Match is case-insensitive so
/// we don't care whether the caller passed `"Gmail"` or `"gmail"`.
pub(super) fn top_k_for_toolkit(toolkit: &str) -> usize {
    if HEAVY_SCHEMA_TOOLKITS
        .iter()
        .any(|t| t.eq_ignore_ascii_case(toolkit))
    {
        TOOL_FILTER_TOP_K_HEAVY
    } else {
        TOOL_FILTER_TOP_K_DEFAULT
    }
}

// ── Text-mode protocol block ────────────────────────────────────────────

/// Format a set of `ToolSpec`s as an XML tool-use protocol block
/// appended to the system prompt in text mode. Mirrors
/// [`crate::openhuman::agent::dispatcher::XmlToolDispatcher::prompt_instructions`]
/// — same `<tool_call>{…}</tool_call>` format so the existing
/// `parse_tool_calls` helper understands what the model emits.
///
/// Per-parameter rendering is intentionally **compact**: name, type, a
/// "required" marker, and a short one-line description if present. We
/// do **not** serialise the full JSON schema. Composio/Fireworks action
/// schemas for toolkits like Gmail or Notion run multiple KB each —
/// embedding them verbatim blows up the prompt past the model's
/// context window (282k+ tokens for 26 Gmail tools vs a 196k cap).
/// The compact listing keeps the model informed enough to call tools
/// correctly while staying within budget. If the model needs deeper
/// schema detail it can surface the error and the orchestrator will
/// clarify on the next turn.
pub(crate) fn build_text_mode_tool_instructions(_specs: &[ToolSpec]) -> String {
    // The tool catalog is already rendered in the prompt's `## Tools`
    // section (see `prompts::ToolsSection::build`) with full
    // `Call as: NAME[arg|arg]` signatures. We previously also emitted
    // an `### Available Tools` subsection here with a different
    // formatting (`Parameters: name:type, ...`), which doubled the
    // tool list bytes for text-mode agents — especially expensive for
    // the integrations_agent toolkit-scoped spawns (~50 actions ×
    // 2 listings). Keep only the protocol explanation; the tool
    // catalog itself comes from the prompt template.
    let mut out = String::new();
    out.push_str("## Tool Use Protocol\n\n");
    out.push_str(
        "To use a tool, wrap a JSON object in <tool_call></tool_call> tags. \
         Do not nest tags. Emit one tag per call; you can emit multiple tags \
         in the same response if you need to run calls in parallel.\n\n",
    );
    out.push_str(
        "```\n<tool_call>\n{\"name\": \"tool_name\", \"arguments\": {\"param\": \"value\"}}\n</tool_call>\n```\n",
    );
    out
}

// ── Tool filtering ──────────────────────────────────────────────────────

/// Tools that spawn a new sub-agent turn. A sub-agent must never be
/// able to invoke any of these — only the top-level orchestrator
/// delegates. Nested spawns would create a recursion tree the harness
/// is not designed to budget, cost, or observe.
///
/// Matches:
/// * the generic `spawn_subagent` meta-tool (arbitrary archetype by id);
/// * every synthesised per-archetype `delegate_*` tool
///   ([`crate::openhuman::tools::orchestrator_tools::collect_orchestrator_tools`]
///   emits `delegate_researcher`, `delegate_planner`, …).
/// * custom delegate names that intentionally do not use the `delegate_*`
///   prefix, currently `use_tinyplace`.
/// * `agent_prepare_context` — the context-scout entry point. It reads the
///   *parent's* visible catalog/session via `current_parent()`, which inside a
///   nested run is still the top-level orchestrator (the runner does not
///   install a child-scoped parent context). A wildcard or named sub-agent
///   calling it would therefore scout against the orchestrator's surface, not
///   its own. Context preparation is a top-level concern only.
///
/// Kept as a tight prefix/exact match rather than a registry lookup so
/// the strip is cheap to run inside [`super::ops::run_typed_mode`]'s
/// filter pass. If the delegation-tool naming scheme changes, update
/// this function and the corresponding generator in
/// `orchestrator_tools.rs` together.
pub(super) fn is_subagent_spawn_tool(name: &str) -> bool {
    name == "spawn_subagent"
        || name.starts_with("delegate_")
        || name == "use_tinyplace"
        || name == "agent_prepare_context"
}

/// Returns indices into `parent_tools` for the tools the sub-agent may
/// invoke. Index-based filtering avoids cloning `Box<dyn Tool>` (which
/// isn't Clone) and lets us reuse the parent's existing instances.
///
/// Filters are applied in this order (shorter-circuit first):
/// 1. `disallowed` — explicit deny list.
/// 2. `skill_filter` — restrict to tools named `{skill}__*`.
/// 3. `scope` — `Wildcard` (everything remaining) or `Named` allowlist.
///
/// Exposed `pub(crate)` so the debug dump path in
/// [`crate::openhuman::agent::debug`] shares the exact same
/// filter logic as the live runner instead of keeping a separate copy.
pub(crate) fn filter_tool_indices(
    parent_tools: &[Box<dyn Tool>],
    scope: &ToolScope,
    disallowed: &[String],
    skill_filter: Option<&str>,
) -> Vec<usize> {
    let skill_prefix = skill_filter.map(|s| format!("{s}__"));

    parent_tools
        .iter()
        .enumerate()
        .filter(|(_, tool)| {
            let name = tool.name();
            if disallowed_tool_matches(disallowed, name) {
                return false;
            }
            // The CCR recovery tool is advertised to any agent that has a tool
            // surface — compaction applies to its tool output, so the retrieve
            // footer must be actionable regardless of scope/skill filters (an
            // explicit `disallow` above still wins). A deliberately tool-less
            // agent (`Named([])`, e.g. the payload summarizer) runs no tools,
            // produces no compacted output, and so stays tool-less.
            if crate::openhuman::tokenjuice::is_recovery_tool(name) {
                return !matches!(scope, ToolScope::Named(allowed) if allowed.is_empty());
            }
            if let Some(prefix) = skill_prefix.as_deref() {
                if !name.starts_with(prefix) {
                    return false;
                }
            }
            match scope {
                ToolScope::Wildcard => true,
                ToolScope::Named(allowed) => allowed.iter().any(|n| n == name),
            }
        })
        .map(|(i, _)| i)
        .collect()
}

pub(crate) fn disallowed_tool_matches(disallowed: &[String], name: &str) -> bool {
    disallowed.iter().any(|entry| {
        if let Some(prefix) = entry.strip_suffix('*') {
            name.starts_with(prefix)
        } else {
            entry == name
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_tinyplace_delegate_is_treated_as_spawn_tool() {
        assert!(is_subagent_spawn_tool("spawn_subagent"));
        assert!(is_subagent_spawn_tool("delegate_researcher"));
        assert!(is_subagent_spawn_tool("use_tinyplace"));
        // Context scouting is top-level only — never visible to sub-agents
        // (incl. wildcard agents), which would otherwise scout the wrong
        // parent context. See #3949 review.
        assert!(is_subagent_spawn_tool("agent_prepare_context"));
        assert!(!is_subagent_spawn_tool("tinyplace_directory_resolve"));
    }
}

#[cfg(test)]
mod recovery_visibility_tests {
    use super::*;
    use crate::openhuman::tokenjuice::LEGACY_RETRIEVE_TOOL_NAME as RECOVERY_TOOL_NAME;
    use crate::openhuman::tools::{CurrentTimeTool, RetrieveToolOutputTool};

    fn tools() -> Vec<Box<dyn crate::openhuman::tools::Tool>> {
        vec![
            Box::new(CurrentTimeTool::new()),
            Box::new(RetrieveToolOutputTool::new()),
        ]
    }

    fn names(idx: &[usize], tools: &[Box<dyn crate::openhuman::tools::Tool>]) -> Vec<String> {
        idx.iter().map(|&i| tools[i].name().to_string()).collect()
    }

    #[test]
    fn named_scope_still_includes_recovery_tool() {
        let t = tools();
        // Named scope allow-lists only current_time — recovery tool not listed.
        let idx = filter_tool_indices(
            &t,
            &ToolScope::Named(vec!["current_time".into()]),
            &[],
            None,
        );
        let got = names(&idx, &t);
        assert!(got.contains(&"current_time".to_string()));
        assert!(
            got.contains(&RECOVERY_TOOL_NAME.to_string()),
            "recovery tool must survive Named scope: {got:?}"
        );
    }

    #[test]
    fn tool_less_agent_stays_tool_less() {
        // A deliberately tool-less agent (e.g. the payload summarizer,
        // ToolScope::Named([])) runs no tools and produces no compacted output,
        // so it must NOT be handed the recovery tool — it stays empty.
        let t = tools();
        let idx = filter_tool_indices(&t, &ToolScope::Named(vec![]), &[], None);
        assert!(idx.is_empty(), "empty scope must yield zero tools: {idx:?}");
    }

    #[test]
    fn skill_filter_still_includes_recovery_tool() {
        let t = tools();
        // A skill-restricted subagent (only `foo__*` tools) must still get it.
        let idx = filter_tool_indices(&t, &ToolScope::Wildcard, &[], Some("foo"));
        assert!(names(&idx, &t).contains(&RECOVERY_TOOL_NAME.to_string()));
    }

    #[test]
    fn explicit_disallow_still_wins() {
        let t = tools();
        let idx = filter_tool_indices(
            &t,
            &ToolScope::Wildcard,
            &[RECOVERY_TOOL_NAME.to_string()],
            None,
        );
        assert!(!names(&idx, &t).contains(&RECOVERY_TOOL_NAME.to_string()));
    }
}

// ── Prompt loading ──────────────────────────────────────────────────────

/// Resolve a [`PromptSource`] to its raw markdown body. Inline sources
/// return immediately, `Dynamic` calls the builder with the supplied
/// [`PromptContext`], `File` sources are read from disk relative to the
/// workspace `prompts/` directory or the agent crate's bundled prompts.
///
/// Exposed `pub(crate)` so the debug dump path in
/// [`crate::openhuman::agent::debug`] loads prompts through the
/// exact same code the runner uses instead of keeping a separate copy.
pub(crate) fn load_prompt_source(
    source: &PromptSource,
    ctx: &PromptContext<'_>,
) -> Result<String, SubagentRunError> {
    let workspace_dir = ctx.workspace_dir;
    match source {
        PromptSource::Inline(body) => Ok(body.clone()),
        PromptSource::Dynamic(build) => build(ctx).map_err(|e| SubagentRunError::PromptLoad {
            path: format!("<dynamic:{}>", ctx.agent_id),
            source: std::io::Error::other(e.to_string()),
        }),
        PromptSource::File { path } => {
            // Try the workspace's `agent/prompts/` first (so users can
            // override built-in prompts), then fall back to the crate's
            // own bundled prompts via `include_str!`-style lookup.
            let prompt_root = workspace_dir.join("agent").join("prompts");
            let workspace_path = prompt_root.join(path);
            if workspace_path.is_file() {
                if let Ok(resolved) = crate::openhuman::security::validate_path_within_root(
                    &workspace_path,
                    &prompt_root,
                ) {
                    return std::fs::read_to_string(&resolved).map_err(|e| {
                        SubagentRunError::PromptLoad {
                            path: resolved.display().to_string(),
                            source: e,
                        }
                    });
                }
                tracing::warn!(
                    "[subagent_runner] prompt path escapes workspace, skipping: {}",
                    workspace_path.display()
                );
            }
            // Built-in prompt fallback. The agent prompts directory is
            // already shipped at `src/openhuman/agent/prompts/` and
            // included in the binary via the `IdentitySection` workspace
            // file write — so we re-use that scaffolding by reading from
            // `<workspace>/<filename>` after the parent agent has
            // bootstrapped its workspace files. For sub-agent
            // archetype prompts (e.g. `archetypes/researcher.md`),
            // we look up by basename in the workspace, then accept
            // missing files as an empty body (the runner will fall
            // back to a generic role hint).
            let workspace_root_path = workspace_dir.join(path);
            if workspace_root_path.is_file() {
                if let Ok(resolved) = crate::openhuman::security::validate_path_within_root(
                    &workspace_root_path,
                    workspace_dir,
                ) {
                    return std::fs::read_to_string(&resolved).map_err(|e| {
                        SubagentRunError::PromptLoad {
                            path: resolved.display().to_string(),
                            source: e,
                        }
                    });
                }
                tracing::warn!(
                    "[subagent_runner] fallback prompt path escapes workspace, skipping: {}",
                    workspace_root_path.display()
                );
            }
            tracing::warn!(
                path = %path,
                "[subagent_runner] archetype prompt file not found, using empty body"
            );
            Ok(String::new())
        }
    }
}
