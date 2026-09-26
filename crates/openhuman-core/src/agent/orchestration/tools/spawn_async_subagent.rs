//! Tool: `spawn_async_subagent` - fire-and-forget sub-agent delegation.
//!
//! Unlike `spawn_subagent`, this tool returns as soon as the child run is
//! accepted. Completion/failure is reported through normal sub-agent lifecycle
//! events and, when possible, persisted in the child worker thread.

use super::subagent_abort_report::AbortReport;
use crate::agent::harness::definition::AgentDefinitionRegistry;
use crate::agent::messages::ChatMessage;
use crate::agent::orchestration::fleet_tools::FleetToolSet;
use crate::agent::orchestration::running_subagents::{self, SubagentStatus};
use crate::agent::orchestration::subagent_sessions::{
    self, DurableSubagentStatus, SubagentSessionSelector, SubagentSessionStore,
    SubagentSessionUpsert,
};
use crate::agent::progress::AgentProgress;
use crate::agent::subagent_host::{
    run_subagent_with_parent, SubagentRunOptions, SubagentRunStatus,
};
use crate::memory::conversations::{self as conversations, ConversationMessage};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::run_queue::RunQueue;
use tinyagents_harness::tool::{ToolDispatch, ToolExecutionContext};
use tinytools::ToolRunContext;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult};

pub struct SpawnAsyncSubagentTool;

/// Harness dispatch for the detached child path. It owns the typed parent run
/// so the spawned child receives the caller's carrier before `tokio::spawn`.
pub(crate) struct SpawnAsyncSubagentDispatch {
    tool: Arc<dyn Tool>,
}

impl SpawnAsyncSubagentDispatch {
    pub(crate) fn new(tool: Arc<dyn Tool>) -> Self {
        Self { tool }
    }
}

#[async_trait]
impl ToolDispatch<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for SpawnAsyncSubagentDispatch
{
    fn tool(&self) -> Arc<dyn Tool> {
        self.tool.clone()
    }

    async fn execute(
        &self,
        _state: &(),
        _call_id: tinyagents_harness::CallId,
        arguments: serde_json::Value,
        _options: ToolCallOptions,
        parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let context = ToolExecutionContext::from_run_context(parent, _call_id.clone());
        let detached_data = parent.data.detached_child();
        let detached_cancellation = detached_data.cancellation.clone();
        let detached_parent = parent
            .child(
                RunConfig::new(format!("async-subagent-{}", uuid::Uuid::new_v4())),
                detached_data,
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))?
            .with_cancellation(detached_cancellation);
        SpawnAsyncSubagentTool::new()
            .execute_with_live_parent_context(
                arguments,
                Some(&context),
                parent.data.child(),
                detached_parent,
            )
            .await
    }
}

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

/// Narrow a session's `spawn_async_subagent` schema to the ids the parent may
/// actually dispatch.
///
/// The tool is registered once per process, so its `agent_id` enum is built
/// from the whole registry: 30-odd ids, of which the orchestrator's
/// `[subagents]` allowlist admits about twenty. `execute` already refuses the
/// rest, so advertising them only bought a refused call and a slice of schema
/// on every turn. Called from the per-session spec view
/// (`builder::visible_tool_specs_for_policy`), the same place `use_skill`'s
/// pack index is narrowed. A missing or empty allowlist leaves the spec alone:
/// wildcard parents keep the full registry.
pub fn scope_spawn_async_subagent_spec(spec: &mut tinytools::ToolSpec, allowed: &[String]) {
    if allowed.is_empty() {
        return;
    }
    let Some(enum_slot) = spec
        .parameters
        .pointer_mut("/properties/agent_id/enum")
        .filter(|value| value.is_array())
    else {
        return;
    };
    let mut ids: Vec<String> = allowed.to_vec();
    ids.sort();
    ids.dedup();
    *enum_slot = serde_json::Value::Array(ids.into_iter().map(serde_json::Value::String).collect());
    if let Some(description) = spec
        .parameters
        .pointer_mut("/properties/agent_id/description")
    {
        *description = serde_json::Value::String(
            "Sub-agent id (only these are dispatchable from here).".to_string(),
        );
    }
}

