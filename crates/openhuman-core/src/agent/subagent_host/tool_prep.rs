//! Helpers that prepare the sub-agent's tool surface and system prompt
//! body before [`super::run_typed_mode`] spins up its tool-loop.
//!
//! Kept together because they share a theme (what does the sub-agent
//! actually see?).

use super::types::SubagentRunError;
use crate::agent::prompts::PromptContext;
use crate::agent_registry::host_definition::AgentDefinition;
use std::sync::Arc;
use tinyagents_definition::{PromptSource, ToolScope};
use tinyagents_harness::tool::ToolSnapshot;
use tinyagents_orchestration::subagent::{restrict_tools_with_scope, SubagentRole};
use tinytools::Tool;

/// Adapts the owned host catalogue to TinyAgents' shared Arc-backed
/// deduplication surface and returns the owned specs expected by dispatch.
pub(crate) fn dedup_owned_tool_specs(specs: Vec<tinytools::ToolSpec>) -> Vec<tinytools::ToolSpec> {
    tinyagents_runtime::dedup_visible_tool_specs(specs.into_iter().map(Arc::new).collect())
        .into_iter()
        .map(|spec| Arc::try_unwrap(spec).expect("newly wrapped spec has one owner"))
        .collect()
}

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
    let instructions = tinytools_agent::dialect::render_protocol_instructions(parent_format, tools);
    (parent_format, instructions)
}

// ── Tool filtering ──────────────────────────────────────────────────────

/// Tools that spawn a new sub-agent turn. A sub-agent must never be
/// able to invoke any of these — only the top-level orchestrator
/// delegates. Nested spawns would create a recursion tree the harness
/// is not designed to budget, cost, or observe.
///
/// Matches TinyAgents' shared spawn-tool list and OpenHuman's single spawn
/// tool. Specialist allowlist entries are arguments to that tool, not aliases.
pub(super) fn is_subagent_spawn_tool(name: &str) -> bool {
    crate::agent::tinyagents::turn_policy::is_subagent_spawn_tool(name)
}

/// Returns indices into `parent_tools` for the tools the sub-agent may
/// invoke. Index-based filtering avoids cloning `Box<dyn Tool>` (which
/// isn't Clone) and lets us reuse the parent's existing instances.
///
/// OpenHuman applies its explicit deny globs and the CCR recovery exception;
/// TinyAgents applies the portable role, skill, scope, and delegation rules.
///
pub(super) fn filter_tool_indices(
    parent_tools: &[Box<dyn Tool>],
    scope: &ToolScope,
    disallowed: &[String],
    skill_filter: Option<&str>,
) -> Vec<usize> {
    filter_tool_indices_with_nested_spawn(parent_tools, scope, disallowed, skill_filter, false)
}

/// Filters child tools with an explicit, host-validated nested-spawn grant.
pub(super) fn filter_tool_indices_with_nested_spawn(
    parent_tools: &[Box<dyn Tool>],
    scope: &ToolScope,
    disallowed: &[String],
    skill_filter: Option<&str>,
    nested_spawn_granted: bool,
) -> Vec<usize> {
    let mut first_specs = std::collections::HashMap::new();
    let mut specs = Vec::new();
    let mut disallowed_recovery = std::collections::HashSet::new();
    let mut delegation_tools = Vec::new();
    for tool in parent_tools {
        let name = tool.name();
        if is_subagent_spawn_tool(name) {
            delegation_tools.push(name.to_string());
        }
        if crate::tools::rules::glob_list_matches(disallowed, name) {
            if crate::inference::tokenjuice::is_recovery_tool(name) {
                disallowed_recovery.insert(name.to_string());
            }
            continue;
        }
        if first_specs.insert(name.to_string(), ()).is_none() {
            specs.push(tool.spec());
        }
    }
    let snapshot = match ToolSnapshot::new(specs) {
        Ok(snapshot) => snapshot,
        Err(_) => return Vec::new(),
    };
    let restricted = restrict_tools_with_scope(
        &snapshot,
        if nested_spawn_granted {
            SubagentRole::Orchestrator
        } else {
            SubagentRole::Leaf
        },
        None,
        &delegation_tools,
        Some(scope),
        skill_filter,
    );
    let mut retained: std::collections::HashSet<String> = restricted
        .specs()
        .iter()
        .map(|spec| spec.name.clone())
        .collect();

    // CCR recovery remains a host exception: it bypasses scope and skill
    // filters, while the deny glob, leaf role, and explicit empty scope win.
    if !matches!(scope, ToolScope::Named(allowed) if allowed.is_empty()) {
        for tool in parent_tools {
            let name = tool.name();
            if crate::inference::tokenjuice::is_recovery_tool(name)
                && !disallowed_recovery.contains(name)
                && !delegation_tools.iter().any(|tool| tool == name)
            {
                retained.insert(name.to_string());
            }
        }
    }

    parent_tools
        .iter()
        .enumerate()
        .filter_map(|(index, tool)| retained.contains(tool.name()).then_some(index))
        .collect()
}

