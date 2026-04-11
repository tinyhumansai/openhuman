//! Built-in [`AgentDefinition`]s.
//!
//! The authoritative list of built-in agents lives in
//! [`crate::openhuman::agent::agents`] — each agent is a subfolder
//! containing `agent.toml` + `prompt.md`. This module is a thin
//! wrapper that loads that set and appends the synthetic `fork`
//! definition (used for byte-exact prompt-cache reuse by the sub-agent
//! runner).
//!
//! Custom TOML definitions loaded later by
//! [`super::definition_loader`] override any built-in with the same id.

use super::definition::{
    AgentDefinition, DefinitionSource, ModelSpec, PromptSource, SandboxMode, ToolScope,
};

/// All built-in definitions, in stable order.
///
/// Panics if the baked-in built-in TOML fails to parse — a compile-time
/// invariant enforced by the unit tests in
/// [`crate::openhuman::agent::agents::tests`].
pub fn all() -> Vec<AgentDefinition> {
    let mut out = crate::openhuman::agent::agents::load_builtins()
        .expect("built-in agent TOML must always parse (see agents/*/agent.toml)");
    out.push(fork_definition());
    out
}

/// The synthetic `fork` definition. Tells the runner to bypass normal
/// prompt construction and replay the parent's exact rendered system
/// prompt + tool schemas + message prefix from
/// [`super::fork_context::ForkContext`]. The OpenAI-compatible backend's
/// automatic prefix caching turns this into a real token win.
pub fn fork_definition() -> AgentDefinition {
    AgentDefinition {
        id: "fork".into(),
        when_to_use: "Spawn a parallel sub-task that shares the parent's full system \
                      prompt, tool set, and message history byte-for-byte. Use when \
                      decomposing a task into independent parallel work streams that \
                      benefit from prefix-cache reuse on the inference backend."
            .into(),
        display_name: Some("Fork".into()),
        // Prompt source is irrelevant — the runner reads from ForkContext.
        system_prompt: PromptSource::Inline(String::new()),
        // Fork preserves bytes — DO NOT strip anything from the parent's prompt.
        omit_identity: false,
        omit_memory_context: false,
        omit_safety_preamble: false,
        omit_skills_catalog: false,
        model: ModelSpec::Inherit,
        // Inherit the parent's temperature too — set to a sentinel that the
        // runner replaces with the parent's actual temp at spawn time.
        // (We use 0.7 here as a safe default for documentation; the runner
        // overrides it from `ParentExecutionContext::temperature`.)
        temperature: 0.7,
        tools: ToolScope::Wildcard,
        disallowed_tools: vec![],
        skill_filter: None,
        category_filter: None,
        // Fork inherits the parent's max iterations from the runtime.
        max_iterations: 15,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        uses_fork_context: true,
        source: DefinitionSource::Builtin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_definitions_present() {
        let defs = all();
        // 8 built-in agents (from agents/) + 1 synthetic `fork`.
        assert_eq!(defs.len(), 9);
    }

    #[test]
    fn all_builtin_ids_are_stamped_builtin_source() {
        for def in all() {
            assert_eq!(
                def.source,
                DefinitionSource::Builtin,
                "{} should be Builtin",
                def.id
            );
        }
    }

    #[test]
    fn fork_definition_has_uses_fork_context_true() {
        let def = fork_definition();
        assert_eq!(def.id, "fork");
        assert!(def.uses_fork_context);
        assert!(matches!(def.model, ModelSpec::Inherit));
        assert!(matches!(def.tools, ToolScope::Wildcard));
        // Fork preserves bytes — must NOT strip anything.
        assert!(!def.omit_identity);
        assert!(!def.omit_memory_context);
        assert!(!def.omit_safety_preamble);
        assert!(!def.omit_skills_catalog);
    }

    #[test]
    fn expected_builtin_ids_are_present() {
        let ids: Vec<String> = all().into_iter().map(|d| d.id).collect();
        for expected in [
            "orchestrator",
            "planner",
            "code_executor",
            "skills_agent",
            "tool_maker",
            "researcher",
            "critic",
            "archivist",
            "fork",
        ] {
            assert!(ids.contains(&expected.to_string()), "missing {expected}");
        }
    }
}
