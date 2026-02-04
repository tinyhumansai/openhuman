//! Inter-skill communication bridge.
//!
//! Allows skills to list other running skills and call their tools.
//! Tool calls are performed on a separate OS thread with a mini Tokio
//! runtime to avoid deadlocking the V8 runtime.

use std::sync::Arc;

use crate::runtime::skill_registry::SkillRegistry;

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
    registry: &Arc<SkillRegistry>,
    caller_skill_id: &str,
    target_skill_id: &str,
    tool_name: &str,
    arguments_json: &str,
) -> Result<String, String> {
    // Prevent self-calls (would deadlock — the skill's message loop
    // is blocked waiting for us, so it can't process the tool call)
    if caller_skill_id == target_skill_id {
        return Err("Cannot call tools on self (would deadlock)".to_string());
    }

    let registry = registry.clone();
    let target = target_skill_id.to_string();
    let tool = tool_name.to_string();
    let arguments: serde_json::Value =
        serde_json::from_str(arguments_json).unwrap_or(serde_json::json!({}));

    let (tx, rx) = std::sync::mpsc::sync_channel(1);

    std::thread::spawn(move || {
        // Create a mini single-threaded Tokio runtime for the async call.
        // We can't reuse the main runtime because we're on an OS thread
        // outside it, and block_in_place would conflict with V8.
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let _ = tx.send(Err(format!("Failed to create runtime: {e}")));
                return;
            }
        };

        let result = rt.block_on(async { registry.call_tool(&target, &tool, arguments).await });

        let _ = tx.send(result);
    });

    let result = rx
        .recv_timeout(std::time::Duration::from_secs(60))
        .map_err(|e| format!("Inter-skill tool call timed out: {e}"))??;

    serde_json::to_string(&result).map_err(|e| format!("Failed to serialize tool result: {e}"))
}
