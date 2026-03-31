//! Inter-skill communication bridge.
//!
//! Deliberately restricted: running skills cannot invoke tools owned by
//! other skills. Keep this boundary explicit even if bridge APIs are
//! reintroduced in the future.

use std::sync::Arc;

use crate::openhuman::skills::skill_registry::SkillRegistry;

/// List all running skills.
/// Returns a JSON string: `[{"skillId":"...","name":"...","status":"..."}]`
pub fn list_skills(registry: &Arc<SkillRegistry>) -> String {
    let skills = registry.list_skills();
    let simplified: Vec<serde_json::Value> = skills
        .iter()
        .map(|s| {
            serde_json::json!({
                "skillId": s.skill_id,
                "name": s.name,
                "status": s.status,
            })
        })
        .collect();
    serde_json::to_string(&simplified).unwrap_or_else(|_| "[]".to_string())
}

/// Call a tool on another skill.
///
/// Spawns an OS thread with a mini Tokio runtime to make the async
/// registry call without conflicting with the V8 runtime or the
/// outer Tokio runtime.
///
/// Returns the ToolResult as a JSON string.
pub fn call_tool(
    _registry: &Arc<SkillRegistry>,
    caller_skill_id: &str,
    target_skill_id: &str,
    _tool_name: &str,
    _arguments_json: &str,
) -> Result<String, String> {
    Err(format!(
        "Cross-skill tool invocation is disabled: '{}' cannot call '{}'",
        caller_skill_id, target_skill_id
    ))
}
