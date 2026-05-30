//! Tool: `delegate_to_personality` — delegate a task to a personality-specific agent.

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{run_subagent, SubagentRunOptions};
use crate::openhuman::agent::personality_paths::PersonalityContext;
use crate::openhuman::agent::profiles::AgentProfileStore;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct DelegateToPersonalityTool;

impl Default for DelegateToPersonalityTool {
    fn default() -> Self {
        Self::new()
    }
}

impl DelegateToPersonalityTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for DelegateToPersonalityTool {
    fn name(&self) -> &str {
        "delegate_to_personality"
    }

    fn description(&self) -> &str {
        "Delegate a task to another personality agent. Each personality has its own \
         memory, identity (SOUL.md), and integration access. Use when the task aligns \
         with a specific personality's expertise or context. The personality roster \
         in the system prompt lists available personalities."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["personality_id", "prompt"],
            "properties": {
                "personality_id": {
                    "type": "string",
                    "description": "The profile id of the target personality (from the personality roster)."
                },
                "prompt": {
                    "type": "string",
                    "description": "Clear, specific instruction for the personality agent. Include all context needed — the personality has its own memory but no awareness of the current conversation."
                },
                "context": {
                    "type": "string",
                    "description": "Optional context blob from prior task results."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let personality_id = args
            .get("personality_id")
            .and_then(|v| v.as_str())
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

        if personality_id.is_empty() {
            return Ok(ToolResult::error(
                "delegate_to_personality: `personality_id` is required.",
            ));
        }
        if prompt.is_empty() {
            return Ok(ToolResult::error(
                "delegate_to_personality: `prompt` is required.",
            ));
        }

        let parent_ctx = match current_parent() {
            Some(ctx) => ctx,
            None => {
                return Ok(ToolResult::error(
                    "delegate_to_personality: no parent execution context available.",
                ));
            }
        };

        tracing::debug!(
            personality_id = %personality_id,
            "[delegate_to_personality] resolving personality profile"
        );

        let store = AgentProfileStore::new(parent_ctx.workspace_dir.clone());

        // Guard: only the master personality may delegate. This prevents a
        // delegated sub-agent from recursing back through this tool — there is
        // no built-in depth counter on cross-personality delegation, so we gate
        // at the entry point.
        let (state, active_profile) = match store.resolve(None) {
            Ok(result) => result,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "delegate_to_personality: failed to resolve active profile: {e}"
                )));
            }
        };
        if !active_profile.is_master {
            tracing::debug!(
                active_profile_id = %active_profile.id,
                "[delegate_to_personality] rejected — caller is not the master personality"
            );
            return Ok(ToolResult::error(format!(
                "delegate_to_personality: only the master personality may delegate \
                 (active='{}', is_master=false).",
                active_profile.id
            )));
        }

        // Reuse the state already loaded above — no second store.resolve() call needed.
        let profile = match state.profiles.iter().find(|p| p.id == personality_id) {
            Some(p) => p.clone(),
            None => {
                tracing::debug!(
                    personality_id = %personality_id,
                    "[delegate_to_personality] profile resolve failed"
                );
                return Ok(ToolResult::error(format!(
                    "delegate_to_personality: personality '{personality_id}' not found."
                )));
            }
        };

        let personality_ctx =
            PersonalityContext::from_profile(&parent_ctx.workspace_dir, profile.clone());

        tracing::debug!(
            personality_id = %personality_id,
            agent_id = %profile.agent_id,
            memory_suffix = %personality_ctx.memory_suffix,
            "[delegate_to_personality] personality resolved, delegating"
        );

        // TODO(phase-2): Memory isolation not yet enforced during delegation.
        // The personality gets its own SQLite DB (plumbing exists in
        // UnifiedMemory::new_with_memory_dir), but run_subagent currently
        // receives the parent's memory instance. To fix: construct a new
        // UnifiedMemory using personality_ctx.memory_suffix and pass it through
        // SubagentRunOptions (needs an additional field there).
        // Also: composio_integrations allowlist is not yet filtered during
        // delegation — personality_ctx.composio_allowlist exists but is not
        // applied to the SubagentRunOptions toolkit_override.
        //
        // Until these are wired, the sub-agent gets the personality's
        // voice/identity at the prompt level but still writes to the parent's
        // memory store and has access to all parent integrations.
        let mut personality_preamble = format!(
            "You are acting as the personality `{}` (\"{}\"). {}",
            profile.id, profile.name, profile.description
        );
        if let Some(ref soul) = personality_ctx.soul_md_override {
            let soul_truncated: String = soul.chars().take(800).collect();
            personality_preamble.push_str("\n\n[Personality SOUL.md]\n");
            personality_preamble.push_str(&soul_truncated);
        }
        if let Some(ref mem) = personality_ctx.memory_md_override {
            let mem_truncated: String = mem.chars().take(800).collect();
            personality_preamble.push_str("\n\n[Personality MEMORY.md]\n");
            personality_preamble.push_str(&mem_truncated);
        }
        let combined_context = match context.as_deref() {
            Some(ctx_str) => format!("{personality_preamble}\n\n[Caller Context]\n{ctx_str}"),
            None => personality_preamble,
        };

        // Look up the agent definition for this personality's agent_id
        let registry = match AgentDefinitionRegistry::global() {
            Some(reg) => reg,
            None => {
                return Ok(ToolResult::error(
                    "delegate_to_personality: agent definition registry not initialized.",
                ));
            }
        };

        let definition = match registry.get(&profile.agent_id) {
            Some(def) => def,
            None => {
                return Ok(ToolResult::error(format!(
                    "delegate_to_personality: agent definition '{}' not found in registry.",
                    profile.agent_id
                )));
            }
        };

        let options = SubagentRunOptions {
            context: Some(combined_context),
            model_override: profile.model_override.clone(),
            toolkit_override: None,
            skill_filter_override: None,
            task_id: None,
            worker_thread_id: None,
        };

        match run_subagent(&definition, &prompt, options).await {
            Ok(outcome) => {
                tracing::debug!(
                    personality_id = %personality_id,
                    iterations = outcome.iterations,
                    elapsed_ms = outcome.elapsed.as_millis(),
                    "[delegate_to_personality] delegation completed"
                );
                Ok(ToolResult::success(format!(
                    "[Personality: {}]\n{}",
                    profile.name, outcome.output
                )))
            }
            Err(e) => {
                tracing::debug!(
                    personality_id = %personality_id,
                    error = %e,
                    "[delegate_to_personality] delegation failed"
                );
                Ok(ToolResult::error(format!(
                    "delegate_to_personality failed for '{}': {e}",
                    personality_id
                )))
            }
        }
    }
}