#[async_trait]
impl Tool for SpawnAsyncSubagentTool {
    fn name(&self) -> &str {
        "spawn_async_subagent"
    }

    fn description(&self) -> &str {
        "Fire-and-forget a sub-agent for background work this reply does not depend on \
         (archiving, cleanup, background investigation). Returns immediately; never for \
         user-visible answers, writes, financial actions, or anything that gates your reply."
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
                    "description": "Self-contained instruction with all needed context; the worker cannot ask the user."
                },
                "context": {
                    "type": "string",
                    "description": "Optional prior results, rendered as a `[Context]` block before the prompt."
                },
                "model": {
                    "type": "string",
                    "description": "Optional exact model id for this spawn only."
                },
                "toolkit": {
                    "type": "string",
                    "description": "Composio toolkit slug; required when agent_id is `integrations_agent`."
                },
                "task_title": {
                    "type": "string",
                    "description": "Optional short title for the worker thread."
                },
                "task_key": {
                    "type": "string",
                    "description": "Optional identity key for reusing an existing worker."
                },
                "fresh": {
                    "type": "boolean",
                    "description": "Force a fresh worker instead of reusing a matching one."
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
        if let Some(live_parent) = super::ambient_parent_run_context("direct-async-subagent") {
            let detached_data = live_parent.data.detached_child();
            let detached_cancellation = detached_data.cancellation.clone();
            let detached_parent = live_parent
                .child(
                    RunConfig::new(format!("async-subagent-{}", uuid::Uuid::new_v4())),
                    detached_data,
                )
                .map_err(|error| anyhow::anyhow!(error.to_string()))?
                .with_cancellation(detached_cancellation);
            return self
                .execute_with_live_parent_context(
                    args,
                    tool_context,
                    live_parent.data.child(),
                    detached_parent,
                )
                .await;
        }
        self.execute_with_context_inner(
            args,
            options,
            tool_context,
            crate::agent::tinyagents::host::OpenHumanRunContext::new(),
            None,
        )
        .await
    }
}

impl SpawnAsyncSubagentTool {
    pub(crate) async fn execute_with_live_parent_context(
        &self,
        args: serde_json::Value,
        tool_context: Option<&dyn ToolRunContext>,
        run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
        detached_parent: RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.execute_with_context_inner(
            args,
            ToolCallOptions::default(),
            tool_context,
            run_context,
            Some(detached_parent),
        )
        .await
    }
}

include!("spawn_async_subagent_execute.rs");
include!("spawn_async_subagent_args.rs");

/// Format the user-facing acceptance text around a structured async sub-agent reference.
///
/// The wording follows what the parent can actually do: a parent without
/// `wait_subagent` (the orchestrator, #5701) is told the result arrives on its
/// own and not to poll, instead of being invited to "wait for completion".
fn format_async_subagent_accepted(
    agent_id: &str,
    payload_json: &str,
    fleet: &FleetToolSet,
) -> String {
    // Steering and waiting are independent fleet capabilities: a parent can
    // have `wait_subagent` without `steer_subagent` (or vice versa), so the
    // guidance text is built from each independently rather than gated
    // entirely on `can_wait()` — otherwise a wait-only parent is told to
    // "send more input" through a tool it does not have.
    let can_send = fleet.has("steer_subagent");
    let can_wait = fleet.can_wait();
    let guidance = match (can_send, can_wait) {
        (true, true) => {
            "Use the structured reference below to send more input, wait for completion, or perform a          short timeout tick to check status. If the user does not need the result now, continue          without blocking."
        }
        (true, false) => {
            "Use the structured reference below to send more input if needed. You cannot and need not          wait or poll for it (no shell/sleep, no fake status checks); its result is delivered to          you automatically on a later turn. If the user does not need the result now, continue          without blocking."
        }
        (false, true) => {
            "Use the structured reference below to wait for completion or perform a short timeout tick          to check status. If the user does not need the result now, continue without blocking."
        }
        (false, false) => {
            "Its result is delivered to you automatically on a later turn — you cannot and need not          wait or poll for it (no shell/sleep, no fake status checks). Reply to the user now with          what you know, say the result is on its way, and continue. The structured reference          below lists the only follow-up tools you have for this worker."
        }
    };
    format!(
        "Accepted async sub-agent `{agent_id}`. {guidance}

[async_subagent_ref]
{payload_json}
[/async_subagent_ref]"
    )
}

