//! Helpers that prepare the sub-agent's tool surface and system prompt
//! body before [`super::run_typed_mode`] spins up its tool-loop.
//!
//! Kept together because they share a theme (what does the sub-agent
//! actually see?). Only the text-mode protocol renderer is exposed outside
//! this module so the debug-dump path in [`crate::openhuman::agent::debug`] can
//! mirror the live runner byte-for-byte instead of carrying its own drifting
//! copy.

use super::super::definition::{PromptSource, ToolScope};
use super::types::SubagentRunError;
use crate::openhuman::agent::context::prompt::PromptContext;
use crate::openhuman::tools::Tool;

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

/// Actions a toolkit must keep on the sub-agent's surface even when the
/// fuzzy ranker does not pick them.
///
/// The ranker (`tinyagents_harness::tool::select`) derives an intent verb
/// from the prompt and then **drops** every tool whose own name-verb
/// conflicts with it. `FETCH`/`GET` are the `Read` verb and `find`/`search`
/// are `List`, so a plain "find the emails about X" prompt removes
/// `GMAIL_FETCH_EMAILS` and both `GMAIL_FETCH_MESSAGE_BY_*` before scoring
/// even starts. A flat verb-alignment bonus then lets zero-overlap tools
/// fill the budget, so the surviving surface can be twelve `GMAIL_LIST_*`
/// actions — ids and settings, nothing that returns a message body. The
/// sub-agent burns its iterations and answers without the email (#6033).
///
/// These names are **reserved inside** the top-K budget, never appended
/// past it: [`TOOL_FILTER_TOP_K_HEAVY`] exists because Gmail at top-K=25
/// produced a 276k-token first-iteration prompt, so an essential displaces
/// the lowest-ranked hit rather than growing the surface.
///
/// **Every entry must be read-only.** These actions are forced onto a
/// delegation whatever it asked for, so a write action here would hand a
/// "read my email" task an unrequested side-effecting capability — and a
/// `ComposioActionTool` does not override `external_effect`, so the
/// approval middleware (which gates on `external_effect_with_args`) would
/// not stop it. Prompt-injected instructions inside a fetched email could
/// then act on the mailbox with no human in the loop. Send and the rest of
/// the write surface stay where they were: reachable when the prompt
/// actually asks for them, via the ranker.
///
/// Only Gmail is seeded, deliberately. A wrong entry spends a slot on
/// every prompt for that toolkit, so add one only with a fixture-backed
/// repro — the ranker's general behaviour is the upstream fix.
const TOOLKIT_ESSENTIAL_ACTIONS: &[(&str, &[&str])] = &[(
    "gmail",
    &[
        // The only action that returns message *content* for a search.
        "GMAIL_FETCH_EMAILS",
        // The follow-up once `GMAIL_LIST_MESSAGES` has handed back bare ids.
        "GMAIL_FETCH_MESSAGE_BY_MESSAGE_ID",
    ],
)];

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

/// Actions [`TOOLKIT_ESSENTIAL_ACTIONS`] reserves for `toolkit`, or an
/// empty slice when the toolkit has no entry. Case-insensitive, matching
/// [`top_k_for_toolkit`].
pub(super) fn essential_actions_for_toolkit(toolkit: &str) -> &'static [&'static str] {
    TOOLKIT_ESSENTIAL_ACTIONS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(toolkit))
        .map(|(_, actions)| *actions)
        .unwrap_or(&[])
}

/// Pick the action indices the sub-agent will actually see: the toolkit's
/// essentials first, then the ranker's hits in rank order, deduplicated and
/// truncated to `top_k`.
///
/// An essential that the connected toolkit does not actually expose is
/// skipped rather than guessed at, so a stale table entry costs nothing and
/// can never produce an out-of-range index. A toolkit with no entry returns
/// the ranker's hits unchanged (capped at `top_k`), which is what every
/// call site did before the essentials existed.
pub(super) fn select_actions_with_essentials(
    toolkit: &str,
    actions: &[crate::openhuman::agent::context::prompt::ConnectedIntegrationTool],
    filter_hits: &[usize],
    top_k: usize,
) -> Vec<usize> {
    let essentials = essential_actions_for_toolkit(toolkit);
    if essentials.is_empty() || top_k == 0 {
        return filter_hits.iter().take(top_k).copied().collect();
    }

    let mut selected: Vec<usize> = Vec::with_capacity(top_k.min(actions.len()));
    let mut reserved: Vec<&str> = Vec::new();
    for essential in essentials {
        if selected.len() >= top_k {
            break;
        }
        let found = actions
            .iter()
            .position(|a| a.name.eq_ignore_ascii_case(essential));
        if let Some(idx) = found {
            if !selected.contains(&idx) {
                selected.push(idx);
                reserved.push(essential);
            }
        }
    }

    let ranked_before = filter_hits.len().min(top_k);
    for &idx in filter_hits {
        if selected.len() >= top_k {
            break;
        }
        if !selected.contains(&idx) {
            selected.push(idx);
        }
    }

    // A reserved action the ranker had already chosen costs nothing; the
    // rest displace the lowest-ranked hits that no longer fit.
    let ranked_kept = filter_hits.iter().filter(|i| selected.contains(i)).count();
    tracing::debug!(
        toolkit = %toolkit,
        reserved = %reserved.join(","),
        kept = selected.len(),
        top_k = top_k,
        dropped_ranked = ranked_before.saturating_sub(ranked_kept),
        "[subagent_runner:tools] reserved essential actions inside the top-K budget"
    );
    selected
}

