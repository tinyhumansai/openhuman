//! Built-in agent definitions.
//!
//! Every built-in agent lives in its own subfolder here, with exactly
//! two files:
//!
//! * `agent.toml`  — id, when_to_use, model, tool allowlist, sandbox,
//!   iteration cap, and the `omit_*` flags. Parsed
//!   directly into [`AgentDefinition`] via serde.
//! * `prompt.rs`   — a Rust module exporting `pub fn build(ctx: &PromptContext)
//!   -> anyhow::Result<String>` that returns the sub-agent's system
//!   prompt body. Dynamic: may branch on available tools, user profile,
//!   connected integrations, model hint, etc.
//!
//! Adding a new built-in agent = creating a new subfolder with those two
//! files, declaring the module, and appending one entry to [`BUILTINS`]
//! below. There are no match arms to update, no enum variants to add,
//! and no `include_str!` paths scattered across the harness.
//!
//! ## Flow
//!
//! 1. [`load_builtins`] walks [`BUILTINS`].
//! 2. For each entry, parses `agent.toml` into an [`AgentDefinition`].
//! 3. Replaces the (unset) `system_prompt` with `PromptSource::Inline(prompt.md contents)`.
//! 4. Stamps `source = DefinitionSource::Builtin`.
//! 5. Returns the full `Vec<AgentDefinition>`, in the order listed in [`BUILTINS`].
//!
//! The synthetic `fork` definition is *not* listed here — it's a
//! byte-stable replay of the parent and has no standalone prompt. It is
//! added by [`crate::openhuman::agent::harness::builtin_definitions::all`] on top of the
//! loader output.
//!
//! Workspace-level overrides (`$OPENHUMAN_WORKSPACE/agents/*.toml`) are
//! handled separately by [`crate::openhuman::agent::harness::definition_loader`] and merged
//! into the global registry, where they replace built-ins on `id`
//! collision.

use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentTier, DefinitionSource, PromptBuilder, PromptSource, SubagentEntry,
};
use anyhow::{Context, Result};
use std::collections::HashMap;

/// A single built-in agent: its id plus the metadata TOML and a
/// function-driven prompt builder.
///
/// Kept as a static slice (rather than e.g. `include_dir!`) so the
/// compile-time file-existence check is explicit and grep-friendly.
pub struct BuiltinAgent {
    pub id: &'static str,
    pub toml: &'static str,
    /// Prompt builder. Invoked at spawn time by the sub-agent runner
    /// with a populated [`crate::openhuman::agent::harness::definition::PromptContext`]
    /// so the returned body can branch on runtime state.
    pub prompt_fn: PromptBuilder,
}

/// Every built-in agent, in stable display order.
///
/// **This is the only list you touch when adding a new built-in agent.**
pub const BUILTINS: &[BuiltinAgent] = &[
    BuiltinAgent {
        id: "orchestrator",
        toml: include_str!("orchestrator/agent.toml"),
        prompt_fn: super::orchestrator::prompt::build,
    },
    BuiltinAgent {
        id: "planner",
        toml: include_str!("planner/agent.toml"),
        prompt_fn: super::planner::prompt::build,
    },
    BuiltinAgent {
        id: "code_executor",
        toml: include_str!("code_executor/agent.toml"),
        prompt_fn: super::code_executor::prompt::build,
    },
    BuiltinAgent {
        id: "integrations_agent",
        toml: include_str!("integrations_agent/agent.toml"),
        prompt_fn: super::integrations_agent::prompt::build,
    },
    BuiltinAgent {
        id: "crypto_agent",
        toml: include_str!("crypto_agent/agent.toml"),
        prompt_fn: super::crypto_agent::prompt::build,
    },
    BuiltinAgent {
        id: "markets_agent",
        toml: include_str!("markets_agent/agent.toml"),
        prompt_fn: super::markets_agent::prompt::build,
    },
    BuiltinAgent {
        id: "tinyplace_agent",
        toml: include_str!("../../tinyplace/agent/agent.toml"),
        prompt_fn: crate::openhuman::tinyplace::agent::prompt::build,
    },
    BuiltinAgent {
        id: "tools_agent",
        toml: include_str!("tools_agent/agent.toml"),
        prompt_fn: super::tools_agent::prompt::build,
    },
    BuiltinAgent {
        id: "task_manager_agent",
        toml: include_str!("task_manager_agent/agent.toml"),
        prompt_fn: super::task_manager_agent::prompt::build,
    },
    BuiltinAgent {
        id: "settings_agent",
        toml: include_str!("settings_agent/agent.toml"),
        prompt_fn: super::settings_agent::prompt::build,
    },
    BuiltinAgent {
        id: "profile_memory_agent",
        toml: include_str!("profile_memory_agent/agent.toml"),
        prompt_fn: super::profile_memory_agent::prompt::build,
    },
    BuiltinAgent {
        id: "account_admin_agent",
        toml: include_str!("account_admin_agent/agent.toml"),
        prompt_fn: super::account_admin_agent::prompt::build,
    },
    BuiltinAgent {
        id: "screen_awareness_agent",
        toml: include_str!("screen_awareness_agent/agent.toml"),
        prompt_fn: super::screen_awareness_agent::prompt::build,
    },
    BuiltinAgent {
        id: "scheduler_agent",
        toml: include_str!("scheduler_agent/agent.toml"),
        prompt_fn: super::scheduler_agent::prompt::build,
    },
    BuiltinAgent {
        id: "presentation_agent",
        toml: include_str!("presentation_agent/agent.toml"),
        prompt_fn: super::presentation_agent::prompt::build,
    },
    BuiltinAgent {
        id: "desktop_control_agent",
        toml: include_str!("desktop_control_agent/agent.toml"),
        prompt_fn: super::desktop_control_agent::prompt::build,
    },
    BuiltinAgent {
        id: "tool_maker",
        toml: include_str!("tool_maker/agent.toml"),
        prompt_fn: super::tool_maker::prompt::build,
    },
    BuiltinAgent {
        id: "skill_creator",
        toml: include_str!("skill_creator/agent.toml"),
        prompt_fn: super::skill_creator::prompt::build,
    },
    BuiltinAgent {
        id: "researcher",
        toml: include_str!("researcher/agent.toml"),
        prompt_fn: super::researcher::prompt::build,
    },
    BuiltinAgent {
        id: "critic",
        toml: include_str!("critic/agent.toml"),
        prompt_fn: super::critic::prompt::build,
    },
    BuiltinAgent {
        id: "vision_agent",
        toml: include_str!("vision_agent/agent.toml"),
        prompt_fn: super::vision_agent::prompt::build,
    },
    BuiltinAgent {
        id: "archivist",
        toml: include_str!("archivist/agent.toml"),
        prompt_fn: super::archivist::prompt::build,
    },
    BuiltinAgent {
        id: "trigger_triage",
        toml: include_str!("trigger_triage/agent.toml"),
        prompt_fn: super::trigger_triage::prompt::build,
    },
    BuiltinAgent {
        id: "trigger_reactor",
        toml: include_str!("trigger_reactor/agent.toml"),
        prompt_fn: super::trigger_reactor::prompt::build,
    },
    BuiltinAgent {
        id: "morning_briefing",
        toml: include_str!("morning_briefing/agent.toml"),
        prompt_fn: super::morning_briefing::prompt::build,
    },
    BuiltinAgent {
        id: "summarizer",
        toml: include_str!("summarizer/agent.toml"),
        prompt_fn: super::summarizer::prompt::build,
    },
    BuiltinAgent {
        id: "help",
        toml: include_str!("help/agent.toml"),
        prompt_fn: super::help::prompt::build,
    },
    BuiltinAgent {
        id: "mcp_setup",
        toml: include_str!("mcp_setup/agent.toml"),
        prompt_fn: super::mcp_setup::prompt::build,
    },
    BuiltinAgent {
        id: "mcp_agent",
        toml: include_str!("mcp_agent/agent.toml"),
        prompt_fn: super::mcp_agent::prompt::build,
    },
    BuiltinAgent {
        id: "skill_setup",
        toml: include_str!("../../skill_registry/agent/skill_setup/agent.toml"),
        prompt_fn: crate::openhuman::skill_registry::agent::skill_setup::prompt::build,
    },
    BuiltinAgent {
        id: "skill_executor",
        toml: include_str!("../../skill_runtime/agent/skill_executor/agent.toml"),
        prompt_fn: crate::openhuman::skill_runtime::agent::skill_executor::prompt::build,
    },
    BuiltinAgent {
        id: "agent_memory",
        toml: include_str!("../../agent_memory/agent/agent.toml"),
        prompt_fn: crate::openhuman::agent_memory::agent::prompt::build,
    },
    BuiltinAgent {
        id: "subconscious",
        toml: include_str!("../../subconscious/agent/agent.toml"),
        prompt_fn: crate::openhuman::subconscious::agent::prompt::build,
    },
];