/// Build the machine-readable reference the orchestrator uses to follow up on a worker.
///
/// Only tools in `fleet` are offered: an instruction naming a tool the parent
/// cannot see costs an iteration of confused reasoning per delegation.
fn async_subagent_ref_payload(
    task_id: &str,
    subagent_session_id: &str,
    agent_id: &str,
    worker_thread_id: Option<&str>,
    reused: bool,
    reuse_decision: &str,
    status: &str,
    fleet: &FleetToolSet,
) -> serde_json::Value {
    let mut instructions = serde_json::Map::new();
    let mut next_actions: Vec<String> = Vec::new();

    if fleet.has("steer_subagent") {
        instructions.insert(
            "send_message".into(),
            json!({
                "tool": "steer_subagent",
                "description": "Send additional instructions or context to this running async sub-agent.",
                "arguments": {
                    "subagent_session_id": subagent_session_id,
                    "message": "<message>",
                    "mode": "steer"
                }
            }),
        );
        next_actions.push("call steer_subagent to send more input".into());
    }
    if fleet.has("wait_subagent") {
        instructions.insert(
            "wait".into(),
            json!({
                "tool": "wait_subagent",
                "description": "Block until the async sub-agent finishes, up to the timeout.",
                "arguments": { "subagent_session_id": subagent_session_id, "timeout_secs": 120 }
            }),
        );
        instructions.insert(
            "timeout_tick".into(),
            json!({
                "tool": "wait_subagent",
                "description": "Perform a short status tick without committing the parent to a long wait.",
                "arguments": { "subagent_session_id": subagent_session_id, "timeout_secs": 1 }
            }),
        );
        next_actions.push("call wait_subagent with timeout_secs to collect the result".into());
        next_actions
            .push("call wait_subagent with timeout_secs=1 as a timeout tick/status check".into());
        let reminder = format!(
            "Check async sub-agent {agent_id} status with wait_subagent using subagent_session_id {subagent_session_id}."
        );
        if fleet.has("wait") {
            instructions.insert(
                "delayed_tick".into(),
                json!({
                    "tool": "wait",
                    "description": "Trigger a delayed callback before checking this async sub-agent again.",
                    "arguments": { "duration_secs": 30, "message": reminder }
                }),
            );
        }
        if fleet.has("wait_loop") {
            instructions.insert(
                "delayed_loop".into(),
                json!({
                    "tool": "wait_loop",
                    "description": "Trigger repeatable delayed callbacks while this async sub-agent is still relevant.",
                    "arguments": {
                        "duration_secs": 30,
                        "message": reminder,
                        "loop_key": subagent_session_id,
                        "iteration": 1
                    }
                }),
            );
        }
        if fleet.has("wait") || fleet.has("wait_loop") {
            next_actions.push(
                "call wait or wait_loop with the returned message to trigger a delayed status check".into(),
            );
        }
    }
    if fleet.has("continue_subagent") {
        instructions.insert(
            "answer_or_resume".into(),
            json!({
                "tool": "continue_subagent",
                "description": "Answer this worker if it pauses on ask_user_clarification (awaiting_user), or resume it later with a follow-up that keeps its context.",
                "arguments": { "subagent_session_id": subagent_session_id, "message": "<answer or follow-up>" }
            }),
        );
        next_actions.push(
            "call continue_subagent only if this worker reports awaiting_user, or to resume it with a follow-up".into(),
        );
    }
    if fleet.has("list_subagents") {
        next_actions.push("call list_subagents to re-enumerate your workers if this reference scrolls out of context".into());
    }
    next_actions.push(if fleet.can_wait() {
        "continue without waiting when the current user reply does not depend on the result".into()
    } else {
        "continue now: the result is delivered to you automatically on a later turn; never poll for it".into()
    });

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
        "result_delivery": "automatic",
        "instructions": instructions,
        "next_actions": next_actions
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
