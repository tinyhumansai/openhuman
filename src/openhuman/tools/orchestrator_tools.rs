//! Dynamic orchestrator tool generation.
//!
//! Instead of a single `spawn_subagent` mega-tool, this module generates
//! one tool per subagent archetype + one tool per installed skill. The
//! orchestrator's function-calling schema becomes a flat list of
//! well-named tools:
//!
//!   `notion`, `gmail`, `research`, `run_code`, `review_code`, `plan`
//!
//! Each tool's `execute()` internally calls `run_subagent` with the
//! correct definition + skill_filter. The LLM just picks the right tool
//! by name — no `agent_id` or `skill_filter` to remember.

use std::collections::HashSet;

use super::{
    skill_description, ArchetypeDelegationTool, SkillDelegationTool, SpawnSubagentTool, Tool,
    ARCHETYPE_TOOLS,
};

/// Build the orchestrator's tool list: one tool per installed skill +
/// one tool per archetype. Also includes `spawn_subagent` as a fallback
/// for advanced use cases (fork mode, custom agent_ids).
///
/// Call this at agent build time when the visible-tool filter is active
/// (i.e. the main agent is an orchestrator).
pub fn collect_orchestrator_tools() -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    let mut seen_names: HashSet<String> = HashSet::new();

    // ── Skill-based tools (dynamic, from running skills) ──────────────
    if let Some(engine) = crate::openhuman::skills::global_engine() {
        let all_skill_tools = engine.all_tools();
        // Extract unique skill ids from tool names like "notion__search"
        let mut skill_ids: Vec<String> = all_skill_tools
            .iter()
            .filter_map(|(skill_id, _)| {
                if seen_names.insert(skill_id.clone()) {
                    Some(skill_id.clone())
                } else {
                    None
                }
            })
            .collect();
        skill_ids.sort();
        skill_ids.dedup();

        for skill_id in &skill_ids {
            log::info!(
                "[orchestrator_tools] registering skill delegation tool: {}",
                skill_id
            );
            tools.push(Box::new(SkillDelegationTool {
                tool_name: skill_id.clone(),
                skill_id: skill_id.clone(),
                tool_description: skill_description(skill_id),
            }));
        }
    }

    // ── Archetype-based tools (static) ────────────────────────────────
    for (tool_name, agent_id, description) in ARCHETYPE_TOOLS {
        log::info!(
            "[orchestrator_tools] registering archetype delegation tool: {} -> {}",
            tool_name,
            agent_id
        );
        tools.push(Box::new(ArchetypeDelegationTool {
            tool_name: tool_name.to_string(),
            agent_id: agent_id.to_string(),
            tool_description: description.to_string(),
        }));
    }

    // ── spawn_subagent as fallback for advanced use ────────────────────
    // Fork mode, custom agent_ids, explicit skill_filter, etc.
    tools.push(Box::new(SpawnSubagentTool::new()));

    log::info!(
        "[orchestrator_tools] total orchestrator tools: {}",
        tools.len()
    );

    tools
}