pub(super) struct DefinitionToolFilter {
    pub indices: Vec<usize>,
    pub nested_spawn_ids: Vec<String>,
    pub nested_spawn_tools: std::collections::HashSet<String>,
}

/// Applies a child's definition scope, denylist, and checked host grant to
/// the parent's registered tools before the inherited visibility ceiling.
pub(super) fn filter_definition_tool_indices(
    parent_tools: &[Box<dyn Tool>],
    definition: &AgentDefinition,
    skill_filter: Option<&str>,
    nested_spawn_depth_available: bool,
    parent_visible: &std::collections::HashSet<String>,
) -> DefinitionToolFilter {
    let nested_spawn_ids = definition.allowed_subagent_ids();
    let explicitly_declares_nested_spawn = |name: &str| {
        is_subagent_spawn_tool(name)
            && (definition.extra_tools.iter().any(|tool| tool == name)
                || matches!(
                    &definition.tools,
                    ToolScope::Named(names) if names.iter().any(|tool| tool == name)
                ))
    };
    let nested_spawn_tools: std::collections::HashSet<String> = parent_tools
        .iter()
        .map(|tool| tool.name())
        .filter(|name| {
            nested_spawn_depth_available
                && !nested_spawn_ids.is_empty()
                && explicitly_declares_nested_spawn(name)
        })
        .map(str::to_owned)
        .collect();
    let nested_spawn_granted = !nested_spawn_tools.is_empty();
    let mut allowed = filter_tool_indices_with_nested_spawn(
        parent_tools,
        &definition.tools,
        &definition.disallowed_tools,
        skill_filter,
        nested_spawn_granted,
    );

    let before = allowed.len();
    allowed.retain(|&index| {
        let name = parent_tools[index].name();
        !is_subagent_spawn_tool(name) || nested_spawn_tools.contains(name)
    });
    let stripped = before - allowed.len();
    if stripped > 0 {
        tracing::debug!(
            agent_id = %definition.id,
            stripped,
            "[subagent_host] removed sub-agent spawn tools from sub-agent's tool surface"
        );
    }

    for (index, tool) in parent_tools.iter().enumerate() {
        let name = tool.name();
        if definition.extra_tools.iter().any(|extra| extra == name)
            && !allowed.contains(&index)
            && !crate::tools::rules::glob_list_matches(&definition.disallowed_tools, name)
            && (!is_subagent_spawn_tool(name) || nested_spawn_tools.contains(name))
        {
            allowed.push(index);
        }
    }

    retain_parent_visible_tool_indices(
        &mut allowed,
        parent_tools,
        parent_visible,
        nested_spawn_granted,
    );

    DefinitionToolFilter {
        indices: allowed,
        nested_spawn_ids,
        nested_spawn_tools,
    }
}

/// Intersect a child definition's tool indices with the tools the parent turn
/// actually exposes. An empty parent set is the legacy "unknown/unrestricted"
/// sentinel used by internal callers and older tests.
pub(super) fn retain_parent_visible_tool_indices(
    indices: &mut Vec<usize>,
    parent_tools: &[Box<dyn Tool>],
    parent_visible: &std::collections::HashSet<String>,
    nested_spawn_granted: bool,
) {
    if parent_visible.is_empty() {
        return;
    }
    let mut seen = std::collections::HashSet::new();
    let specs = parent_tools
        .iter()
        .filter(|tool| seen.insert(tool.name().to_string()))
        .map(|tool| tool.spec())
        .collect();
    let Ok(snapshot) = ToolSnapshot::new(specs) else {
        indices.clear();
        return;
    };
    let mut seen_ceiling = std::collections::HashSet::new();
    let ceiling_specs = parent_tools
        .iter()
        .filter(|tool| {
            parent_visible.contains(tool.name()) && seen_ceiling.insert(tool.name().to_string())
        })
        .map(|tool| tool.spec())
        .collect();
    let Ok(ceiling) = ToolSnapshot::new(ceiling_specs) else {
        indices.clear();
        return;
    };
    let narrowed = restrict_tools_with_scope(
        &snapshot,
        if nested_spawn_granted {
            SubagentRole::Orchestrator
        } else {
            SubagentRole::Leaf
        },
        Some(&ceiling),
        &[],
        None,
        None,
    );
    let names: std::collections::HashSet<_> = narrowed
        .specs()
        .iter()
        .map(|spec| spec.name.as_str())
        .collect();
    indices.retain(|&index| names.contains(parent_tools[index].name()));
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
            // archetype prompts (e.g. `archetypes/<agent_id>.md`),
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
