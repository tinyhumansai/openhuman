//! Built-in [`AgentDefinition`]s.
//!
//! The authoritative list of built-in agents lives in
//! [`crate::openhuman::agent::registry::agents`] — each agent is a subfolder
//! containing `agent.toml` + `prompt.md`. This module is a thin
//! wrapper that loads that set.
//!
//! Custom TOML definitions loaded later by
//! [`super::definition_loader`] override any built-in with the same id.

use super::definition::AgentDefinition;
#[cfg(test)]
use super::definition::DefinitionSource;

/// All built-in definitions, in stable order.
///
/// Panics if the baked-in built-in TOML fails to parse. `include_str!`
/// guarantees at compile time that each file exists, but the actual
/// TOML parse happens at runtime; the unit tests in
/// [`crate::openhuman::agent::registry::agents`] verify in CI that every entry in
/// [`crate::openhuman::agent::registry::agents::BUILTINS`] still parses cleanly.
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
    let mut defs = crate::openhuman::agent::registry::agents::load_builtins()
        .expect("built-in agent TOML must always parse (see agents/*/agent.toml)");
    #[cfg(test)]
    {
        defs.push(test_main_def());
        defs.push(test_inherit_echo_def());
        defs.push(test_inherit_parallel_worker_def());
    }
    defs
}

/// Test-only parent used by `AgentBuilder`'s default `agent_definition_name = "main"`.
///
/// Production builds do not ship a `main` agent definition. In tests, the
/// default builder path drives inherit-based fake subagents through the real
/// delegation tools, so the parent must explicitly allow those test children.
#[cfg(test)]
pub(crate) fn test_main_def() -> AgentDefinition {
    use super::definition::{
        AgentTier, ModelSpec, PromptSource, SandboxMode, SubagentEntry, ToolScope,
    };
    AgentDefinition {
        id: "main".into(),
        when_to_use: "test-only default parent agent".into(),
        display_name: None,
        system_prompt: PromptSource::Inline("You are the test parent agent.".into()),
        omit_identity: true,
        omit_memory_context: true,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: true,
        omit_memory_md: true,
        model: ModelSpec::Inherit,
        temperature: 0.0,
        tools: ToolScope::Wildcard,
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        max_iterations: 8,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        trigger_memory_agent: Default::default(),
        tokenjuice_compression:
            crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression::Auto,
        subagents: vec![
            SubagentEntry::AgentId("__test_inherit_echo".into()),
            SubagentEntry::AgentId("__test_inherit_parallel_worker".into()),
        ],
        delegate_name: None,
        agent_tier: AgentTier::Chat,
        source: DefinitionSource::Builtin,
        graph: Default::default(),
    }
}

/// Test-only sub-agent: `ModelSpec::Inherit`, wildcard tools, minimal
/// prompt. Inherit means the runner uses `parent.provider` verbatim,
/// so a test's scripted `MockProvider` reaches the sub-agent loop —
/// which is exactly what the full-path spawn test needs to assert the
/// dispatch → run_subagent → result-threading chain end to end.
/// Provider *routing* for `Hint` sub-agents is covered separately by
/// `subagent_runner::ops::tests::resolve_subagent_source_*`.
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
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        trigger_memory_agent: Default::default(),
        tokenjuice_compression:
            crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression::Auto,
        subagents: vec![],
        delegate_name: None,
        agent_tier: crate::openhuman::agent::harness::definition::AgentTier::Worker,
        source: DefinitionSource::Builtin,
        graph: Default::default(),
    }
}

/// Test-only sub-agent: inherits the parent's provider and exposes a
/// single named tool so long-running parallel fan-out tests can drive
/// repeated nested tool calls through the real sub-agent loop.
#[cfg(test)]
pub(crate) fn test_inherit_parallel_worker_def() -> AgentDefinition {
    use super::definition::{ModelSpec, PromptSource, SandboxMode, ToolScope};
    AgentDefinition {
        id: "__test_inherit_parallel_worker".into(),
        when_to_use: "test-only parallel sub-agent that inherits the parent provider".into(),
        display_name: None,
        system_prompt: PromptSource::Inline("You are a test parallel worker.".into()),
        omit_identity: true,
        omit_memory_context: true,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: true,
        omit_memory_md: true,
        model: ModelSpec::Inherit,
        temperature: 0.0,
        tools: ToolScope::Named(vec!["fixture_step".into()]),
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        max_iterations: 6,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        trigger_memory_agent: Default::default(),
        tokenjuice_compression:
            crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression::Auto,
        subagents: vec![],
        delegate_name: None,
        agent_tier: crate::openhuman::agent::harness::definition::AgentTier::Worker,
        source: DefinitionSource::Builtin,
        graph: Default::default(),
    }
}

#[cfg(test)]
#[path = "builtin_definitions_tests.rs"]
mod tests;
