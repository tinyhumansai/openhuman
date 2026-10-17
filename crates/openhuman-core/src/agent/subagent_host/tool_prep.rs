//! Helpers that prepare the sub-agent's tool surface and system prompt
//! body before [`super::run_typed_mode`] spins up its tool-loop.
//!
//! Kept together because they share a theme (what does the sub-agent
//! actually see?).

use super::types::SubagentRunError;
use crate::agent::harness::host_definition::AgentDefinition;
use crate::agent::prompts::PromptContext;
use tinyagents_definition::{PromptSource, ToolScope};
use tinytools::Tool;

// ── Prompt dialect selection ────────────────────────────────────────────

/// The tool-call format a sub-agent's prompt is rendered for, and the
/// dispatcher protocol block that goes with it. Both follow the parent.
///
/// Tool syntax and tool catalogue rendering live in TinyTools. Keeping only
/// dialect selection here prevents OpenHuman prompt text from drifting from
/// the parser that consumes the response.
pub(crate) fn subagent_prompt_protocol(
    parent_format: tinytools_agent::dialect::ToolCallFormat,
    tools: &[tinytools::ToolSpec],
) -> (tinytools_agent::dialect::ToolCallFormat, String) {
    use tinytools_agent::dialect::ToolCallFormat;
    use tinytools_agent::dialect::{
        CodeDialect, CodeStyle, NativeDialect, PFormatDialect, ToolDialect,
    };
    let instructions = match parent_format {
        ToolCallFormat::PFormat => {
            let registry = tinytools_agent::build_registry(
                tools
                    .iter()
                    .map(|tool| (tool.name.as_str(), &tool.parameters)),
            );
            PFormatDialect::new(registry).prompt_instructions(tools)
        }
        // The native request carries its own structured catalogue; retain
        // only the dialect guidance in the text prompt.
        ToolCallFormat::Native => NativeDialect.prompt_instructions(&[]),
        ToolCallFormat::Json => {
            crate::agent::prompts::render_helpers::harness_json_tool_prompt(tools)
        }
        ToolCallFormat::Python => CodeDialect::instructions(CodeStyle::Python),
        ToolCallFormat::TypeScript => CodeDialect::instructions(CodeStyle::TypeScript),
    };
    (parent_format, instructions)
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
///   ([`crate::tools::orchestrator_tools::collect_orchestrator_tools`]
///   emits `delegate_code_executor`, `delegate_planner`, …).
///
/// Kept as a tight prefix/exact match rather than a registry lookup so
/// the strip is cheap to run inside [`super::ops::run_typed_mode`]'s
/// filter pass. If the delegation-tool naming scheme changes, update
/// this function and the corresponding generator in
/// `orchestrator_tools.rs` together.
pub(super) fn is_subagent_spawn_tool(name: &str) -> bool {
    if name == "spawn_subagent" || name.starts_with("delegate_") {
        return true;
    }
    // Synthesised delegation tools are named by the target agent's
    // `delegate_name` override, which mostly does NOT carry the `delegate_`
    // prefix (`manage_tasks`, `create_image`, `setup_skills`,
    // `build_workflow`, …). The prefix check above misses every one of them,
    // which let wildcard-scoped children inherit the orchestrator's spawn
    // surface. Resolve the override names via the registry so the strip
    // stays in lockstep with `collect_orchestrator_tools`'s naming.
    if let Some(registry) =
        crate::agent::harness::host_definition::AgentDefinitionRegistry::current()
    {
        return registry
            .list()
            .iter()
            .any(|def| def.delegate_name.as_deref() == Some(name));
    }
    false
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
pub(super) fn filter_tool_indices(
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
            if crate::tools::rules::glob_list_matches(disallowed, name) {
                return false;
            }
            // The CCR recovery tool is advertised to any agent that has a tool
            // surface — compaction applies to its tool output, so the retrieve
            // footer must be actionable regardless of scope/skill filters (an
            // explicit `disallow` above still wins). A deliberately tool-less
            // agent (`Named([])`, e.g. the payload summarizer) runs no tools,
            // produces no compacted output, and so stays tool-less.
            if crate::inference::tokenjuice::is_recovery_tool(name) {
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

/// Intersect a child definition's tool indices with the tools the parent turn
/// actually exposes. An empty parent set is the legacy "unknown/unrestricted"
/// sentinel used by internal callers and older tests.
pub(super) fn retain_parent_visible_tool_indices(
    indices: &mut Vec<usize>,
    parent_tools: &[Box<dyn Tool>],
    parent_visible: &std::collections::HashSet<String>,
) {
    if parent_visible.is_empty() {
        return;
    }
    indices.retain(|&index| parent_visible.contains(parent_tools[index].name()));
}

#[cfg(test)]
#[path = "tool_prep_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tool_prep_recovery_visibility_tests_tests.rs"]
mod recovery_visibility_tests;

// ── Prompt loading ──────────────────────────────────────────────────────

/// Resolve a [`PromptSource`] to its raw markdown body. Inline sources
/// return immediately, `Dynamic` calls the builder with the supplied
/// [`PromptContext`], `File` sources are read from disk relative to the
/// workspace `prompts/` directory or the agent crate's bundled prompts.
///
/// The allowed indices whose tools the sub-agent's rules let it list. A
/// withheld tool stays allowed — the harness gate decides calls — but is kept
/// off the specs and catalogue the host renders into the child's prompt.
pub(super) fn rule_listed_indices(
    allowed: &[usize],
    tools: &[Box<dyn Tool>],
    rules: Option<&tinyagents_harness::tool::ToolRulePolicy>,
) -> Vec<usize> {
    let Some(rules) = rules else {
        return allowed.to_vec();
    };
    allowed
        .iter()
        .copied()
        .filter(|&i| {
            let tool = tools[i].as_ref();
            let surface = if tool.exposure() == tinytools::ToolExposure::Deferred {
                tinytools::Surface::Search
            } else {
                tinytools::Surface::Catalog
            };
            rules
                .rules
                .visible(&tinytools::ToolSubject::of(tool), &rules.context, surface)
        })
        .collect()
}

/// The prompt catalogue entries for `indices`.
pub(super) fn prompt_tools_for<'a>(
    indices: &[usize],
    tools: &'a [Box<dyn Tool>],
) -> Vec<crate::agent::prompts::PromptTool<'a>> {
    indices
        .iter()
        .map(|&i| {
            let tool = tools[i].as_ref();
            crate::agent::prompts::PromptTool {
                name: std::borrow::Cow::Borrowed(tool.name()),
                description: std::borrow::Cow::Borrowed(tool.description()),
                parameters_schema: Some(tool.parameters_schema().to_string()),
            }
        })
        .collect()
}

pub(super) fn load_prompt_source(
    definition: &AgentDefinition,
    ctx: &PromptContext<'_>,
) -> Result<String, SubagentRunError> {
    let workspace_dir = ctx.workspace_dir;
    match &definition.system_prompt {
        PromptSource::Inline(body) | PromptSource::Verbatim(body) => Ok(body.clone()),
        PromptSource::Dynamic(()) => {
            let dynamic_path = || format!("<dynamic:{}>", ctx.agent_id);
            let build = definition.extensions.prompt_builder.ok_or_else(|| {
                SubagentRunError::PromptLoad {
                    path: dynamic_path(),
                    source: std::io::Error::other("dynamic prompt has no registered builder"),
                }
            })?;
            build(ctx).map_err(|e| SubagentRunError::PromptLoad {
                path: dynamic_path(),
                source: std::io::Error::other(e.to_string()),
            })
        }
        PromptSource::File { path } => {
            // Try the workspace's `agent/prompts/` first (so users can
            // override built-in prompts), then fall back to the crate's
            // own bundled prompts via `include_str!`-style lookup.
            let prompt_root = workspace_dir.join("agent").join("prompts");
            let workspace_path = prompt_root.join(path);
            if workspace_path.is_file() {
                if let Ok(resolved) =
                    crate::security::validate_path_within_root(&workspace_path, &prompt_root)
                {
                    return std::fs::read_to_string(&resolved).map_err(|e| {
                        SubagentRunError::PromptLoad {
                            path: resolved.display().to_string(),
                            source: e,
                        }
                    });
                }
                tracing::warn!(
                    "[subagent_host] prompt path escapes workspace, skipping: {}",
                    workspace_path.display()
                );
            }
            // Built-in prompt fallback. The agent prompts directory is
            // already shipped at `crates/openhuman-core/src/agent/prompts/` and
            // included in the binary via the `IdentitySection` workspace
            // file write — so we re-use that scaffolding by reading from
            // `<workspace>/<filename>` after the parent agent has
            // bootstrapped its workspace files. For sub-agent
            // archetype prompts (e.g. `archetypes/critic.md`),
            // we look up by basename in the workspace, then accept
            // missing files as an empty body (the runner will fall
            // back to a generic role hint).
            let workspace_root_path = workspace_dir.join(path);
            if workspace_root_path.is_file() {
                if let Ok(resolved) =
                    crate::security::validate_path_within_root(&workspace_root_path, workspace_dir)
                {
                    return std::fs::read_to_string(&resolved).map_err(|e| {
                        SubagentRunError::PromptLoad {
                            path: resolved.display().to_string(),
                            source: e,
                        }
                    });
                }
                tracing::warn!(
                    "[subagent_host] fallback prompt path escapes workspace, skipping: {}",
                    workspace_root_path.display()
                );
            }
            tracing::warn!(
                path = %path,
                "[subagent_host] archetype prompt file not found, using empty body"
            );
            Ok(String::new())
        }
    }
}
