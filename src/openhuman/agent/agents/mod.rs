//! Built-in agent definitions.
//!
//! Every built-in agent lives in its own subfolder here, with exactly
//! two files:
//!
//! * `agent.toml`  — id, when_to_use, model, tool allowlist, sandbox,
//!   iteration cap, and the `omit_*` flags. Parsed
//!   directly into [`AgentDefinition`] via serde.
//! * `prompt.md`   — the sub-agent's system prompt body.
//!
//! Adding a new built-in agent = creating a new subfolder with those two
//! files and appending one entry to [`BUILTINS`] below. There are no
//! match arms to update, no enum variants to add, and no `include_str!`
//! paths scattered across the harness.
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
    AgentDefinition, DefinitionSource, PromptSource,
};
use anyhow::{Context, Result};

/// A single built-in agent: its id plus the two files that define it.
///
/// Kept as a static slice (rather than e.g. `include_dir!`) so the
/// compile-time file-existence check is explicit and grep-friendly.
pub struct BuiltinAgent {
    pub id: &'static str,
    pub toml: &'static str,
    pub prompt: &'static str,
}

/// Every built-in agent, in stable display order.
///
/// **This is the only list you touch when adding a new built-in agent.**
pub const BUILTINS: &[BuiltinAgent] = &[
    BuiltinAgent {
        id: "orchestrator",
        toml: include_str!("orchestrator/agent.toml"),
        prompt: include_str!("orchestrator/prompt.md"),
    },
    BuiltinAgent {
        id: "planner",
        toml: include_str!("planner/agent.toml"),
        prompt: include_str!("planner/prompt.md"),
    },
    BuiltinAgent {
        id: "code_executor",
        toml: include_str!("code_executor/agent.toml"),
        prompt: include_str!("code_executor/prompt.md"),
    },
    BuiltinAgent {
        id: "skills_agent",
        toml: include_str!("skills_agent/agent.toml"),
        prompt: include_str!("skills_agent/prompt.md"),
    },
    BuiltinAgent {
        id: "tool_maker",
        toml: include_str!("tool_maker/agent.toml"),
        prompt: include_str!("tool_maker/prompt.md"),
    },
    BuiltinAgent {
        id: "researcher",
        toml: include_str!("researcher/agent.toml"),
        prompt: include_str!("researcher/prompt.md"),
    },
    BuiltinAgent {
        id: "critic",
        toml: include_str!("critic/agent.toml"),
        prompt: include_str!("critic/prompt.md"),
    },
    BuiltinAgent {
        id: "archivist",
        toml: include_str!("archivist/agent.toml"),
        prompt: include_str!("archivist/prompt.md"),
    },
    BuiltinAgent {
        id: "trigger_triage",
        toml: include_str!("trigger_triage/agent.toml"),
        prompt: include_str!("trigger_triage/prompt.md"),
    },
    BuiltinAgent {
        id: "trigger_reactor",
        toml: include_str!("trigger_reactor/agent.toml"),
        prompt: include_str!("trigger_reactor/prompt.md"),
    },
    BuiltinAgent {
        id: "morning_briefing",
        toml: include_str!("morning_briefing/agent.toml"),
        prompt: include_str!("morning_briefing/prompt.md"),
    },
    BuiltinAgent {
        id: "welcome",
        toml: include_str!("welcome/agent.toml"),
        prompt: include_str!("welcome/prompt.md"),
    },
    BuiltinAgent {
        id: "summarizer",
        toml: include_str!("summarizer/agent.toml"),
        prompt: include_str!("summarizer/prompt.md"),
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

    // Inject the prompt body and stamp the source.
    def.system_prompt = PromptSource::Inline(b.prompt.to_string());
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
        assert_eq!(defs.len(), 13, "expected 13 built-in agents");
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
        for def in load_builtins().unwrap() {
            match &def.system_prompt {
                PromptSource::Inline(body) => {
                    assert!(!body.is_empty(), "{} has empty prompt", def.id);
                }
                PromptSource::File { .. } => {
                    panic!("{} should use inline prompt, not File", def.id);
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
    fn skills_agent_is_wildcard_with_skill_category_filter() {
        let def = find("skills_agent");
        assert!(matches!(def.tools, ToolScope::Wildcard));
        assert_eq!(
            def.category_filter,
            Some(crate::openhuman::tools::ToolCategory::Skill)
        );
        assert!(!def.omit_safety_preamble);
    }

    #[test]
    fn archivist_runs_in_background() {
        let def = find("archivist");
        assert!(def.background);
        assert_eq!(def.max_iterations, 3);
    }

    #[test]
    fn morning_briefing_is_read_only_with_skill_filter() {
        let def = find("morning_briefing");
        assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
        assert!(matches!(def.tools, ToolScope::Wildcard));
        assert_eq!(
            def.category_filter,
            Some(crate::openhuman::tools::ToolCategory::Skill)
        );
        assert!(!def.omit_memory_context);
        assert!(def.omit_identity);
        assert!(def.omit_safety_preamble);
        assert_eq!(def.max_iterations, 8);
    }

    #[test]
    fn welcome_has_onboarding_and_memory_tools() {
        let def = find("welcome");
        assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
        match &def.tools {
            ToolScope::Named(tools) => {
                assert_eq!(tools.len(), 3, "welcome should have exactly three tools");
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
            }
            ToolScope::Wildcard => panic!("welcome must have a Named tool scope"),
        }
        assert!(!def.omit_memory_context);
        assert!(def.omit_identity);
        assert_eq!(def.max_iterations, 6);
    }
}
