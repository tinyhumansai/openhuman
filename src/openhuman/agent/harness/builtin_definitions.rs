//! Built-in [`AgentDefinition`]s.
//!
//! The authoritative list of built-in agents lives in
//! [`crate::openhuman::agent::agents`] — each agent is a subfolder
//! containing `agent.toml` + `prompt.md`. This module is a thin
//! wrapper that loads that set.
//!
//! Custom TOML definitions loaded later by
//! [`super::definition_loader`] override any built-in with the same id.

use super::definition::{AgentDefinition, DefinitionSource};

/// All built-in definitions, in stable order.
///
/// Panics if the baked-in built-in TOML fails to parse. `include_str!`
/// guarantees at compile time that each file exists, but the actual
/// TOML parse happens at runtime; the unit tests in
/// [`crate::openhuman::agent::agents`] verify in CI that every entry in
/// [`crate::openhuman::agent::agents::BUILTINS`] still parses cleanly.
///
/// In `#[cfg(test)]` builds the list additionally contains
/// [`test_inherit_echo_def`] — a sub-agent with `ModelSpec::Inherit`
/// that exists solely so the spawn-subagent end-to-end test can
/// exercise the dispatch/threading plumbing with the *parent's*
/// provider (every shipped builtin uses `Hint(...)`, which after
/// #1710 builds a fresh factory provider and therefore can't share a
/// test's `MockProvider`). It is never compiled into release builds.
pub fn all() -> Vec<AgentDefinition> {
    #[allow(unused_mut)]
    let mut defs = crate::openhuman::agent::agents::load_builtins()
        .expect("built-in agent TOML must always parse (see agents/*/agent.toml)");
    #[cfg(test)]
    defs.push(test_inherit_echo_def());
    defs
}

/// Test-only sub-agent: `ModelSpec::Inherit`, wildcard tools, minimal
/// prompt. Inherit means the runner uses `parent.provider` verbatim,
/// so a test's scripted `MockProvider` reaches the sub-agent loop —
/// which is exactly what the full-path spawn test needs to assert the
/// dispatch → run_subagent → result-threading chain end to end.
/// Provider *routing* for `Hint` sub-agents is covered separately by
/// `subagent_runner::ops::tests::resolve_subagent_provider_*`.
#[cfg(test)]
pub(crate) fn test_inherit_echo_def() -> AgentDefinition {
    use super::definition::{ModelSpec, PromptSource, SandboxMode, ToolScope};
    AgentDefinition {
        id: "__test_inherit_echo".into(),
        when_to_use: "test-only sub-agent that inherits the parent provider".into(),
        display_name: None,
        system_prompt: PromptSource::Inline("You are a test sub-agent.".into()),
        omit_identity: true,
        omit_memory_context: true,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: true,
        omit_memory_md: true,
        model: ModelSpec::Inherit,
        temperature: 0.0,
        tools: ToolScope::Named(vec![]),
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        max_iterations: 3,
        max_result_chars: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        subagents: vec![],
        delegate_name: None,
        source: DefinitionSource::Builtin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_definitions_present() {
        let defs = all();
        // +1 for the cfg(test) `__test_inherit_echo` def appended by all().
        assert_eq!(
            defs.len(),
            crate::openhuman::agent::agents::BUILTINS.len() + 1
        );
    }

    #[test]
    fn test_inherit_echo_is_present_and_inherits() {
        use super::super::definition::ModelSpec;
        let def = all()
            .into_iter()
            .find(|d| d.id == "__test_inherit_echo")
            .expect("test-only inherit agent must be registered in test builds");
        assert!(
            matches!(def.model, ModelSpec::Inherit),
            "must be Inherit so the sub-agent uses the parent's (mock) provider"
        );
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
    fn expected_builtin_ids_are_present() {
        let ids: Vec<String> = all().into_iter().map(|d| d.id).collect();
        for expected in [
            "orchestrator",
            "planner",
            "code_executor",
            "integrations_agent",
            "tool_maker",
            "researcher",
            "critic",
            "archivist",
            "summarizer",
        ] {
            assert!(ids.contains(&expected.to_string()), "missing {expected}");
        }
    }
}
