//! The single resolution point for a turn's tool-iteration cap.

use crate::agent::harness::definition::AgentDefinition;
use crate::config::AgentConfig;

/// The tool-iteration cap a turn built from `agent` runs under.
///
/// In order of precedence:
///
/// 1. `agent.max_tool_iterations_override` (config or
///    `OPENHUMAN_AGENT_MAX_TOOL_ITERATIONS`), when set and non-zero. It is the
///    operator's explicit word and wins over any definition, up or down
///    (#6958: the only way to lift the orchestrator was patching its
///    `agent.toml`).
/// 2. The definition's `effective_max_iterations()` (#4868), which honours
///    `iteration_policy = "extended"`. It replaces the global default rather
///    than being bounded by it, so a strict short-lived agent keeps its small
///    cap and the orchestrator keeps its large one.
/// 3. `agent.max_tool_iterations`, for a turn with no definition.
///
/// Every direct-invocation path (flows build/discover, agent nodes, cron, the
/// MCP server, RPC and web chat) resolves its cap here through the session
/// factory. Sub-agents spawned mid-turn resolve theirs in
/// `subagent_host::ops::runner` from their own definitions.
pub(crate) fn resolve_max_tool_iterations(
    agent: &AgentConfig,
    def: Option<&AgentDefinition>,
) -> usize {
    if let Some(cap) = agent.max_tool_iterations_override.filter(|cap| *cap > 0) {
        log::info!(
            "[agent::builder] iteration cap from explicit override: {cap} \
             (definition={:?} definition_cap={:?} global={})",
            def.map(|d| d.id.as_str()),
            def.map(AgentDefinition::effective_max_iterations),
            agent.max_tool_iterations,
        );
        return cap;
    }
    match def {
        Some(def) => {
            let cap = def.effective_max_iterations();
            log::info!(
                "[agent::builder] iteration cap from definition agent_id={}: \
                 max_iterations={} iteration_policy={:?} -> {cap} (global default {})",
                def.id,
                def.max_iterations,
                def.iteration_policy,
                agent.max_tool_iterations,
            );
            cap
        }
        None => agent.max_tool_iterations,
    }
}

#[cfg(test)]
#[path = "iteration_cap_tests.rs"]
mod tests;
