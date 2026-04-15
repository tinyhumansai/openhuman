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
use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{run_subagent, SubagentRunOptions};
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
        "Delegate a task to a specialised sub-agent. See the Delegation \
         Guide in the system prompt for available agent_ids and when to \
         use each. When delegating to `skills_agent`, you MUST also pass \
         `toolkit=\"<name>\"` naming the Composio integration the \
         sub-task targets (e.g. `gmail`, `notion`); the sub-agent will \
         only see that toolkit's actions."
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
                "category_filter": {
                    "type": "string",
                    "enum": ["system", "skill"],
                    "description": "Optional tool-category restriction. `skill` scopes the sub-agent to integration tools (for example Composio-backed SaaS actions); `system` scopes it to built-in Rust tools. Overrides the definition's `category_filter` for this single spawn."
                },
                "toolkit": {
                    "type": "string",
                    "description": "Composio toolkit slug to scope this spawn to — e.g. `gmail`, `notion`, `slack`. REQUIRED when `agent_id = \"skills_agent\"`. Narrows the sub-agent's visible Composio actions AND its Connected Integrations prompt section to only that toolkit's catalogue, so the sub-agent's context window only carries the platform it was asked to operate on. Must match a currently-connected integration (see the Delegation Guide)."
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

        let toolkit_override = args
            .get("toolkit")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

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

        // ── skills_agent toolkit gate ──────────────────────────────────
        // skills_agent is a platform-parameterised specialist. Every
        // spawn MUST name a CONNECTED toolkit so the sub-agent only
        // sees one integration's tool catalogue instead of all of
        // them. We split validation into three cases so the model
        // gets a precise, actionable error on every failure mode —
        // nothing reaches the LLM loop unless the spawn is valid.
        if definition.id == "skills_agent" {
            let parent_ctx = current_parent();
            let allowlist: Vec<&crate::openhuman::context::prompt::ConnectedIntegration> =
                parent_ctx
                    .as_ref()
                    .map(|p| p.connected_integrations.iter().collect())
                    .unwrap_or_default();
            let connected_slugs: Vec<String> = allowlist
                .iter()
                .filter(|ci| ci.connected)
                .map(|ci| ci.toolkit.clone())
                .collect();

            match toolkit_override.as_deref() {
                None => {
                    return Ok(ToolResult::error(format!(
                        "spawn_subagent(skills_agent): the `toolkit` argument is required. \
                         Pass one of the currently-connected toolkits: [{}]. \
                         See the Delegation Guide in your system prompt for which toolkit \
                         matches each task.",
                        connected_slugs.join(", ")
                    )));
                }
                Some(tk) => {
                    let entry = allowlist
                        .iter()
                        .find(|ci| ci.toolkit.eq_ignore_ascii_case(tk));
                    match entry {
                        None => {
                            // Toolkit isn't even in the backend allowlist.
                            return Ok(ToolResult::error(format!(
                                "spawn_subagent(skills_agent): toolkit '{tk}' is not in \
                                 the backend allowlist. Valid toolkits: [{}]. Check the \
                                 Delegation Guide in your system prompt for the exact slug.",
                                allowlist
                                    .iter()
                                    .map(|ci| ci.toolkit.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )));
                        }
                        Some(ci) if !ci.connected => {
                            // Toolkit exists in the allowlist but isn't connected.
                            // This is NOT a tool error — it's an expected condition
                            // the orchestrator should communicate to the user. We
                            // return `ToolResult::success` so:
                            //   1. The agent loop doesn't prepend "Error: " to
                            //      the result text (which would bias the model
                            //      toward defensive failure language).
                            //   2. The web channel emits `success: true` on the
                            //      `tool_result` socket event, so the frontend
                            //      doesn't render this as a failed tool call.
                            // The model still reads the explanation and produces
                            // an appropriate user-facing response.
                            return Ok(ToolResult::success(format!(
                                "Integration '{tk}' is available but the user has not \
                                 authorized it yet. Do NOT retry this spawn. Tell the user \
                                 the integration is available and ask them to authorize \
                                 '{tk}' in Settings → Integrations before retrying the \
                                 original request."
                            )));
                        }
                        Some(_) => {
                            // Connected — fall through to spawn.
                        }
                    }
                }
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
            skill_filter_override: None,
            category_filter_override,
            toolkit_override,
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