/// Parse every entry in [`BUILTINS`] into an [`AgentDefinition`].
///
/// Errors out of the whole call on any parse failure — built-in TOML is
/// baked into the binary and therefore must always be valid. Unit tests
/// below keep that invariant honest.
pub fn load_builtins() -> Result<Vec<AgentDefinition>> {
    let defs: Vec<AgentDefinition> = BUILTINS.iter().map(parse_builtin).collect::<Result<_>>()?;
    validate_tier_hierarchy(&defs)
        .context("built-in agents violate the spawn-hierarchy contract")?;
    Ok(defs)
}

/// Validate the cross-agent spawn-hierarchy contract documented on
/// [`AgentTier`].
///
/// Rules enforced here:
///
/// * `Chat` agents MUST NOT list another `Chat` agent in `subagents`.
/// * `Reasoning` agents MUST NOT list another `Reasoning` agent in
///   `subagents`.
/// * `Worker` agents MUST NOT list any [`SubagentEntry::AgentId`]
///   entries. (Workflow wildcards are allowed: they expand to the generic
///   `integrations_agent`, which is itself a `Worker`, and the call
///   happens via a single delegation tool rather than recursive spawn.)
///
/// Workflow-wildcard entries (`{ skills = "*" }`) are intentionally
/// untouched: they collapse to one `delegate_to_integrations_agent`
/// tool whose target is a `Worker` and whose use sites are well
/// understood. Mis-tiering of the `integrations_agent` itself is still
/// caught because it appears as a normal entry elsewhere.
///
/// Called from [`load_builtins`] for the bundled archetype set and from
/// [`crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::load`]
/// after workspace-local TOML overrides are merged, so custom user
/// agents that violate the contract fail the boot rather than crashing
/// at spawn time.
pub fn validate_tier_hierarchy(defs: &[AgentDefinition]) -> Result<()> {
    let tier_by_id: HashMap<&str, AgentTier> =
        defs.iter().map(|d| (d.id.as_str(), d.agent_tier)).collect();

    for def in defs {
        for entry in &def.subagents {
            let child_id = match entry {
                SubagentEntry::AgentId(id) => id.as_str(),
                // Workflow wildcards always route to `integrations_agent`
                // (a Worker) via a single collapsed delegation tool —
                // not subject to the tier-mismatch rule.
                SubagentEntry::Skills(_) => continue,
            };

            // Worker leaves: no open-ended spawn surface.
            if def.agent_tier == AgentTier::Worker {
                anyhow::bail!(
                    "agent `{parent}` is a `worker` tier and must not list `{child}` in its \
                     subagents — workers are leaf executors.",
                    parent = def.id,
                    child = child_id,
                );
            }

            let Some(child_tier) = tier_by_id.get(child_id).copied() else {
                // Unknown id — that's a separate `subagents` integrity
                // concern (covered by existing tests / runtime spawn
                // resolution); don't mask it as a tier error.
                continue;
            };

            // Same-tier delegation is forbidden for chat and reasoning.
            // (Chat→Chat would defeat the whole point of the fast tier;
            // Reasoning→Reasoning produces a depth-blowing recursion of
            // slow models.)
            match (def.agent_tier, child_tier) {
                (AgentTier::Chat, AgentTier::Chat) => anyhow::bail!(
                    "agent `{parent}` (chat) lists `{child}` (chat) in subagents — the chat tier \
                     is a leaf in its own dimension. Hand off to a `reasoning` or `worker` agent \
                     instead.",
                    parent = def.id,
                    child = child_id,
                ),
                (AgentTier::Reasoning, AgentTier::Reasoning) => anyhow::bail!(
                    "agent `{parent}` (reasoning) lists `{child}` (reasoning) in subagents — \
                     reasoning agents compose downward into workers, not into each other.",
                    parent = def.id,
                    child = child_id,
                ),
                _ => {}
            }
        }
    }

    Ok(())
}

