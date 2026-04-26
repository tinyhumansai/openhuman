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
//! added by [`super::harness::builtin_definitions::all`] on top of the
//! loader output.
//!
//! Workspace-level overrides (`$OPENHUMAN_WORKSPACE/agents/*.toml`) are
//! handled separately by [`super::harness::definition_loader`] and merged
//! into the global registry, where they replace built-ins on `id`
//! collision.

use crate::openhuman::agent::harness::definition::{
    AgentDefinition, DefinitionSource, PromptBuilder, PromptSource,
};
use anyhow::{Context, Result};

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
        id: "tools_agent",
        toml: include_str!("tools_agent/agent.toml"),
        prompt_fn: super::tools_agent::prompt::build,
    },
    BuiltinAgent {
        id: "tool_maker",
        toml: include_str!("tool_maker/agent.toml"),
        prompt_fn: super::tool_maker::prompt::build,
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
        id: "welcome",
        toml: include_str!("welcome/agent.toml"),
        prompt_fn: super::welcome::prompt::build,
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
];

/// Parse every entry in [`BUILTINS`] into an [`AgentDefinition`].
///
/// Errors out of the whole call on any parse failure — built-in TOML is
/// baked into the binary and therefore must always be valid. Unit tests
/// below keep that invariant honest.
pub fn load_builtins() -> Result<Vec<AgentDefinition>> {
    BUILTINS.iter().map(parse_builtin).collect()
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
    use crate::openhuman::agent::harness::definition::{ModelSpec, SandboxMode, ToolScope};

    #[test]
    fn all_builtins_parse() {
        let defs = load_builtins().expect("built-in TOML must parse");
        assert_eq!(defs.len(), BUILTINS.len());
        assert_eq!(defs.len(), 15, "expected 15 built-in agents");
    }

    #[test]
    fn trigger_reactor_has_agentic_hint_and_narrow_tools() {
        let def = find("trigger_reactor");
        assert!(matches!(def.model, ModelSpec::Hint(ref h) if h == "agentic"));
        match &def.tools {
            ToolScope::Named(tools) => {
                assert!(
                    tools.iter().any(|t| t == "memory_recall"),
                    "trigger_reactor needs memory_recall"
                );
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
                        skills: &[],
                        dispatcher_instructions: "",
                        learned: LearnedContextData::default(),
                        visible_tool_names: &empty_visible,
                        tool_call_format: ToolCallFormat::PFormat,
                        connected_integrations: &empty_integrations,
                        connected_identities_md: String::new(),
                        include_profile: false,
                        include_memory_md: false,
                        curated_snapshot: None,
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
    fn orchestrator_has_reasoning_hint_and_named_tools() {
        let def = find("orchestrator");
        assert!(matches!(def.model, ModelSpec::Hint(ref h) if h == "reasoning"));
        match def.tools {
            ToolScope::Named(tools) => {
                assert!(tools.iter().any(|t| t == "spawn_subagent"));
                assert!(!tools.iter().any(|t| t == "shell"));
                assert!(!tools.iter().any(|t| t == "file_write"));
            }
            ToolScope::Wildcard => panic!("orchestrator must have named tool allowlist"),
        }
        assert_eq!(def.max_iterations, 15);
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
    fn critic_is_read_only() {
        let def = find("critic");
        assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
        assert!(def.omit_safety_preamble);
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
        assert!(!def.omit_memory_context);
        assert!(def.omit_identity);
        assert!(def.omit_safety_preamble);
        assert_eq!(def.max_iterations, 8);
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
                assert!(
                    tools.iter().any(|t| t == "memory_recall"),
                    "help needs memory_recall for personalisation"
                );
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

    #[test]
    fn welcome_has_onboarding_and_memory_tools() {
        let def = find("welcome");
        assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
        match &def.tools {
            ToolScope::Named(tools) => {
                assert!(
                    tools.iter().any(|t| t == "check_onboarding_status"),
                    "welcome needs check_onboarding_status"
                );
                assert!(
                    tools.iter().any(|t| t == "complete_onboarding"),
                    "welcome needs complete_onboarding"
                );
                assert!(
                    tools.iter().any(|t| t == "memory_recall"),
                    "welcome needs memory_recall"
                );
                assert!(
                    tools.iter().any(|t| t == "composio_authorize"),
                    "welcome needs composio_authorize"
                );
                assert!(
                    tools.iter().any(|t| t == "gitbooks_search"),
                    "welcome needs gitbooks_search to answer 'how does X work' during onboarding"
                );
                assert!(
                    tools.iter().any(|t| t == "gitbooks_get_page"),
                    "welcome needs gitbooks_get_page for full-page lookups"
                );
                // Welcome must not gain write/exec power; onboarding stays read-only.
                assert!(!tools.iter().any(|t| t == "shell"));
                assert!(!tools.iter().any(|t| t == "file_write"));
                assert!(!tools.iter().any(|t| t == "curl"));
            }
            ToolScope::Wildcard => panic!("welcome must have a Named tool scope"),
        }
        assert!(!def.omit_memory_context);
        assert!(def.omit_identity);
        assert_eq!(def.max_iterations, 6);
    }
}
