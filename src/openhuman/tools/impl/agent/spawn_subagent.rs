//! Tool: `spawn_subagent` — delegate a sub-task to a specialised sub-agent.
//!
//! The orchestrator (or any parent agent that has this tool registered)
//! calls `spawn_subagent` to hand off a focused sub-task. The runner
//! looks up the requested [`AgentDefinition`] in the global registry,
//! filters the parent's tool registry per the definition, builds a
//! narrow system prompt, and runs an inner tool-call loop using the
//! parent's provider. The sub-agent's intra-loop history is collapsed
//! into a single text result that the parent receives as a normal
//! `tool_result`.
//!
//! Modes:
//! - `"typed"` (default) — narrow prompt + filtered tools + cheaper
//!   model. Use for delegated work where the parent doesn't need to
//!   share its full context.
//! - `"fork"` — replay the parent's *exact* rendered prompt + tool
//!   schemas + message prefix. Use for parallel decomposition of a
//!   homogeneous task; relies on the inference backend's automatic
//!   prefix caching for token savings.
//!
//! API specialists (Notion, Gmail, …) ride on the built-in `skills_agent`
//! definition by passing `skill_filter: "<skill_id>"`, which restricts
//! the resolved tool list to tools whose names start with `{skill}__`.

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{run_subagent, SubagentRunOptions};
use crate::openhuman::event_bus::{publish_global, DomainEvent};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};
use async_trait::async_trait;
use serde_json::json;

/// Spawns a sub-agent of the requested type to handle a delegated task.
///
/// Registered into the parent agent's tool list by
/// [`crate::openhuman::tools::all_tools_with_runtime`]. The orchestrator
/// archetype's tool whitelist already includes `spawn_subagent`, so
/// orchestrated runs see it; non-orchestrator parents see it too unless
/// explicitly removed.
pub struct SpawnSubagentTool;

impl Default for SpawnSubagentTool {
    fn default() -> Self {
        Self::new()
    }
}