/// Parse a single [`BuiltinAgent`] triple into a finished [`AgentDefinition`].
fn parse_builtin(b: &BuiltinAgent) -> Result<AgentDefinition> {
    // The TOML ships without `system_prompt` — serde falls back to
    // `defaults::empty_inline_prompt` — and the loader injects the
    // rendered sibling `prompt.md` immediately below.
    let mut def: AgentDefinition = toml::from_str(b.toml)
        .with_context(|| format!("parsing built-in agent `{}` TOML", b.id))?;

    // Install the function-driven prompt builder and stamp the source.
    def.system_prompt = PromptSource::Dynamic(b.prompt_fn);
    def.source = DefinitionSource::Builtin;

    // Sanity check: file layout id must match declared TOML id. This
    // catches copy-paste mistakes where someone forgets to update the
    // `id` field after duplicating a folder.
    anyhow::ensure!(
        def.id == b.id,
        "built-in agent folder `{}` declares mismatched TOML id `{}`",
        b.id,
        def.id
    );

    Ok(def)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::definition::{
        ModelSpec, SandboxMode, SubagentEntry, ToolScope, TriggerMemoryAgent,
    };

    #[test]
    fn all_builtins_parse() {
        let defs = load_builtins().expect("built-in TOML must parse");
        assert_eq!(defs.len(), BUILTINS.len());
    }

    #[test]
    fn automatic_memory_agents_do_not_expose_call_memory_agent() {
        for def in load_builtins().expect("built-in TOML must parse") {
            if def.trigger_memory_agent != TriggerMemoryAgent::Always {
                continue;
            }

            let exposes_call_memory_agent = match &def.tools {
                ToolScope::Named(tools) => tools.iter().any(|tool| tool == "call_memory_agent"),
                ToolScope::Wildcard => false,
            };

            assert!(
                !exposes_call_memory_agent,
                "{} uses trigger_memory_agent but still exposes call_memory_agent",
                def.id
            );
            assert!(
                !def.subagents.iter().any(
                    |entry| matches!(entry, SubagentEntry::AgentId(id) if id == "agent_memory")
                ),
                "{} uses trigger_memory_agent but still lists agent_memory in subagents",
                def.id
            );
        }
    }

    #[test]
    fn trigger_reactor_has_agentic_hint_and_narrow_tools() {
        let def = find("trigger_reactor");
        assert!(matches!(def.model, ModelSpec::Hint(ref h) if h == "agentic"));
        match &def.tools {
            ToolScope::Named(tools) => {
                assert!(!tools.iter().any(|t| t == "call_memory_agent"));
                assert!(
                    tools.iter().any(|t| t == "memory_store"),
                    "trigger_reactor needs memory_store"
                );
                assert!(
                    tools.iter().any(|t| t == "spawn_subagent"),
                    "trigger_reactor needs spawn_subagent for escalation"
                );
                // No shell / file_write — reactor does not execute code.
                assert!(!tools.iter().any(|t| t == "shell"));
                assert!(!tools.iter().any(|t| t == "file_write"));
            }
            ToolScope::Wildcard => panic!("trigger_reactor must have a Named tool scope"),
        }
        assert_eq!(def.sandbox_mode, SandboxMode::None);
        assert_eq!(def.max_iterations, 6);
        assert!(
            !def.omit_memory_context,
            "trigger_reactor needs global memory/context"
        );
    }

    #[test]
    fn trigger_triage_has_no_tools_and_pulls_memory_context() {
        let def = find("trigger_triage");
        match &def.tools {
            ToolScope::Named(tools) => assert!(
                tools.is_empty(),
                "trigger_triage must have zero tools (got {tools:?})"
            ),
            ToolScope::Wildcard => panic!("trigger_triage must have a Named empty tool scope"),
        }
        assert!(
            !def.omit_memory_context,
            "trigger_triage needs global memory/context to reason about triggers"
        );
        assert!(def.omit_identity);
        assert!(def.omit_safety_preamble);
        assert!(def.omit_skills_catalog);
        assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
        assert_eq!(def.max_iterations, 2);
    }

    #[test]
    fn folder_ids_match_toml_ids() {
        for b in BUILTINS {
            let def = parse_builtin(b).expect("parse");
            assert_eq!(def.id, b.id, "folder `{}` id mismatch", b.id);
        }
    }

    /// Regression guard for #3236.
    ///
    /// PR #3074 introduced the `Config.action_dir` / `Config.workspace_dir`
    /// split: acting tools resolve to `action_dir` (default
    /// `~/OpenHuman/projects`), and `workspace_dir` is reserved for
    /// internal product state (memory / sessions / vault / etc.) that is
    /// denied to agent tools. The coding-agent prompts must reflect that
    /// split — saying "in a sandboxed environment" or "the workspace has
    /// code …" without anchoring contradicts the new model and steers
    /// the model toward paths that hit the internal-state denylist.
    ///
    /// If a future edit reintroduces stale phrasing, this assertion fires
    /// at `cargo test` time before the bad prompt ships.
    #[test]
    fn coding_agent_prompts_reference_action_sandbox_not_stale_workspace() {
        let code_executor = include_str!("code_executor/prompt.md");
        assert!(
            !code_executor.contains("sandboxed environment"),
            "code_executor/prompt.md still says 'sandboxed environment' \
             generically — anchor in the action sandbox path (see #3236)"
        );
        assert!(
            code_executor.contains("action sandbox") || code_executor.contains("action_dir"),
            "code_executor/prompt.md must reference the action sandbox or action_dir (see #3236)"
        );

        let planner = include_str!("planner/prompt.md");
        assert!(
            !planner.contains("the workspace has code"),
            "planner/prompt.md still says 'the workspace has code …' — \
             use 'the project tree' or similar to avoid colliding with \
             `Config.workspace_dir` (internal product state). See #3236."
        );
    }

    #[test]
    fn every_builtin_has_a_prompt_body() {
        use crate::openhuman::context::prompt::{
            ConnectedIntegration, LearnedContextData, PromptContext, PromptTool, ToolCallFormat,
        };
        let empty_tools: Vec<PromptTool<'_>> = Vec::new();
        let empty_integrations: Vec<ConnectedIntegration> = Vec::new();
        let empty_visible: std::collections::HashSet<String> = std::collections::HashSet::new();
        for def in load_builtins().unwrap() {
            match &def.system_prompt {
                PromptSource::Dynamic(build) => {
                    let ctx = PromptContext {
                        workspace_dir: std::path::Path::new("."),
                        model_name: "test",
                        agent_id: &def.id,
                        tools: &empty_tools,
                        workflows: &[],
                        dispatcher_instructions: "",
                        learned: LearnedContextData::default(),
                        visible_tool_names: &empty_visible,
                        tool_call_format: ToolCallFormat::PFormat,
                        connected_integrations: &empty_integrations,
                        connected_identities_md: String::new(),
                        include_profile: false,
                        include_memory_md: false,
                        curated_snapshot: None,
                        user_identity: None,
                        personality_soul_md: None,
                        personality_memory_md: None,
                        personality_roster: vec![],
                    };
                    let body = build(&ctx)
                        .unwrap_or_else(|e| panic!("{} prompt build failed: {e}", def.id));
                    assert!(!body.is_empty(), "{} has empty prompt", def.id);
                }
                PromptSource::Inline(_) | PromptSource::File { .. } => {
                    panic!("{} should use dynamic prompt builder", def.id);
                }
            }
        }
    }

    #[test]
    fn every_builtin_is_stamped_builtin_source() {
        for def in load_builtins().unwrap() {
            assert_eq!(def.source, DefinitionSource::Builtin);
        }
    }

    fn find(id: &str) -> AgentDefinition {
        load_builtins()
            .unwrap()
            .into_iter()
            .find(|d| d.id == id)
            .unwrap_or_else(|| panic!("missing built-in {id}"))
    }

    #[test]
    fn vision_agent_loads_on_vision_hint() {
        // The vision sub-agent rides the multimodal `vision-v1` tier (via the
        // `vision` hint) so its model is image-capable, and it must be reachable
        // from the orchestrator's subagent allowlist.
        let def = find("vision_agent");
        assert!(matches!(def.model, ModelSpec::Hint(ref h) if h == "vision"));

        let orchestrator = find("orchestrator");
        assert!(
            orchestrator
                .subagents
                .iter()
                .any(|s| matches!(s, SubagentEntry::AgentId(id) if id == "vision_agent")),
            "orchestrator must list vision_agent in its subagents allowlist"
        );
    }

    #[test]
    fn orchestrator_has_chat_hint_and_named_tools() {
        let def = find("orchestrator");
        assert!(matches!(def.model, ModelSpec::Hint(ref h) if h == "chat"));
        match def.tools {
            ToolScope::Named(tools) => {
                // spawn_subagent was removed in #1141; spawn_worker_thread is the replacement
                assert!(
                    tools.iter().any(|t| t == "spawn_worker_thread"),
                    "orchestrator must have spawn_worker_thread"
                );
                assert!(
                    tools.iter().any(|t| t == "spawn_async_subagent"),
                    "orchestrator must have spawn_async_subagent for sparse background work"
                );
                assert!(
                    !tools.iter().any(|t| t == "spawn_subagent"),
                    "spawn_subagent must not appear — removed in #1141"
                );
                assert!(!tools.iter().any(|t| t == "call_memory_agent"));
                assert!(!tools.iter().any(|t| t == "shell"));
                assert!(!tools.iter().any(|t| t == "file_write"));
            }
            ToolScope::Wildcard => panic!("orchestrator must have named tool allowlist"),
        }
        assert_eq!(def.max_iterations, 15);
        // Memory retrieval is on-demand (via the `agent_memory` subagent,
        // surfaced as `delegate_retrieve_memory`), not an eager pre-turn
        // pre-fetch. The allowlist entry is what makes that route reachable
        // (see the `agent_memory::tools` allowlist gate).
        assert_eq!(def.trigger_memory_agent, TriggerMemoryAgent::Never);
        assert!(
            def.subagents.iter().any(|entry| matches!(
                entry,
                SubagentEntry::AgentId(id) if id == "agent_memory"
            )),
            "orchestrator must allow `agent_memory` for on-demand retrieval"
        );
    }

    /// Regression guard for the `resolve_time` wiring. Agents that emit
    /// timestamp arguments to downstream tools must keep the deterministic
    /// time resolver in their allowlist — otherwise the model falls back to
    /// hand-computing epoch seconds, which once produced a ~10-month-wrong
    /// `oldest` and silently fetched the wrong Slack window. If any of these
    /// drops `resolve_time`, this test fails loudly.
    #[test]
    fn time_sensitive_agents_expose_resolve_time() {
        for id in [
            "orchestrator",
            "integrations_agent",
            "scheduler_agent",
            "task_manager_agent",
            "crypto_agent",
            "markets_agent",
            "tinyplace_agent",
        ] {
            let def = find(id);
            match def.tools {
                ToolScope::Named(tools) => assert!(
                    tools.iter().any(|t| t == "resolve_time"),
                    "{id} must keep `resolve_time` in its named tool allowlist"
                ),
                ToolScope::Wildcard => {
                    // Wildcard agents inherit the full built-in surface, which
                    // already includes resolve_time — nothing to assert here.
                }
            }
        }
    }

    #[test]
    fn code_executor_is_sandboxed_and_keeps_safety_preamble() {
        let def = find("code_executor");
        assert_eq!(def.sandbox_mode, SandboxMode::Sandboxed);
        assert!(!def.omit_safety_preamble);
        assert_eq!(def.max_iterations, 10);
    }

    #[test]
    fn tool_maker_is_sandboxed_with_max_2_iterations() {
        let def = find("tool_maker");
        assert_eq!(def.sandbox_mode, SandboxMode::Sandboxed);
        assert_eq!(def.max_iterations, 2);
        assert!(!def.omit_safety_preamble);
    }

    #[test]
    fn skill_creator_is_sandboxed_and_has_node_tools() {
        let def = find("skill_creator");
        assert_eq!(def.sandbox_mode, SandboxMode::Sandboxed);
        assert_eq!(def.max_iterations, 10);
        assert!(!def.omit_safety_preamble);
        match &def.tools {
            ToolScope::Named(names) => {
                for required in ["node_exec", "npm_exec", "apply_patch", "update_memory_md"] {
                    assert!(
                        names.iter().any(|name| name == required),
                        "skill_creator tool list missing `{required}`"
                    );
                }
            }
            ToolScope::Wildcard => panic!("skill_creator must have named tool allowlist"),
        }
    }

    #[test]
    fn critic_is_read_only() {
        let def = find("critic");
        assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
        assert!(def.omit_safety_preamble);
    }

    /// Planner runs `composio_execute` so it can ground plans in real
    /// integration data, but it must stay strictly read-only — issue
    /// #685. `sandbox_mode = "read_only"` in `planner/agent.toml` is the
    /// runtime hook that activates the agent-level gate inside
    /// `ComposioExecuteTool::execute`; this test pins that contract so a
    /// future TOML edit that drops the sandbox mode can never silently
    /// turn the planner into a write-capable agent.
    #[test]
    fn planner_is_read_only_with_composio_meta_tools() {
        let def = find("planner");
        assert_eq!(
            def.sandbox_mode,
            SandboxMode::ReadOnly,
            "planner.sandbox_mode must be read_only — gates Write/Admin composio actions",
        );
        match &def.tools {
            ToolScope::Named(names) => {
                for required in [
                    "composio_list_toolkits",
                    "composio_list_connections",
                    "composio_list_tools",
                    "composio_execute",
                ] {
                    assert!(
                        names.iter().any(|n| n == required),
                        "planner tool list missing `{required}` — composio meta-tools must \
                         all be present so the planner can inspect integrations under the \
                         read-only sandbox gate",
                    );
                }
            }
            other => panic!("planner must use Named tool scope, got {other:?}"),
        }
    }

    /// The planner grounds plans in connected-MCP context the same way it
    /// grounds in Composio — but read-only. It must carry the MCP *discovery*
    /// tools (`status` / `installed_list` / `list_tools`, all
    /// `PermissionLevel::ReadOnly`) and must NOT carry `mcp_registry_tool_call`
    /// (no read-only gate exists for an arbitrary MCP tool call) nor the
    /// install/connect mutators. Execution stays with `mcp_agent`.
    #[test]
    fn planner_has_readonly_mcp_discovery_not_execute() {
        let def = find("planner");
        assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
        match &def.tools {
            ToolScope::Named(names) => {
                for required in [
                    "mcp_registry_status",
                    "mcp_registry_installed_list",
                    "mcp_registry_list_tools",
                ] {
                    assert!(
                        names.iter().any(|n| n == required),
                        "planner needs read-only MCP discovery tool `{required}`"
                    );
                }
                for forbidden in [
                    "mcp_registry_tool_call",
                    "mcp_registry_connect",
                    "mcp_registry_install",
                    "mcp_registry_uninstall",
                ] {
                    assert!(
                        !names.iter().any(|n| n == forbidden),
                        "planner must NOT have `{forbidden}` — it is read-only; MCP execution \
                         belongs to mcp_agent"
                    );
                }
            }
            other => panic!("planner must use Named tool scope, got {other:?}"),
        }
    }

    #[test]
    fn integrations_agent_tool_scope_honours_toml() {
        let def = find("integrations_agent");
        // Current TOML: `named = ["composio_list_tools", "file_read"]`.
        // Sub-agent runner additionally injects per-toolkit
        // ComposioActionTools at spawn time.
        match &def.tools {
            ToolScope::Named(names) => {
                assert!(names.iter().any(|n| n == "composio_list_tools"));
            }
            other => panic!("expected Named scope, got {other:?}"),
        }
        assert!(!def.omit_safety_preamble);
    }

    #[test]
    fn tools_agent_is_registered() {
        let def = find("tools_agent");
        assert!(matches!(def.tools, ToolScope::Wildcard));
    }

    #[test]
    fn tinyplace_agent_is_registered_and_narrow() {
        let def = find("tinyplace_agent");
        assert!(matches!(def.model, ModelSpec::Hint(ref h) if h == "agentic"));
        assert_eq!(def.sandbox_mode, SandboxMode::None);
        assert!(!def.omit_safety_preamble);
        assert_eq!(def.delegate_name.as_deref(), Some("use_tinyplace"));
        match &def.tools {
            ToolScope::Named(names) => {
                // Curated flow surface (replaced the per-controller 1:1 tools).
                for required in [
                    "tinyplace_whoami",
                    "tinyplace_status",
                    "tinyplace_feed",
                    "tinyplace_find_work",
                    "tinyplace_register",
                    "tinyplace_post_bounty",
                    "tinyplace_submit_work",
                    "tinyplace_job_apply",
                    "tinyplace_graphql",
                    "tinyplace_call",
                    "tinyplace_help",
                    "ask_user_clarification",
                    "resolve_time",
                    "current_time",
                ] {
                    assert!(
                        names.iter().any(|name| name == required),
                        "tinyplace_agent tool list missing `{required}`"
                    );
                }
                for forbidden in [
                    "shell",
                    "file_write",
                    "composio_execute",
                    "mcp_registry_tool_call",
                ] {
                    assert!(
                        !names.iter().any(|name| name == forbidden),
                        "tinyplace_agent must not expose broad tool `{forbidden}`"
                    );
                }
            }
            other => panic!("tinyplace_agent must use Named tool scope, got {other:?}"),
        }

        let orchestrator = find("orchestrator");
        assert!(
            orchestrator.subagents.iter().any(
                |entry| matches!(entry, SubagentEntry::AgentId(id) if id == "tinyplace_agent")
            ),
            "orchestrator must allow `tinyplace_agent` so use_tinyplace can spawn it"
        );
    }

    #[test]
    fn specialist_agents_are_registered_with_narrow_tools() {
        let scheduler = find("scheduler_agent");
        match &scheduler.tools {
            ToolScope::Named(names) => {
                for required in ["current_time", "cron_add", "cron_list", "cron_remove"] {
                    assert!(
                        names.iter().any(|name| name == required),
                        "scheduler_agent missing `{required}`"
                    );
                }
            }
            other => panic!("scheduler_agent must use Named tool scope, got {other:?}"),
        }

        let presentation = find("presentation_agent");
        match &presentation.tools {
            ToolScope::Named(names) => {
                assert!(names.iter().any(|name| name == "generate_presentation"));
                assert!(!names.iter().any(|name| name == "call_memory_agent"));
                assert!(names.iter().any(|name| name == "web_search_tool"));
            }
            other => panic!("presentation_agent must use Named tool scope, got {other:?}"),
        }
        // Memory pre-fetch is no longer eager; `omit_memory_context = false`
        // still gives the deck builder the cheap per-turn recall.
        assert_eq!(presentation.trigger_memory_agent, TriggerMemoryAgent::Never);

        let desktop = find("desktop_control_agent");
        match &desktop.tools {
            ToolScope::Named(names) => {
                for required in [
                    "launch_app",
                    "ax_interact",
                    "automate",
                    "screenshot",
                    "mouse",
                    "keyboard",
                ] {
                    assert!(names.iter().any(|name| name == required));
                }
            }
            other => panic!("desktop_control_agent must use Named tool scope, got {other:?}"),
        }
    }

    #[test]
    fn archivist_runs_in_background() {
        let def = find("archivist");
        assert!(def.background);
        assert_eq!(def.max_iterations, 3);
    }

    #[test]
    fn morning_briefing_is_read_only() {
        let def = find("morning_briefing");
        assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
        assert!(matches!(def.tools, ToolScope::Wildcard));
        // The brief pulls its own last-24h memory via the `memory_tree`
        // `cover_window` tool, so the stale all-time memory blob is suppressed.
        assert!(def.omit_memory_context);
        assert!(def.omit_identity);
        assert!(def.omit_safety_preamble);
        assert_eq!(def.max_iterations, 8);
        assert!(
            def.disallowed_tools.iter().any(|t| t == "tinyplace_*"),
            "morning_briefing.disallowed_tools must contain `tinyplace_*` so \
             tiny.place routes through tinyplace_agent exclusively"
        );
    }

    #[test]
    fn help_uses_gitbooks_tools_and_is_read_only() {
        let def = find("help");
        assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
        match &def.tools {
            ToolScope::Named(tools) => {
                assert!(
                    tools.iter().any(|t| t == "gitbooks_search"),
                    "help needs gitbooks_search"
                );
                assert!(
                    tools.iter().any(|t| t == "gitbooks_get_page"),
                    "help needs gitbooks_get_page"
                );
                assert!(!tools.iter().any(|t| t == "call_memory_agent"));
                // Help is docs-only — no write/exec tools.
                assert!(!tools.iter().any(|t| t == "shell"));
                assert!(!tools.iter().any(|t| t == "file_write"));
                assert!(!tools.iter().any(|t| t == "curl"));
                assert!(!tools.iter().any(|t| t == "spawn_subagent"));
            }
            ToolScope::Wildcard => panic!("help must have a Named tool scope"),
        }
        assert!(def.omit_identity);
        assert!(def.omit_safety_preamble);
        assert!(!def.omit_memory_context);
        // Help personalises from the cheap per-turn recall (memory_context on),
        // so it no longer pre-fetches the full memory agent before every turn.
        assert_eq!(def.trigger_memory_agent, TriggerMemoryAgent::Never);
    }

    #[test]
    fn researcher_has_curl_for_artifact_downloads() {
        let def = find("researcher");
        match &def.tools {
            ToolScope::Named(tools) => {
                assert!(
                    tools.iter().any(|t| t == "curl"),
                    "researcher needs curl for artifact downloads"
                );
                assert!(
                    tools.iter().any(|t| t == "http_request"),
                    "researcher still needs http_request"
                );
            }
            ToolScope::Wildcard => panic!("researcher must have Named tool scope"),
        }
    }

    #[test]
    fn code_executor_has_curl_for_artifact_downloads() {
        let def = find("code_executor");
        match &def.tools {
            ToolScope::Named(tools) => {
                assert!(
                    tools.iter().any(|t| t == "curl"),
                    "code_executor needs curl for artifact/dataset fetches"
                );
            }
            ToolScope::Wildcard => panic!("code_executor must have Named tool scope"),
        }
    }

    #[test]
    fn orchestrator_does_not_get_curl() {
        // Per design: curl is a `Write` permission tool that writes
        // to the workspace. The orchestrator delegates rather than
        // executing — code_executor / researcher own actual downloads.
        let def = find("orchestrator");
        if let ToolScope::Named(tools) = &def.tools {
            assert!(
                !tools.iter().any(|t| t == "curl"),
                "orchestrator must not have curl — it should delegate"
            );
        }
    }

    /// Crypto Agent (#1397) is the dedicated specialist for wallet
    /// actions and market operations. It must have a *narrow* tool
    /// allowlist (no shell, no file_write, no broad HTTP), MUST keep
    /// the safety preamble on (financial-risk gate), and MUST require
    /// quote/confirm-before-execute via `ask_user_clarification`.
    #[test]
    fn crypto_agent_has_narrow_wallet_market_tools_and_safety_on() {
        let def = find("crypto_agent");
        // Hint must be agentic — the agent reasons about quotes vs.
        // executes across multiple tool calls per turn.
        assert!(matches!(def.model, ModelSpec::Hint(ref h) if h == "agentic"));
        assert_eq!(def.sandbox_mode, SandboxMode::None);
        // Financial-risk agent — global safety preamble stays ON.
        assert!(
            !def.omit_safety_preamble,
            "crypto_agent must keep the global safety preamble — financial-risk gate"
        );
        match &def.tools {
            ToolScope::Named(tools) => {
                // Wallet read surface.
                for required in [
                    "wallet_status",
                    "wallet_balances",
                    "wallet_network_defaults",
                    "wallet_supported_assets",
                    "wallet_chain_status",
                    "wallet_encode_erc20_transfer",
                ] {
                    assert!(
                        tools.iter().any(|t| t == required),
                        "crypto_agent needs read tool `{required}`"
                    );
                }
                // Quote / prepare surface: native+token transfers on the
                // wallet, swaps/bridges/dapp calls on the web3 layer.
                for required in [
                    "wallet_prepare_transfer",
                    "web3_swap_quote",
                    "web3_bridge_quote",
                    "web3_dapp_call",
                ] {
                    assert!(
                        tools.iter().any(|t| t == required),
                        "crypto_agent needs prepare tool `{required}`"
                    );
                }
                // Transaction inspection surface.
                for required in ["wallet_tx_status", "wallet_tx_receipt", "wallet_lookup_tx"] {
                    assert!(
                        tools.iter().any(|t| t == required),
                        "crypto_agent needs tx-read tool `{required}`"
                    );
                }
                // Execute surface — gated by the prepared blob from a
                // matching prepare_* call in the same turn.
                assert!(
                    tools.iter().any(|t| t == "wallet_execute_prepared"),
                    "crypto_agent needs wallet_execute_prepared"
                );
                // Confirmation gate — MUST be present so the prompt's
                // "confirm before execute" rule is mechanically enforceable.
                assert!(
                    tools.iter().any(|t| t == "ask_user_clarification"),
                    "crypto_agent needs ask_user_clarification to gate write ops"
                );
                // Market grounding + time helpers. Memory retrieval is the
                // orchestrator's on-demand concern — this specialist gets a
                // grounded request and does not pre-fetch memory itself.
                for required in [
                    "stock_quote",
                    "stock_exchange_rate",
                    "stock_crypto_series",
                    "current_time",
                ] {
                    assert!(
                        tools.iter().any(|t| t == required),
                        "crypto_agent needs supporting tool `{required}`"
                    );
                }
                // x402 paid HTTP requests — signs on-chain USDC payments
                // for APIs behind HTTP 402 challenges.
                assert!(
                    tools.iter().any(|t| t == "x402_request"),
                    "crypto_agent needs x402_request for paid API access"
                );
                assert!(!tools.iter().any(|t| t == "call_memory_agent"));
                // Hard exclusions — no broad-surface or write-anywhere tools.
                // Includes the orchestrator-level delegate_* tools so a future
                // TOML edit can't accidentally hand crypto writes to the
                // generic integrations or code-execution paths.
                for forbidden in [
                    "shell",
                    "file_write",
                    "curl",
                    "http_request",
                    "composio_execute",
                    "composio_list_tools",
                    "spawn_subagent",
                    "spawn_worker_thread",
                    "delegate_to_integrations_agent",
                    "delegate_run_code",
                    "delegate_research",
                    "delegate_plan",
                ] {
                    assert!(
                        !tools.iter().any(|t| t == forbidden),
                        "crypto_agent must NOT have `{forbidden}` — keeps blast radius bounded"
                    );
                }
            }
            ToolScope::Wildcard => panic!("crypto_agent must have a Named tool scope"),
        }
        // Keep iteration cap tight — quote → confirm → execute is a
        // 3-step loop, not a research crawl.
        assert!(
            def.max_iterations <= 10,
            "crypto_agent max_iterations must stay tight (got {})",
            def.max_iterations
        );
        assert!(def.omit_identity);
        assert!(def.omit_memory_context);
        assert!(def.omit_skills_catalog);
        // Pure-function specialist (omit_memory_context = true) — no eager
        // memory pre-fetch; the orchestrator hands it a grounded request.
        assert_eq!(def.trigger_memory_agent, TriggerMemoryAgent::Never);
    }

    /// Routing: the orchestrator must list `crypto_agent` in its
    /// `subagents` so a `delegate_do_crypto` tool is synthesised at
    /// agent-build time. Without this entry the orchestrator can't
    /// route crypto-shaped requests to the specialist.
    #[test]
    fn orchestrator_subagents_include_crypto_agent() {
        use crate::openhuman::agent::harness::definition::SubagentEntry;
        let def = find("orchestrator");
        let listed = def.subagents.iter().any(|e| match e {
            SubagentEntry::AgentId(id) => id == "crypto_agent",
            _ => false,
        });
        assert!(
            listed,
            "orchestrator.subagents must list `crypto_agent` so the \
             routing layer can synthesise `delegate_do_crypto`"
        );
    }

    #[test]
    fn markets_agent_has_narrow_prediction_market_tools_and_safety_on() {
        let def = find("markets_agent");
        // Hint must be agentic — the agent reasons about market shape vs.
        // executes across multiple tool calls per turn.
        assert!(matches!(def.model, ModelSpec::Hint(ref h) if h == "agentic"));
        assert_eq!(def.sandbox_mode, SandboxMode::None);
        // Financial-side-effect agent — global safety preamble stays ON.
        assert!(
            !def.omit_safety_preamble,
            "markets_agent must keep the global safety preamble — financial-risk gate"
        );
        match &def.tools {
            ToolScope::Named(tools) => {
                // Prediction-market venues.
                for required in ["polymarket", "kalshi"] {
                    assert!(
                        tools.iter().any(|t| t == required),
                        "markets_agent needs venue tool `{required}`"
                    );
                }
                // Confirmation gate — MUST be present so the prompt's
                // "confirm before execute" rule is mechanically enforceable.
                assert!(
                    tools.iter().any(|t| t == "ask_user_clarification"),
                    "markets_agent needs ask_user_clarification to gate write ops"
                );
                // Time grounding stays as a tool; memory retrieval is the
                // orchestrator's on-demand concern — this specialist gets a
                // grounded request and does not pre-fetch memory itself.
                for required in ["current_time"] {
                    assert!(
                        tools.iter().any(|t| t == required),
                        "markets_agent needs supporting tool `{required}`"
                    );
                }
                assert!(!tools.iter().any(|t| t == "call_memory_agent"));
                // Hard exclusions — no broad-surface tools, no wallet
                // primitives (those belong to crypto_agent), no
                // delegation tools (markets_agent is a worker leaf).
                for forbidden in [
                    "shell",
                    "file_write",
                    "curl",
                    "http_request",
                    "composio_execute",
                    "composio_list_tools",
                    "spawn_subagent",
                    "spawn_worker_thread",
                    "delegate_to_integrations_agent",
                    "delegate_run_code",
                    "delegate_research",
                    "delegate_plan",
                    "wallet_execute_prepared",
                    "wallet_prepare_transfer",
                    "web3_swap_execute",
                    "web3_bridge_execute",
                    "web3_dapp_execute",
                ] {
                    assert!(
                        !tools.iter().any(|t| t == forbidden),
                        "markets_agent must NOT have `{forbidden}` — keeps blast radius bounded"
                    );
                }
            }
            ToolScope::Wildcard => panic!("markets_agent must have a Named tool scope"),
        }
        // Keep iteration cap tight — browse → propose → confirm → execute
        // is a short loop, not a research crawl.
        assert!(
            def.max_iterations <= 10,
            "markets_agent max_iterations must stay tight (got {})",
            def.max_iterations
        );
        assert!(def.omit_identity);
        assert!(def.omit_memory_context);
        assert!(def.omit_skills_catalog);
        // Pure-function specialist (omit_memory_context = true) — no eager
        // memory pre-fetch; the orchestrator hands it a grounded request.
        assert_eq!(def.trigger_memory_agent, TriggerMemoryAgent::Never);
        // Delegate name must be the stable, chat-friendly slug — the
        // orchestrator surfaces it as `delegate_do_prediction_markets`.
        assert_eq!(
            def.delegate_name.as_deref(),
            Some("do_prediction_markets"),
            "markets_agent must keep its `do_prediction_markets` delegate name stable"
        );
    }

    /// Routing: the orchestrator must list `markets_agent` in its
    /// `subagents` so a `delegate_do_prediction_markets` tool is
    /// synthesised at agent-build time. Without this entry the
    /// orchestrator can't route Polymarket / Kalshi requests to the
    /// specialist and they fall back into the generalist tools_agent
    /// wildcard.
    #[test]
    fn orchestrator_subagents_include_markets_agent() {
        use crate::openhuman::agent::harness::definition::SubagentEntry;
        let def = find("orchestrator");
        let listed = def.subagents.iter().any(|e| match e {
            SubagentEntry::AgentId(id) => id == "markets_agent",
            _ => false,
        });
        assert!(
            listed,
            "orchestrator.subagents must list `markets_agent` so the \
             routing layer can synthesise `delegate_do_prediction_markets`"
        );
    }

    /// `tools_agent` must explicitly disallow specialist-owned external action
    /// families so the wildcard inventory does not surface raw paid/write
    /// tools to the generalist, bypassing specialist prompts.
    #[test]
    fn tools_agent_disallows_specialist_owned_external_tools() {
        let def = find("tools_agent");
        assert!(
            def.disallowed_tools.iter().any(|t| t == "polymarket"),
            "tools_agent.disallowed_tools must contain `polymarket` so the \
             venue routes through markets_agent exclusively"
        );
        assert!(
            def.disallowed_tools.iter().any(|t| t == "kalshi"),
            "tools_agent.disallowed_tools must contain `kalshi` so the \
             venue routes through markets_agent exclusively"
        );
        assert!(
            def.disallowed_tools.iter().any(|t| t == "tinyplace_*"),
            "tools_agent.disallowed_tools must contain `tinyplace_*` so \
             tiny.place routes through tinyplace_agent exclusively"
        );
    }

    /// Routing: the orchestrator must list `mcp_agent` in its `subagents`
    /// so a `delegate_use_mcp_server` tool is synthesised at agent-build
    /// time. Without this entry the orchestrator can only *set up* MCP
    /// servers (via `mcp_setup`) and has no route to actually *use* an
    /// already-connected server's tools from chat (issue #3495).
    #[test]
    fn orchestrator_subagents_include_mcp_agent() {
        use crate::openhuman::agent::harness::definition::SubagentEntry;
        let def = find("orchestrator");
        let listed = def.subagents.iter().any(|e| match e {
            SubagentEntry::AgentId(id) => id == "mcp_agent",
            _ => false,
        });
        assert!(
            listed,
            "orchestrator.subagents must list `mcp_agent` so the routing \
             layer can synthesise `delegate_use_mcp_server`"
        );
    }

    /// The orchestrator gets lightweight MCP discovery (`mcp_registry_status`,
    /// like `composio_list_connections`) but must NOT carry the per-server
    /// enumerate/execute tools — those belong to `mcp_agent`, keeping the
    /// chat agent's schema from ballooning with every connected server's
    /// full toolset (#3495).
    #[test]
    fn orchestrator_has_mcp_discovery_but_not_execution() {
        let def = find("orchestrator");
        match &def.tools {
            ToolScope::Named(tools) => {
                assert!(
                    tools.iter().any(|t| t == "mcp_registry_status"),
                    "orchestrator must have mcp_registry_status for lightweight MCP discovery"
                );
                for forbidden in ["mcp_registry_list_tools", "mcp_registry_tool_call"] {
                    assert!(
                        !tools.iter().any(|t| t == forbidden),
                        "orchestrator must NOT have `{forbidden}` — enumerating/calling \
                         connected MCP tools is mcp_agent's job (keeps the chat schema small)"
                    );
                }
            }
            ToolScope::Wildcard => panic!("orchestrator must have a Named tool scope"),
        }
    }

    /// `mcp_agent` is the connected-server execution specialist: it must hold
    /// the discover + call surface and a stable `use_mcp_server` delegate name,
    /// but must NOT hold the secret-handling install/uninstall tools (those are
    /// `mcp_setup`'s) or any shell/file/network capability.
    #[test]
    fn mcp_agent_drives_connected_servers_without_install_or_shell() {
        let def = find("mcp_agent");
        assert_eq!(def.agent_tier, AgentTier::Worker);
        assert_eq!(
            def.delegate_name.as_deref(),
            Some("use_mcp_server"),
            "mcp_agent must keep its `use_mcp_server` delegate name stable"
        );
        match &def.tools {
            ToolScope::Named(tools) => {
                for required in [
                    "mcp_registry_status",
                    "mcp_registry_list_tools",
                    "mcp_registry_connect",
                    "mcp_registry_tool_call",
                ] {
                    assert!(
                        tools.iter().any(|t| t == required),
                        "mcp_agent missing `{required}`"
                    );
                }
                for forbidden in [
                    "mcp_registry_install",
                    "mcp_registry_uninstall",
                    "shell",
                    "file_write",
                    "curl",
                    "http_request",
                ] {
                    assert!(
                        !tools.iter().any(|t| t == forbidden),
                        "mcp_agent must NOT have `{forbidden}` — it only relays through \
                         already-connected servers; install/secrets belong to mcp_setup"
                    );
                }
            }
            ToolScope::Wildcard => panic!("mcp_agent must have a Named tool scope"),
        }
    }

    #[test]
    fn orchestrator_subagents_include_skill_creator() {
        use crate::openhuman::agent::harness::definition::SubagentEntry;
        let def = find("orchestrator");
        let listed = def.subagents.iter().any(|e| match e {
            SubagentEntry::AgentId(id) => id == "skill_creator",
            _ => false,
        });
        assert!(
            listed,
            "orchestrator.subagents must list `skill_creator` so the \
            routing layer can synthesise `create_skill`"
        );
    }

    #[test]
    fn orchestrator_subagents_include_control_specialists() {
        use crate::openhuman::agent::harness::definition::SubagentEntry;
        let def = find("orchestrator");
        let subagents: std::collections::HashSet<&str> = def
            .subagents
            .iter()
            .filter_map(|entry| match entry {
                SubagentEntry::AgentId(id) => Some(id.as_str()),
                SubagentEntry::Skills(_) => None,
            })
            .collect();

        for expected in [
            "task_manager_agent",
            "settings_agent",
            "profile_memory_agent",
            "account_admin_agent",
            "screen_awareness_agent",
        ] {
            assert!(
                subagents.contains(expected),
                "orchestrator.subagents must list `{expected}` so the routing layer can synthesize its delegate tool"
            );
        }
    }

    #[test]
    fn control_specialists_have_named_tools_and_are_worker_leaves() {
        use crate::openhuman::agent::harness::definition::SubagentEntry;

        for expected in [
            "task_manager_agent",
            "settings_agent",
            "profile_memory_agent",
            "account_admin_agent",
            "screen_awareness_agent",
        ] {
            let def = find(expected);
            assert_eq!(def.agent_tier, AgentTier::Worker);
            let visible_subagents: Vec<&str> = def
                .subagents
                .iter()
                .filter_map(|entry| match entry {
                    SubagentEntry::AgentId(id) => Some(id.as_str()),
                    _ => None,
                })
                .collect();
            assert!(
                visible_subagents.is_empty(),
                "{expected} must be a worker leaf"
            );
            match def.tools {
                ToolScope::Named(tools) => {
                    assert!(
                        !tools.is_empty(),
                        "{expected} must have a concrete tool allowlist"
                    );
                    assert!(
                        tools.iter().any(|tool| tool == "ask_user_clarification"),
                        "{expected} must be able to ask for confirmation before risky writes"
                    );
                    assert!(
                        !tools.iter().any(|tool| tool == "shell"),
                        "{expected} must not inherit shell access"
                    );
                }
                ToolScope::Wildcard => panic!("{expected} must not use wildcard tools"),
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Spawn-hierarchy contract
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn orchestrator_is_chat_tier() {
        assert_eq!(find("orchestrator").agent_tier, AgentTier::Chat);
    }

    #[test]
    fn planner_is_reasoning_tier() {
        assert_eq!(find("planner").agent_tier, AgentTier::Reasoning);
    }

    #[test]
    fn other_builtins_default_to_worker_tier() {
        for def in load_builtins().unwrap() {
            if def.id == "orchestrator" || def.id == "planner" || def.id == "subconscious" {
                continue;
            }
            assert_eq!(
                def.agent_tier,
                AgentTier::Worker,
                "{} should default to worker tier (only orchestrator/planner/subconscious are non-worker today)",
                def.id
            );
        }
    }

    #[test]
    fn builtins_pass_tier_validation() {
        // load_builtins() already calls validate_tier_hierarchy; this
        // just makes the contract a named invariant in the test suite.
        let defs = load_builtins().expect("built-ins must pass tier validation");
        validate_tier_hierarchy(&defs).expect("explicit re-check must pass");
    }

    #[test]
    fn rejects_chat_to_chat_delegation() {
        let mut defs = load_builtins().unwrap();
        // Add a synthetic second chat agent and have the orchestrator
        // try to delegate to it.
        let mut bad_chat = find("orchestrator");
        bad_chat.id = "second_orchestrator".to_string();
        defs.push(bad_chat);
        let orch = defs.iter_mut().find(|d| d.id == "orchestrator").unwrap();
        orch.subagents
            .push(SubagentEntry::AgentId("second_orchestrator".into()));

        let err = validate_tier_hierarchy(&defs).expect_err("chat→chat must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("chat") && msg.contains("leaf"),
            "error should call out chat-tier leaf rule, got: {msg}"
        );
    }

    #[test]
    fn rejects_reasoning_to_reasoning_delegation() {
        let mut defs = load_builtins().unwrap();
        let mut bad_reasoning = find("planner");
        bad_reasoning.id = "second_planner".to_string();
        defs.push(bad_reasoning);
        let planner = defs.iter_mut().find(|d| d.id == "planner").unwrap();
        planner
            .subagents
            .push(SubagentEntry::AgentId("second_planner".into()));

        let err = validate_tier_hierarchy(&defs).expect_err("reasoning→reasoning must be rejected");
        assert!(err.to_string().contains("reasoning"));
    }

    #[test]
    fn rejects_worker_with_subagents() {
        let mut defs = load_builtins().unwrap();
        let researcher = defs.iter_mut().find(|d| d.id == "researcher").unwrap();
        researcher
            .subagents
            .push(SubagentEntry::AgentId("critic".into()));

        let err = validate_tier_hierarchy(&defs)
            .expect_err("worker with declared subagents must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("worker") && msg.contains("leaf"),
            "error should call out worker leaf rule, got: {msg}"
        );
    }

    #[test]
    fn allows_skill_wildcards_on_any_non_worker_tier() {
        // Skills wildcards collapse to delegate_to_integrations_agent
        // and must not be policed by the tier check (it'd be a false
        // positive — they fan out to a worker anyway).
        let mut defs = load_builtins().unwrap();
        let planner = defs.iter_mut().find(|d| d.id == "planner").unwrap();
        planner.subagents.push(SubagentEntry::Skills(
            crate::openhuman::agent::harness::definition::SkillsWildcard { skills: "*".into() },
        ));
        validate_tier_hierarchy(&defs).expect("skill wildcards on reasoning tier must validate");
    }
}