// ── Text-mode protocol block ────────────────────────────────────────────

/// Format the tool-use protocol block appended to the system prompt in text
/// mode. Teaches **P-Format** first (the same protocol
/// [`crate::openhuman::agent::dispatcher::PFormatToolDispatcher`] renders and
/// the tinyagents adapter parses via `parse_tool_calls_with_pformat`), with
/// the legacy JSON-in-tag form as the documented fallback for nested
/// arguments. The `## Tools` catalogue already renders `Call as:` p-format
/// signatures for every tool, so teaching JSON here contradicted the
/// catalogue and threw away the p-format token savings.
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
pub(crate) fn build_text_mode_tool_instructions() -> String {
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
        "Tool calls use **P-Format** (Parameter-Format): compact, positional, \
         pipe-delimited syntax wrapped in `<tool_call>` tags.\n\n",
    );
    out.push_str("```\n<tool_call>\nGMAIL_FETCH_EMAILS[ca_123||10]\n</tool_call>\n```\n\n");
    out.push_str(
        "**Rules:**\n\
         - Form: `name[arg1|arg2|...|argN]`. Arguments are positional and must match the \
           order shown in each tool's `Call as:` signature in the `## Tools` section \
           (alphabetical by parameter name). Leave a slot empty to omit that argument.\n\
         - Empty calls: `name[]` for zero-arg tools.\n\
         - Escapes inside argument values: `\\|` for a literal `|`, `\\]` for `]`, `\\\\` for `\\`.\n\
         - Do not nest tags. Emit one tag per call; you can emit multiple tags in the same \
           response to run calls in parallel.\n\
         - When an argument needs a nested object or array that p-format cannot express, \
           fall back to the JSON form in the same tags: \
           `<tool_call>{\"name\": \"tool_name\", \"arguments\": {\"param\": \"value\"}}</tool_call>`. \
           Prefer p-format for everything else.\n",
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
    if name == "spawn_subagent" || name.starts_with("delegate_") || name == "agent_prepare_context"
    {
        return true;
    }
    // Synthesised delegation tools are named by the target agent's
    // `delegate_name` override, which mostly does NOT carry the `delegate_`
    // prefix (`plan`, `run_code`, `research`, `review_code`, `do_crypto`,
    // `schedule_task`, …). The prefix check above misses every one of them,
    // which let wildcard-scoped children inherit the orchestrator's spawn
    // surface. Resolve the override names via the registry so the strip
    // stays in lockstep with `collect_orchestrator_tools`'s naming.
    if let Some(registry) =
        crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
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
            if disallowed_tool_matches(disallowed, name) {
                return false;
            }
            // The CCR recovery tool is advertised to any agent that has a tool
            // surface — compaction applies to its tool output, so the retrieve
            // footer must be actionable regardless of scope/skill filters (an
            // explicit `disallow` above still wins). A deliberately tool-less
            // agent (`Named([])`, e.g. the payload summarizer) runs no tools,
            // produces no compacted output, and so stays tool-less.
            if crate::openhuman::inference::tokenjuice::is_recovery_tool(name) {
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

pub(super) fn disallowed_tool_matches(disallowed: &[String], name: &str) -> bool {
    disallowed.iter().any(|entry| {
        if let Some(prefix) = entry.strip_suffix('*') {
            name.starts_with(prefix)
        } else {
            entry == name
        }
    })
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
pub(super) fn load_prompt_source(
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

/// Remove every sub-agent spawn/delegate tool from a child's **dynamic**
/// (per-spawn) tool list, in place.
///
/// The archetype's static surface is stripped via `allowed_indices` in
/// [`super::ops::run_typed_mode`], but dynamic tools — per-action Composio
/// toolkit tools and `extract_from_result` — are appended afterwards and never
/// pass through that filter. That makes this the only route by which a
/// spawn/delegate name can reach a child's allowlist *admitted*, so the #4452
/// invariant has to be re-asserted here (issue #6157).
///
/// Strip once, at the point the list is finished, rather than at each use: the
/// same `Vec` feeds the provider-visible specs, the resolved allowlist, the
/// prompt catalogue, harness registration and the custom-graph request. Filter
/// one and the others disagree — a tool advertised but not admitted, or the
/// reverse. The registration-time backstop in
/// `tinyagents::is_subagent_spawn_or_delegate_tool` is not sufficient on its
/// own here: it cannot resolve `delegate_name` overrides, so it matches
/// strictly less than [`is_subagent_spawn_tool`] does.
pub(super) fn strip_spawn_tools_from_dynamic(
    dynamic_tools: &mut Vec<Box<dyn Tool>>,
    agent_id: &str,
) {
    dynamic_tools.retain(|tool| {
        let name = tool.name();
        // `spawn_worker_thread` is matched separately for the same reason the
        // caller-side strip does it: it spawns a run without being a delegate.
        let is_spawn = is_subagent_spawn_tool(name) || name == "spawn_worker_thread";
        if is_spawn {
            tracing::warn!(
                agent_id = %agent_id,
                tool = name,
                "[subagent_runner] dropped a spawn/delegate tool from a sub-agent's dynamic tools"
            );
        }
        !is_spawn
    });
}
