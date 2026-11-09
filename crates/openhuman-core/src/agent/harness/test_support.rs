//! Test-only agent definitions used by harness and orchestration tests.
//!
//! The authoritative built-in list lives in
//! [`crate::agent_registry::agents`]. Flow agent prompts are fetched from
//! TinyFlows Copilot through the module bus; the test fixtures here let
//! orchestration tests exercise those definitions without moving registry
//! ownership into the agent harness.

use crate::agent_registry::host_definition::AgentDefinition;
use tinyagents_definition::DefinitionSource;

/// Test-only parent used by `AgentBuilder`'s default `agent_definition_name = "main"`.
///
/// Production builds do not ship a `main` agent definition. In tests, the
/// default builder path drives inherit-based fake subagents through the real
/// delegation tools, so the parent must explicitly allow those test children.
#[cfg(test)]
pub(crate) fn test_main_def() -> AgentDefinition {
    use tinyagents_definition::{
        AgentTier, ModelSpec, PromptSource, SandboxMode, SubagentEntry, ToolScope,
    };
    AgentDefinition {
        id: "main".into(),
        when_to_use: "test-only default parent agent".into(),
        display_name: None,
        system_prompt: PromptSource::Inline("You are the test parent agent.".into()),
        model: ModelSpec::Inherit,
        temperature: 0.0,
        tools: ToolScope::Wildcard,
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        deferred_tools: Vec::new(),
        tool_rules: None,
        max_iterations: 8,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        subagents: vec![
            SubagentEntry::AgentId("__test_inherit_echo".into()),
            SubagentEntry::AgentId("__test_inherit_parallel_worker".into()),
        ],
        delegate_name: None,
        agent_tier: AgentTier::Chat,
        source: DefinitionSource::Builtin,
        extensions: crate::agent_registry::host_definition::AgentExt {
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: true,
            tokenjuice_compression: tinyjuice_bus::types::AgentTokenjuiceCompression::Auto,
            searches_connected_mcp: false,
            ..Default::default()
        },
        role: None,
    }
}

/// Test-only sub-agent: `ModelSpec::Inherit`, wildcard tools, minimal
/// prompt. Inherit means the runner uses `parent.provider` verbatim,
/// so a test's scripted `MockProvider` reaches the sub-agent loop —
/// which is exactly what the full-path spawn test needs to assert the
/// dispatch → detached child execution → result-threading chain end to end.
/// Provider *routing* for `Hint` sub-agents is covered separately by
/// `subagent_host::ops::tests::resolve_subagent_source_*`.
#[cfg(test)]
pub(crate) fn test_inherit_echo_def() -> AgentDefinition {
    use tinyagents_definition::{ModelSpec, PromptSource, SandboxMode, ToolScope};
    AgentDefinition {
        id: "__test_inherit_echo".into(),
        when_to_use: "test-only sub-agent that inherits the parent provider".into(),
        display_name: None,
        system_prompt: PromptSource::Inline("You are a test sub-agent.".into()),
        model: ModelSpec::Inherit,
        temperature: 0.0,
        tools: ToolScope::Named(vec![]),
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        deferred_tools: Vec::new(),
        tool_rules: None,
        max_iterations: 3,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        subagents: vec![],
        delegate_name: None,
        agent_tier: tinyagents_definition::AgentTier::Worker,
        source: DefinitionSource::Builtin,
        extensions: crate::agent_registry::host_definition::AgentExt {
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: true,
            tokenjuice_compression: tinyjuice_bus::types::AgentTokenjuiceCompression::Auto,
            searches_connected_mcp: false,
            ..Default::default()
        },
        role: None,
    }
}

/// Test-only sub-agent: inherits the parent's provider and exposes a
/// single named tool so long-running parallel fan-out tests can drive
/// repeated nested tool calls through the real sub-agent loop.
#[cfg(test)]
pub(crate) fn test_inherit_parallel_worker_def() -> AgentDefinition {
    use tinyagents_definition::{ModelSpec, PromptSource, SandboxMode, ToolScope};
    AgentDefinition {
        id: "__test_inherit_parallel_worker".into(),
        when_to_use: "test-only parallel sub-agent that inherits the parent provider".into(),
        display_name: None,
        system_prompt: PromptSource::Inline("You are a test parallel worker.".into()),
        model: ModelSpec::Inherit,
        temperature: 0.0,
        tools: ToolScope::Named(vec!["fixture_step".into()]),
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        deferred_tools: Vec::new(),
        tool_rules: None,
        max_iterations: 6,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        subagents: vec![],
        delegate_name: None,
        agent_tier: tinyagents_definition::AgentTier::Worker,
        source: DefinitionSource::Builtin,
        extensions: crate::agent_registry::host_definition::AgentExt {
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: true,
            tokenjuice_compression: tinyjuice_bus::types::AgentTokenjuiceCompression::Auto,
            searches_connected_mcp: false,
            ..Default::default()
        },
        role: None,
    }
}
