//! Tool: `call_memory_agent` — invoke the memory retrieval agent to walk
//! the memory tree and return context for a query.
//!
//! Unlike the lower-level `memory_tree` tool (whose `walk`/`smart_walk` modes
//! now run the deterministic E2GraphRAG retriever and the other modes are
//! individual retrieval primitives), this tool spawns the full `agent_memory`
//! sub-agent which decides which retrieval strategies to combine and returns a
//! synthesised, cited answer.
//!
//! Supports both sync (blocking) and async (fire-and-forget) modes.

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentRunOptions, SubagentRunStatus,
};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult, ToolScope};
use async_trait::async_trait;
use serde_json::json;

const AGENT_ID: &str = "agent_memory";

pub struct CallMemoryAgentTool;

impl CallMemoryAgentTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CallMemoryAgentTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CallMemoryAgentTool {
    fn name(&self) -> &str {
        "call_memory_agent"
    }

    fn description(&self) -> &str {
        "Invoke the memory retrieval agent to walk the memory tree and \
         return relevant context for a query. The agent autonomously \
         combines vector search, keyword matching, entity lookup, and \
         tree browsing — then returns a cited answer. Use this when you \
         need a comprehensive memory search rather than a single-strategy \
         lookup."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language query to search the user's memory for."
                },
                "context": {
                    "type": "string",
                    "description": "Optional context to help the memory agent understand what you're looking for and why."
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Max retrieval turns the memory agent can take. Default: 15, hard cap: 20.",
                    "minimum": 1,
                    "maximum": 20
                },
                "async": {
                    "type": "boolean",
                    "description": "If true, fire-and-forget — the agent runs in the background and results are not returned inline. Default: false (synchronous)."
                }
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn scope(&self) -> ToolScope {
        ToolScope::AgentOnly
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("call_memory_agent: `query` is required"))?;

        let context = args.get("context").and_then(|v| v.as_str());

        let max_turns = args
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .map(|v| v.max(1).min(20) as usize);

        let is_async = args.get("async").and_then(|v| v.as_bool()).unwrap_or(false);

        let parent = current_parent();
        if parent.is_none() {
            return Ok(ToolResult::error(
                "call_memory_agent: no parent agent context — this tool must be \
                 called from within an agent turn."
                    .to_string(),
            ));
        }

        let registry = AgentDefinitionRegistry::global()
            .ok_or_else(|| anyhow::anyhow!("call_memory_agent: agent registry not initialised"))?;

        let definition = registry
            .list()
            .iter()
            .find(|d| d.id == AGENT_ID)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "call_memory_agent: agent definition '{AGENT_ID}' not found in registry"
                )
            })?;

        let parent = parent.expect("checked above");
        if !parent.allowed_subagent_ids.contains(AGENT_ID) {
            log::warn!(
                "[call_memory_agent] blocked memory subagent outside parent allowlist parent_agent={} requested_agent={} allowed={:?}",
                parent.agent_definition_id,
                AGENT_ID,
                parent.allowed_subagent_ids
            );
            return Ok(ToolResult::error(format!(
                "call_memory_agent: agent '{AGENT_ID}' is not in parent agent '{}' subagents.allowlist",
                parent.agent_definition_id
            )));
        }

        let mut prompt = format!(
            "Search the user's memory tree and return relevant context for this query:\n\n{query}"
        );
        if let Some(turns) = max_turns {
            prompt.push_str(&format!(
                "\n\nConstraint: use at most {turns} retrieval turns."
            ));
        }
        if let Some(ctx) = context {
            prompt.push_str(&format!("\n\nAdditional context:\n{ctx}"));
        }

        let task_id = format!("mem-{}", uuid::Uuid::new_v4());

        log::debug!(
            "[call_memory_agent] query_len={} async={} task_id={}",
            query.len(),
            is_async,
            task_id
        );

        let options = SubagentRunOptions {
            task_id: Some(task_id.clone()),
            ..Default::default()
        };

        if is_async {
            let def = definition.clone();
            let prompt_clone = prompt.clone();
            let tid = task_id.clone();
            tokio::spawn(async move {
                match run_subagent(&def, &prompt_clone, options).await {
                    Ok(outcome) => {
                        log::info!(
                            "[call_memory_agent] async task_id={} completed iterations={} elapsed={:?}",
                            tid,
                            outcome.iterations,
                            outcome.elapsed
                        );
                    }
                    Err(e) => {
                        log::warn!("[call_memory_agent] async task_id={} failed: {e:#}", tid);
                    }
                }
            });

            return Ok(ToolResult::success(format!(
                "Memory agent dispatched asynchronously (task_id: {task_id}). \
                 Results will be available when the agent completes."
            )));
        }

        // Synchronous path — block until the memory agent finishes.
        let started = std::time::Instant::now();
        match run_subagent(&definition, &prompt, options).await {
            Ok(outcome) => {
                let elapsed = started.elapsed();
                log::info!(
                    "[call_memory_agent] task_id={} completed iterations={} elapsed={:?} status={:?}",
                    task_id,
                    outcome.iterations,
                    elapsed,
                    outcome.status
                );

                let mut result = outcome.output;

                match outcome.status {
                    SubagentRunStatus::Completed => {}
                    SubagentRunStatus::AwaitingUser { question, .. } => {
                        result.push_str(&format!(
                            "\n\n⚠️ The memory agent needs clarification: {question}"
                        ));
                    }
                }

                result.push_str(&format!(
                    "\n\n---\n_Memory agent: {} iterations, {:.1}s_",
                    outcome.iterations,
                    elapsed.as_secs_f64()
                ));

                Ok(ToolResult::success(result))
            }
            Err(e) => {
                log::warn!("[call_memory_agent] task_id={} failed: {e:#}", task_id);
                Ok(ToolResult::error(format!("Memory agent failed: {e}")))
            }
        }
    }
}