impl SpawnSubagentTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SpawnSubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Delegate a task to a specialised sub-agent. \
         See the Delegation Guide in the system prompt for \
         available agent_ids and when to use each."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        // Build the agent_id enum dynamically from the global registry
        // when it's been initialised. Falls back to a string-with-hint
        // when the registry hasn't been set up yet (e.g. early tests).
        let agent_ids: Vec<String> = AgentDefinitionRegistry::global()
            .map(|reg| reg.list().iter().map(|d| d.id.clone()).collect())
            .unwrap_or_default();

        let agent_id_schema = if agent_ids.is_empty() {
            json!({
                "type": "string",
                "description": "Sub-agent id (e.g. code_executor, researcher, critic, fork)."
            })
        } else {
            json!({
                "type": "string",
                "enum": agent_ids,
                "description": "Sub-agent id from the registry."
            })
        };

        json!({
            "type": "object",
            "required": ["agent_id", "prompt"],
            "properties": {
                "agent_id": agent_id_schema,
                // Back-compat alias — older callers used `archetype`.
                "archetype": {
                    "type": "string",
                    "description": "Deprecated alias for `agent_id`. Use `agent_id` going forward."
                },
                "prompt": {
                    "type": "string",
                    "description": "Clear, specific instruction for the sub-agent. The sub-agent has no memory of the parent's conversation, so include all context the sub-agent needs to act."
                },
                "context": {
                    "type": "string",
                    "description": "Optional context blob from prior task results. Rendered as a `[Context]` block before the prompt."
                },
                "skill_filter": {
                    "type": "string",
                    "description": "Optional skill id (e.g. `notion`, `gmail`) — when set, the sub-agent's tool list is restricted to tools named `{skill}__*`. Pair with `agent_id: skills_agent` for an API specialist."
                },
                "category_filter": {
                    "type": "string",
                    "enum": ["system", "skill"],
                    "description": "Optional tool-category restriction. `skill` scopes the sub-agent to QuickJS skill-bridge tools (Notion, Gmail, Telegram, …); `system` scopes it to built-in Rust tools (shell, file_*, memory_*, …). Overrides the definition's `category_filter` for this single spawn."
                },
                "mode": {
                    "type": "string",
                    "enum": ["typed", "fork"],
                    "description": "`typed` (default) builds a narrow prompt + filtered tools. `fork` replays the parent's exact prompt for prefix-cache reuse on the inference backend."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        // ── Argument extraction with back-compat ───────────────────────
        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("archetype").and_then(|v| v.as_str()))
            .unwrap_or("")
            .trim()
            .to_string();

        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        let context = args
            .get("context")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let skill_filter_override = args
            .get("skill_filter")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let category_filter_override = match args.get("category_filter").and_then(|v| v.as_str()) {
            Some("system") => Some(ToolCategory::System),
            Some("skill") => Some(ToolCategory::Skill),
            Some(other) => {
                return Ok(ToolResult::error(format!(
                    "spawn_subagent: unknown category_filter '{other}' (expected 'system' or 'skill')"
                )));
            }
            None => None,
        };

        let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("typed");

        // ── Validation ─────────────────────────────────────────────────
        if agent_id.is_empty() {
            return Ok(ToolResult::error(
                "spawn_subagent: `agent_id` (or legacy `archetype`) is required",
            ));
        }
        if prompt.is_empty() {
            return Ok(ToolResult::error("spawn_subagent: `prompt` is required"));
        }

        let registry = match AgentDefinitionRegistry::global() {
            Some(reg) => reg,
            None => {
                return Ok(ToolResult::error(
                    "spawn_subagent: AgentDefinitionRegistry has not been initialised. \
                     This usually means the core process started without calling \
                     AgentDefinitionRegistry::init_global at startup.",
                ));
            }
        };

        // Resolve `mode` against the definition. Explicit `mode` argument
        // wins; otherwise we infer from the definition itself.
        let lookup_id = if mode == "fork" {
            "fork"
        } else {
            agent_id.as_str()
        };
        let definition = match registry.get(lookup_id) {
            Some(def) => def,
            None => {
                let available: Vec<&str> = registry.list().iter().map(|d| d.id.as_str()).collect();
                return Ok(ToolResult::error(format!(
                    "spawn_subagent: unknown agent_id '{lookup_id}'. Available: {}",
                    available.join(", ")
                )));
            }
        };

        // ── Validate skill filter against the runtime if set ───────────
        if let Some(skill) = skill_filter_override.as_deref() {
            if let Err(err) = validate_skill_filter(skill) {
                return Ok(ToolResult::error(err));
            }
        }

        // ── Publish SubagentSpawned event ──────────────────────────────
        let parent_session = current_parent()
            .map(|p| p.session_id.clone())
            .unwrap_or_else(|| "standalone".into());
        let task_id = format!("sub-{}", uuid::Uuid::new_v4());

        publish_global(DomainEvent::SubagentSpawned {
            parent_session: parent_session.clone(),
            agent_id: definition.id.clone(),
            mode: mode.to_string(),
            task_id: task_id.clone(),
            prompt_chars: prompt.chars().count(),
        });

        // ── Run the sub-agent ──────────────────────────────────────────
        let options = SubagentRunOptions {
            skill_filter_override,
            category_filter_override,
            context,
            task_id: Some(task_id.clone()),
        };

        match run_subagent(definition, &prompt, options).await {
            Ok(outcome) => {
                publish_global(DomainEvent::SubagentCompleted {
                    parent_session,
                    task_id: outcome.task_id.clone(),
                    agent_id: outcome.agent_id.clone(),
                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                    output_chars: outcome.output.chars().count(),
                    iterations: outcome.iterations,
                });
                Ok(ToolResult::success(outcome.output))
            }
            Err(err) => {
                let message = err.to_string();
                publish_global(DomainEvent::SubagentFailed {
                    parent_session,
                    task_id,
                    agent_id: definition.id.clone(),
                    error: message.clone(),
                });
                // Surface as a non-fatal tool error so the parent model
                // can react and (e.g.) retry with different params.
                Ok(ToolResult::error(format!(
                    "spawn_subagent failed: {message}"
                )))
            }
        }
    }
}

/// Validate that the requested skill_filter matches a currently-loaded
/// skill in the global skill runtime, if a runtime is available. When
/// no runtime is available (e.g. tests, CLI), this is a no-op.
///
/// Public alias for use by `orchestrator_tools`.
pub fn validate_skill_filter_public(skill_id: &str) -> Result<(), String> {
    validate_skill_filter(skill_id)
}

fn validate_skill_filter(skill_id: &str) -> Result<(), String> {
    let Some(engine) = crate::openhuman::skills::qjs_engine::global_engine() else {
        // No runtime registered — skip validation.
        return Ok(());
    };
    // `engine.all_tools()` returns `(skill_id, ToolDefinition)` pairs
    // where `skill_id` is the skill prefix (e.g. "gmail", "notion")
    // and `ToolDefinition.name` is the raw tool name (e.g. "get-emails").
    let mut known: Vec<String> = engine
        .all_tools()
        .into_iter()
        .map(|(skill_id, _)| skill_id)
        .collect();
    known.sort();
    known.dedup();
    if known.iter().any(|s| s == skill_id) {
        Ok(())
    } else {
        Err(format!(
            "skill_filter '{skill_id}' does not match any installed skill. Available: {}",
            known.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_agent_id_returns_error() {
        let tool = SpawnSubagentTool;
        let result = tool
            .execute(json!({
                "prompt": "do thing"
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("agent_id"));
    }

    #[tokio::test]
    async fn missing_prompt_returns_error() {
        let tool = SpawnSubagentTool;
        let result = tool
            .execute(json!({
                "agent_id": "researcher"
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("prompt"));
    }

    #[tokio::test]
    async fn no_registry_returns_clear_error() {
        // The global registry has not been initialised in this test.
        let tool = SpawnSubagentTool;
        let result = tool
            .execute(json!({
                "agent_id": "researcher",
                "prompt": "find x",
            }))
            .await
            .unwrap();
        // Either: registry uninitialised → clear init error, OR
        // registry was initialised by a previous test → "no parent context"
        // because we're not running inside an Agent::turn. Both are
        // acceptable: the tool gracefully refuses.
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn unknown_agent_id_lists_available() {
        // Force-init the global registry with builtins.
        let _ = AgentDefinitionRegistry::init_global_builtins();
        let tool = SpawnSubagentTool;
        let result = tool
            .execute(json!({
                "agent_id": "totally_made_up",
                "prompt": "x",
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        let out = result.output();
        // Should list at least one valid built-in.
        assert!(out.contains("code_executor") || out.contains("researcher"));
    }
}
