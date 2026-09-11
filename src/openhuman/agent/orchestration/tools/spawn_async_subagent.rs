//! Tool: `spawn_async_subagent` - fire-and-forget sub-agent delegation.
//!
//! Unlike `spawn_subagent`, this tool returns as soon as the child run is
//! accepted. Completion/failure is reported through normal sub-agent lifecycle
//! events and, when possible, persisted in the child worker thread.

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::{current_parent, with_parent_context};
use crate::openhuman::agent::harness::run_queue::RunQueue;
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentRunOptions, SubagentRunStatus,
};
use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::agent::orchestration::running_subagents::{self, SubagentStatus};
use crate::openhuman::agent::orchestration::subagent_sessions::{
    self, DurableSubagentStatus, SubagentSessionSelector, SubagentSessionStore,
    SubagentSessionUpsert,
};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::memory::conversations::{self as conversations, ConversationMessage};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use tinytools::ToolRunContext;

pub struct SpawnAsyncSubagentTool;

impl SpawnAsyncSubagentTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SpawnAsyncSubagentTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SpawnAsyncSubagentTool {
    fn name(&self) -> &str {
        "spawn_async_subagent"
    }

    fn description(&self) -> &str {
        "Fire-and-forget a sub-agent for low-attention background work the user does not need in this reply (archiving, cleanup, background investigation). Returns immediately, so never use it for user-visible answers, writes, financial actions, or anything whose result must gate your final answer."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let agent_ids: Vec<String> = AgentDefinitionRegistry::global()
            .map(|reg| reg.list().iter().map(|d| d.id.clone()).collect())
            .unwrap_or_default();

        let agent_id_schema = if agent_ids.is_empty() {
            json!({
                "type": "string",
                "description": "Sub-agent id (e.g. archivist, researcher, tools_agent)."
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
                "prompt": {
                    "type": "string",
                    "description": "Clear, self-contained background instruction. Include all context needed. The sub-agent must not ask the user for clarification."
                },
                "context": {
                    "type": "string",
                    "description": "Optional context blob from prior task results. Rendered as a `[Context]` block before the prompt."
                },
                "model": {
                    "type": "string",
                    "description": "Optional exact model id for this background spawn only."
                },
                "toolkit": {
                    "type": "string",
                    "description": "Composio toolkit slug to scope this spawn to. Required when agent_id is `integrations_agent`."
                },
                "task_title": {
                    "type": "string",
                    "description": "Optional short title for the persisted background worker thread."
                },
                "task_key": {
                    "type": "string",
                    "description": "Optional deterministic identity key for reusable delegation. Defaults to a normalized task_title/prompt."
                },
                "fresh": {
                    "type": "boolean",
                    "description": "When true, bypass reusable subagent matching and create a fresh durable worker."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        options: ToolCallOptions,
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.execute_with_context_inner(args, options, tool_context)
            .await
    }
}

include!("spawn_async_subagent_execute.rs");

/// Format the user-facing acceptance text around a structured async sub-agent reference.
fn format_async_subagent_accepted(agent_id: &str, payload_json: &str) -> String {
    format!(
        "Accepted async sub-agent `{agent_id}`. Use the structured reference below to send more input, \
         wait for completion, or perform a short timeout tick to check status. If the user does not need \
         the result now, continue without blocking.\n\n[async_subagent_ref]\n{payload_json}\n[/async_subagent_ref]"
    )
}

/// Build the machine-readable reference the orchestrator uses to steer, wait, or poll a worker.
fn async_subagent_ref_payload(
    task_id: &str,
    subagent_session_id: &str,
    agent_id: &str,
    worker_thread_id: Option<&str>,
    reused: bool,
    reuse_decision: &str,
    status: &str,
) -> serde_json::Value {
    json!({
        "task_id": task_id,
        "taskId": task_id,
        "subagent_session_id": subagent_session_id,
        "subagentSessionId": subagent_session_id,
        "agent_id": agent_id,
        "agentId": agent_id,
        "mode": "async",
        "status": status,
        "worker_thread_id": worker_thread_id,
        "workerThreadId": worker_thread_id,
        "reused": reused,
        "reuse_decision": reuse_decision,
        "reuseDecision": reuse_decision,
        "instructions": {
            "send_message": {
                "tool": "steer_subagent",
                "description": "Send additional instructions or context to this running async sub-agent.",
                "arguments": {
                    "subagent_session_id": subagent_session_id,
                    "message": "<message>",
                    "mode": "steer"
                }
            },
            "wait": {
                "tool": "wait_subagent",
                "description": "Block until the async sub-agent finishes, up to the timeout.",
                "arguments": {
                    "subagent_session_id": subagent_session_id,
                    "timeout_secs": 120
                }
            },
            "timeout_tick": {
                "tool": "wait_subagent",
                "description": "Perform a short status tick without committing the parent to a long wait.",
                "arguments": {
                    "subagent_session_id": subagent_session_id,
                    "timeout_secs": 1
                }
            },
            "delayed_tick": {
                "tool": "wait",
                "description": "Trigger a delayed callback before checking this async sub-agent again.",
                "arguments": {
                    "duration_secs": 30,
                    "message": format!("Check async sub-agent {agent_id} status with wait_subagent using subagent_session_id {subagent_session_id}.")
                }
            },
            "delayed_loop": {
                "tool": "wait_loop",
                "description": "Trigger repeatable delayed callbacks while this async sub-agent is still relevant.",
                "arguments": {
                    "duration_secs": 30,
                    "message": format!("Check async sub-agent {agent_id} status with wait_subagent using subagent_session_id {subagent_session_id}."),
                    "loop_key": subagent_session_id,
                    "iteration": 1
                }
            }
        },
        "next_actions": [
            "call steer_subagent to send more input",
            "call wait_subagent with timeout_secs to collect the result",
            "call wait_subagent with timeout_secs=1 as a timeout tick/status check",
            "call wait or wait_loop with the returned message to trigger a delayed status check",
            "continue without waiting when the current user reply does not depend on the result"
        ]
    })
}

fn add_background_contract(prompt: &str) -> String {
    format!(
        "[Background Contract]\n\
         Run this task without requiring attention from the parent or user. \
         Do not call ask_user_clarification. If required information is missing, \
         make the safest best-effort progress and record the limitation in your final output.\n\n\
         [Task]\n{prompt}"
    )
}

fn durable_task_key_source(
    args: &serde_json::Value,
    prompt: &str,
    context: Option<&str>,
) -> String {
    if let Some(task_key) = args
        .get("task_key")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return task_key.to_string();
    }

    match context.map(str::trim).filter(|s| !s.is_empty()) {
        Some(context) => format!("{prompt}\n\n[Context]\n{context}"),
        None => prompt.to_string(),
    }
}

/// Scan a finished child's history for the LAST `workflow_proposal` tool
/// result (the workflow_builder's `propose_workflow` / `revise_workflow` /
/// `edit_workflow` all return `{"type":"workflow_proposal", ...}` JSON).
/// Returns the parsed payload, or `None` when the run produced no proposal.
/// Lives here (not in `flows`) so the always-on orchestration path has no
/// dependency on the feature-gated flows domain — it is a generic scan for a
/// structured tool payload.
pub(crate) fn extract_workflow_proposal_from_history(
    history: &[ChatMessage],
) -> Option<serde_json::Value> {
    history
        .iter()
        .rev()
        .filter(|message| message.role == "tool")
        .find_map(|message| {
            let value: serde_json::Value = serde_json::from_str(message.content.trim()).ok()?;
            (value.get("type").and_then(|t| t.as_str()) == Some("workflow_proposal"))
                .then_some(value)
        })
}

/// Durably surface a workflow proposal found in a finished child's history:
/// persist it as a parent-thread conversation message (metadata carries the
/// full payload so the UI can rehydrate the proposal card after reload) and
/// append a `[workflow_proposal]` envelope to the delivery summary so the
/// follow-up turn presents it faithfully. Returns the (possibly extended)
/// summary; on any persistence error the summary still carries the envelope —
/// losing durability must not lose delivery.
fn attach_workflow_proposal(
    workspace_dir: &std::path::Path,
    parent_thread_id: Option<&str>,
    task_id: &str,
    agent_id: &str,
    final_history: &[ChatMessage],
    summary: String,
) -> String {
    let Some(proposal) = extract_workflow_proposal_from_history(final_history) else {
        return summary;
    };
    let proposal_json = match serde_json::to_string(&proposal) {
        Ok(json) => json,
        Err(err) => {
            log::warn!(
                "[spawn_async_subagent] workflow proposal re-serialize failed task_id={task_id} error={err}"
            );
            return summary;
        }
    };
    let name = proposal
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("Untitled workflow");
    log::info!(
        "[spawn_async_subagent] extracted workflow proposal '{name}' task_id={task_id} \
         ({} chars) — persisting to parent thread {:?}",
        proposal_json.len(),
        parent_thread_id
    );
    if let Some(thread_id) = parent_thread_id {
        let persisted = conversations::append_message(
            workspace_dir.to_path_buf(),
            thread_id,
            ConversationMessage {
                id: format!("workflow-proposal:{task_id}"),
                content: format!("Workflow proposal ready: {name}"),
                message_type: "text".to_string(),
                extra_metadata: json!({
                    "scope": "workflow_proposal",
                    "proposal": proposal,
                    "task_id": task_id,
                    "agent_id": agent_id,
                }),
                sender: "agent".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        );
        if let Err(err) = persisted {
            log::warn!(
                "[spawn_async_subagent] workflow proposal persistence failed \
                 thread_id={thread_id} task_id={task_id} error={err} — proposal still \
                 rides the delivery notice"
            );
        }
    }
    format!(
        "{summary}\n\n[workflow_proposal]\n{proposal_json}\n[/workflow_proposal]\n\
         (The full proposal above was also saved to the chat thread; present it to the \
         user for review — do not re-run the builder unless they ask for changes.)"
    )
}

fn reusable_follow_up_message(prompt: &str, context: Option<&str>) -> String {
    let mut message = String::from("[Follow-up instruction for reusable sub-agent]\n");
    if let Some(context) = context.map(str::trim).filter(|s| !s.is_empty()) {
        message.push_str("\n[Context]\n");
        message.push_str(context);
        message.push_str("\n\n");
    }
    message.push_str("[Task]\n");
    message.push_str(prompt);
    message
}

#[cfg(test)]
#[path = "spawn_async_subagent_tests.rs"]
mod tests;
