//! Sub-agent lifecycle handlers for [`super::spawn_progress_bridge`]'s
//! `AgentProgress` dispatch loop.
//!
//! Split out of `progress_bridge.rs` (pure mechanical extraction, no
//! behavior change) to keep that file under the layout ratchet's pinned
//! line-count limit. Each function here is exactly one `AgentProgress::Subagent*`
//! match arm's original body, taking as parameters whatever state the arm
//! read or mutated.

use std::collections::HashMap;

use serde_json::json;
use tinyagents_session::run_ledger::{
    AgentRunKind, AgentRunStatus, AgentRunUpsert, RunEventAppend, RunTelemetryUpsert,
};

use crate::web_chat::{SubagentProgressDetail, WebChannelEvent};

use super::{
    cap_wire_args, cap_wire_output, ledger_append_event, ledger_upsert_agent_run,
    ledger_upsert_telemetry, publish_seq_stamped, subagent_worktree_detail,
};

/// Bundles the progress bridge's per-turn identity fields (read-only for the
/// duration of one `AgentProgress` event) so the handlers below don't need a
/// four-parameter prefix on every call.
pub(super) struct BridgeCtx<'a> {
    pub(super) client_id: &'a str,
    pub(super) thread_id: &'a str,
    pub(super) request_id: &'a str,
    pub(super) config: &'a crate::config::Config,
    pub(super) dispatch: &'a crate::core::runtime::DispatchCtx,
}

impl<'a> BridgeCtx<'a> {
    pub(super) fn new(
        client_id: &'a str,
        thread_id: &'a str,
        request_id: &'a str,
        config: &'a crate::config::Config,
        dispatch: &'a crate::core::runtime::DispatchCtx,
    ) -> Self {
        Self {
            client_id,
            thread_id,
            request_id,
            config,
            dispatch,
        }
    }
}

pub(super) fn on_subagent_spawned(
    ctx: &BridgeCtx<'_>,
    emit_seq: &mut u64,
    subagent_parent_call_ids: &mut HashMap<String, Option<String>>,
    round: u32,
    agent_id: String,
    task_id: String,
    mode: String,
    dedicated_thread: bool,
    prompt_chars: usize,
    worker_thread_id: Option<String>,
    display_name: Option<String>,
    parent_call_id: Option<String>,
) {
    subagent_parent_call_ids.insert(task_id.clone(), parent_call_id.clone());
    let label = display_name.as_deref().unwrap_or(&agent_id);
    let kind = if worker_thread_id.is_some() {
        AgentRunKind::WorkerThread
    } else {
        AgentRunKind::Subagent
    };
    ledger_upsert_agent_run(
        ctx.config,
        AgentRunUpsert {
            id: task_id.clone(),
            kind,
            parent_run_id: Some(ctx.request_id.to_string()),
            parent_thread_id: Some(ctx.thread_id.to_string()),
            agent_id: Some(agent_id.clone()),
            status: AgentRunStatus::Running,
            prompt_ref: worker_thread_id
                .as_ref()
                .map(|id| format!("thread:{id}:message:seed")),
            worker_thread_id: worker_thread_id.clone(),
            checkpoint_path: None,
            checkpoint: None,
            summary: None,
            error: None,
            metadata: json!({
                "mode": mode,
                "dedicatedThread": dedicated_thread,
                "promptChars": prompt_chars,
                "displayName": display_name,
                "parentCallId": parent_call_id,
                "source": "agent_progress",
                "schemaVersion": 1
            }),
            started_at: None,
            completed_at: None,
        },
    );
    ledger_append_event(
        ctx.config,
        RunEventAppend {
            run_id: task_id.clone(),
            event_type: "subagent_spawned".to_string(),
            payload: json!({
                "agentId": agent_id,
                "parentRunId": ctx.request_id,
                "threadId": ctx.thread_id,
                "workerThreadId": worker_thread_id,
                "mode": mode,
                "dedicatedThread": dedicated_thread,
                "promptChars": prompt_chars,
                "displayName": display_name,
                "parentCallId": parent_call_id
            }),
        },
    );
    publish_seq_stamped(
        ctx.dispatch,
        emit_seq,
        WebChannelEvent {
            event: "subagent_spawned".to_string(),
            client_id: ctx.client_id.to_string(),
            thread_id: ctx.thread_id.to_string(),
            request_id: ctx.request_id.to_string(),
            message: Some(format!("Sub-agent '{label}' spawned")),
            tool_name: Some(agent_id),
            skill_id: Some(task_id),
            round: Some(round),
            subagent: Some(SubagentProgressDetail {
                mode: Some(mode),
                dedicated_thread: Some(dedicated_thread),
                prompt_chars: Some(prompt_chars as u64),
                worker_thread_id,
                display_name,
                parent_call_id,
                ..Default::default()
            }),
            ..Default::default()
        },
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn on_subagent_completed(
    ctx: &BridgeCtx<'_>,
    emit_seq: &mut u64,
    subagent_parent_call_ids: &mut HashMap<String, Option<String>>,
    child_tool_counts: &HashMap<String, u64>,
    round: u32,
    agent_id: String,
    task_id: String,
    elapsed_ms: u64,
    iterations: u32,
    output_chars: usize,
    output: String,
    usage: Option<crate::agent::subagent_host::SubagentUsage>,
    worktree_path: Option<String>,
    changed_files: Vec<String>,
    dirty_status: Option<bool>,
) {
    let parent_call_id = subagent_parent_call_ids.remove(&task_id).flatten();
    let capped_output = cap_wire_output(output);
    let completed_at = chrono::Utc::now();
    ledger_upsert_agent_run(
        ctx.config,
        AgentRunUpsert {
            id: task_id.clone(),
            kind: AgentRunKind::Subagent,
            parent_run_id: Some(ctx.request_id.to_string()),
            parent_thread_id: Some(ctx.thread_id.to_string()),
            agent_id: Some(agent_id.clone()),
            status: AgentRunStatus::Completed,
            prompt_ref: None,
            worker_thread_id: None,
            checkpoint_path: None,
            checkpoint: None,
            summary: Some(format!(
                "Completed in {iterations} iteration(s), {output_chars} output chars"
            )),
            error: None,
            metadata: json!({}),
            started_at: None,
            completed_at: Some(completed_at),
        },
    );
    ledger_upsert_telemetry(
        ctx.config,
        RunTelemetryUpsert {
            run_id: task_id.clone(),
            elapsed_ms: Some(elapsed_ms),
            tool_count: child_tool_counts.get(&task_id).copied(),
            ..Default::default()
        },
    );
    ledger_append_event(
        ctx.config,
        RunEventAppend {
            run_id: task_id.clone(),
            event_type: "subagent_completed".to_string(),
            payload: json!({
                "agentId": agent_id,
                "elapsedMs": elapsed_ms,
                "iterations": iterations,
                "outputChars": output_chars,
                "worktreePath": worktree_path,
                "changedFiles": changed_files,
                "dirtyStatus": dirty_status,
                "parentCallId": parent_call_id
            }),
        },
    );
    publish_seq_stamped(
        ctx.dispatch,
        emit_seq,
        WebChannelEvent {
            event: "subagent_completed".to_string(),
            client_id: ctx.client_id.to_string(),
            thread_id: ctx.thread_id.to_string(),
            request_id: ctx.request_id.to_string(),
            message: Some(format!(
                "Sub-agent '{agent_id}' completed in {elapsed_ms}ms"
            )),
            tool_name: Some(agent_id),
            skill_id: Some(task_id),
            success: Some(true),
            round: Some(round),
            subagent: Some(SubagentProgressDetail {
                elapsed_ms: Some(elapsed_ms),
                iterations: Some(iterations),
                output_chars: Some(output_chars as u64),
                output: Some(capped_output),
                parent_call_id,
                // Present only when this child's spend is NOT already in the
                // parent turn's totals — the emitting site decides, because
                // only it can see whether the usage reached
                // `parent_subagent_usage`. Absent is the safe default and
                // means "add nothing".
                input_tokens: usage.as_ref().map(|u| u.input_tokens),
                output_tokens: usage.as_ref().map(|u| u.output_tokens),
                cached_input_tokens: usage.as_ref().map(|u| u.cached_input_tokens),
                // `None` also when the child's cost is unknown (never a guess).
                cost_usd: usage.as_ref().and_then(|u| u.cost().usd()),
                // Worktree isolation metadata (#3376) — drives the inline
                // subagent worktree row's open/diff/remove actions. All
                // `None`/absent for non-isolated workers.
                ..subagent_worktree_detail(worktree_path, changed_files, dirty_status)
            }),
            ..Default::default()
        },
    );
}

pub(super) fn on_subagent_failed(
    ctx: &BridgeCtx<'_>,
    emit_seq: &mut u64,
    subagent_parent_call_ids: &mut HashMap<String, Option<String>>,
    child_tool_counts: &HashMap<String, u64>,
    round: u32,
    agent_id: String,
    task_id: String,
    error: String,
) {
    let parent_call_id = subagent_parent_call_ids.remove(&task_id).flatten();
    let completed_at = chrono::Utc::now();
    ledger_upsert_agent_run(
        ctx.config,
        AgentRunUpsert {
            id: task_id.clone(),
            kind: AgentRunKind::Subagent,
            parent_run_id: Some(ctx.request_id.to_string()),
            parent_thread_id: Some(ctx.thread_id.to_string()),
            agent_id: Some(agent_id.clone()),
            status: AgentRunStatus::Failed,
            prompt_ref: None,
            worker_thread_id: None,
            checkpoint_path: None,
            checkpoint: None,
            summary: None,
            error: Some(error.clone()),
            metadata: json!({}),
            started_at: None,
            completed_at: Some(completed_at),
        },
    );
    ledger_upsert_telemetry(
        ctx.config,
        RunTelemetryUpsert {
            run_id: task_id.clone(),
            tool_count: child_tool_counts.get(&task_id).copied(),
            error: Some(error.clone()),
            ..Default::default()
        },
    );
    ledger_append_event(
        ctx.config,
        RunEventAppend {
            run_id: task_id.clone(),
            event_type: "subagent_failed".to_string(),
            payload: json!({
                "agentId": agent_id,
                "error": error,
                "parentCallId": parent_call_id
            }),
        },
    );
    publish_seq_stamped(
        ctx.dispatch,
        emit_seq,
        WebChannelEvent {
            event: "subagent_failed".to_string(),
            client_id: ctx.client_id.to_string(),
            thread_id: ctx.thread_id.to_string(),
            request_id: ctx.request_id.to_string(),
            message: Some(error),
            tool_name: Some(agent_id),
            skill_id: Some(task_id),
            success: Some(false),
            subagent: Some(SubagentProgressDetail {
                parent_call_id,
                ..Default::default()
            }),
            round: Some(round),
            ..Default::default()
        },
    );
}

pub(super) fn on_subagent_awaiting_user(
    ctx: &BridgeCtx<'_>,
    emit_seq: &mut u64,
    subagent_parent_call_ids: &HashMap<String, Option<String>>,
    round: u32,
    agent_id: String,
    task_id: String,
    question: String,
    worker_thread_id: Option<String>,
    checkpoint_path: Option<String>,
) {
    let parent_call_id = subagent_parent_call_ids.get(&task_id).cloned().flatten();
    log::debug!(
        "[web_channel][bridge] subagent_awaiting_user agent_id={} task_id={} client_id={} thread_id={} request_id={}",
        agent_id,
        task_id,
        ctx.client_id,
        ctx.thread_id,
        ctx.request_id,
    );
    ledger_upsert_agent_run(
        ctx.config,
        AgentRunUpsert {
            id: task_id.clone(),
            kind: if worker_thread_id.is_some() {
                AgentRunKind::WorkerThread
            } else {
                AgentRunKind::Subagent
            },
            parent_run_id: Some(ctx.request_id.to_string()),
            parent_thread_id: Some(ctx.thread_id.to_string()),
            agent_id: Some(agent_id.clone()),
            status: AgentRunStatus::AwaitingUser,
            prompt_ref: None,
            worker_thread_id: worker_thread_id.clone(),
            // What the runner actually wrote; the old rebuild from
            // `workspace_dir` asserted a checkpoint that may never have been
            // written (#5928).
            checkpoint_path: checkpoint_path.clone(),
            checkpoint: Some(json!({
                "resumeTool": "continue_subagent",
                "taskId": task_id,
                "agentId": agent_id,
                "question": question,
                "workerThreadId": worker_thread_id,
                "checkpointPersisted": checkpoint_path.is_some()
            })),
            summary: Some(question.clone()),
            error: None,
            metadata: json!({}),
            started_at: None,
            completed_at: None,
        },
    );
    ledger_append_event(
        ctx.config,
        RunEventAppend {
            run_id: task_id.clone(),
            event_type: "subagent_awaiting_user".to_string(),
            payload: json!({
                "agentId": agent_id,
                "question": question,
                "workerThreadId": worker_thread_id,
                "parentCallId": parent_call_id
            }),
        },
    );
    publish_seq_stamped(
        ctx.dispatch,
        emit_seq,
        WebChannelEvent {
            event: "subagent_awaiting_user".to_string(),
            client_id: ctx.client_id.to_string(),
            thread_id: ctx.thread_id.to_string(),
            request_id: ctx.request_id.to_string(),
            message: Some(question),
            tool_name: Some(agent_id),
            skill_id: Some(task_id),
            success: Some(true),
            round: Some(round),
            subagent: Some(SubagentProgressDetail {
                worker_thread_id,
                parent_call_id,
                ..Default::default()
            }),
            ..Default::default()
        },
    );
}

pub(super) fn on_subagent_iteration_started(
    ctx: &BridgeCtx<'_>,
    emit_seq: &mut u64,
    round: u32,
    agent_id: String,
    task_id: String,
    iteration: u32,
    max_iterations: u32,
    extended_policy: bool,
) {
    publish_seq_stamped(
        ctx.dispatch,
        emit_seq,
        WebChannelEvent {
            event: "subagent_iteration_start".to_string(),
            client_id: ctx.client_id.to_string(),
            thread_id: ctx.thread_id.to_string(),
            request_id: ctx.request_id.to_string(),
            message: Some(if extended_policy {
                format!("Sub-agent '{agent_id}' step {iteration}")
            } else {
                format!("Sub-agent '{agent_id}' iteration {iteration}/{max_iterations}")
            }),
            tool_name: Some(agent_id),
            skill_id: Some(task_id),
            round: Some(round),
            subagent: Some(SubagentProgressDetail {
                child_iteration: Some(iteration),
                child_max_iterations: if extended_policy {
                    None
                } else {
                    Some(max_iterations)
                },
                ..Default::default()
            }),
            ..Default::default()
        },
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn on_subagent_tool_call_started(
    ctx: &BridgeCtx<'_>,
    emit_seq: &mut u64,
    child_tool_counts: &mut HashMap<String, u64>,
    round: u32,
    agent_id: String,
    task_id: String,
    call_id: String,
    tool_name: String,
    arguments: serde_json::Value,
    iteration: u32,
    display_label: Option<String>,
    display_detail: Option<String>,
) {
    let count = child_tool_counts.entry(task_id.clone()).or_insert(0);
    *count += 1;
    ledger_upsert_telemetry(
        ctx.config,
        RunTelemetryUpsert {
            run_id: task_id.clone(),
            tool_count: Some(*count),
            ..Default::default()
        },
    );
    ledger_append_event(
        ctx.config,
        RunEventAppend {
            run_id: task_id.clone(),
            event_type: "subagent_tool_call_started".to_string(),
            payload: json!({
                "agentId": agent_id,
                "callId": call_id,
                "toolName": tool_name,
                "iteration": iteration
            }),
        },
    );
    publish_seq_stamped(
        ctx.dispatch,
        emit_seq,
        WebChannelEvent {
            event: "subagent_tool_call".to_string(),
            client_id: ctx.client_id.to_string(),
            thread_id: ctx.thread_id.to_string(),
            request_id: ctx.request_id.to_string(),
            tool_name: Some(tool_name),
            skill_id: Some(task_id.clone()),
            // The child's tool arguments, so the UI can show what the
            // sub-agent actually did (issue: subagent drawer detail).
            // Skipped from the wire when `null`.
            args: if arguments.is_null() {
                None
            } else {
                Some(arguments)
            },
            round: Some(round),
            tool_call_id: Some(call_id),
            tool_display_label: display_label,
            tool_display_detail: display_detail,
            subagent: Some(SubagentProgressDetail {
                child_iteration: Some(iteration),
                agent_id: Some(agent_id),
                task_id: Some(task_id),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn on_subagent_tool_call_completed(
    ctx: &BridgeCtx<'_>,
    emit_seq: &mut u64,
    round: u32,
    agent_id: String,
    task_id: String,
    call_id: String,
    tool_name: String,
    success: bool,
    output_chars: usize,
    output: String,
    arguments: Option<serde_json::Value>,
    elapsed_ms: u64,
    iteration: u32,
    failure: Option<tinytools::status::ClassifiedFailure>,
    display_label: Option<String>,
    display_detail: Option<String>,
    structured: Option<serde_json::Value>,
) {
    // Serialize the classified failure (if any) so a failed sub-agent tool
    // row carries its "why + next" copy on the wire + ledger, matching the
    // main-agent path (#4459).
    let failure_json = failure.as_ref().and_then(|f| serde_json::to_value(f).ok());
    ledger_append_event(
        ctx.config,
        RunEventAppend {
            run_id: task_id.clone(),
            event_type: "subagent_tool_call_completed".to_string(),
            payload: json!({
                "agentId": agent_id,
                "callId": call_id,
                "toolName": tool_name,
                "success": success,
                "outputChars": output_chars,
                "elapsedMs": elapsed_ms,
                "iteration": iteration,
                "failure": failure_json,
            }),
        },
    );
    publish_seq_stamped(
        ctx.dispatch,
        emit_seq,
        WebChannelEvent {
            event: "subagent_tool_result".to_string(),
            client_id: ctx.client_id.to_string(),
            thread_id: ctx.thread_id.to_string(),
            request_id: ctx.request_id.to_string(),
            tool_name: Some(tool_name),
            skill_id: Some(task_id.clone()),
            success: Some(success),
            round: Some(round),
            tool_call_id: Some(call_id),
            // The child's actual tool output, so the drawer can show *what
            // came back* (not just a char count). Capped to a bounded size
            // for the wire (#4007); `output_chars` + `elapsed_ms` still ride
            // along in `subagent` below.
            output: Some(cap_wire_output(output)),
            args: cap_wire_args(arguments),
            elapsed_ms: Some(elapsed_ms),
            structured,
            tool_display_label: display_label,
            tool_display_detail: display_detail,
            failure: failure_json,
            subagent: Some(SubagentProgressDetail {
                child_iteration: Some(iteration),
                agent_id: Some(agent_id),
                task_id: Some(task_id),
                elapsed_ms: Some(elapsed_ms),
                output_chars: Some(output_chars as u64),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
}

pub(super) fn on_subagent_text_delta(
    ctx: &BridgeCtx<'_>,
    emit_seq: &mut u64,
    round: u32,
    agent_id: String,
    task_id: String,
    delta: String,
    iteration: u32,
) {
    publish_seq_stamped(
        ctx.dispatch,
        emit_seq,
        WebChannelEvent {
            event: "subagent_text_delta".to_string(),
            client_id: ctx.client_id.to_string(),
            thread_id: ctx.thread_id.to_string(),
            request_id: ctx.request_id.to_string(),
            round: Some(round),
            delta: Some(delta),
            delta_kind: Some("text".to_string()),
            skill_id: Some(task_id.clone()),
            subagent: Some(SubagentProgressDetail {
                child_iteration: Some(iteration),
                agent_id: Some(agent_id),
                task_id: Some(task_id),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
}

pub(super) fn on_subagent_thinking_delta(
    ctx: &BridgeCtx<'_>,
    emit_seq: &mut u64,
    round: u32,
    agent_id: String,
    task_id: String,
    delta: String,
    iteration: u32,
) {
    publish_seq_stamped(
        ctx.dispatch,
        emit_seq,
        WebChannelEvent {
            event: "subagent_thinking_delta".to_string(),
            client_id: ctx.client_id.to_string(),
            thread_id: ctx.thread_id.to_string(),
            request_id: ctx.request_id.to_string(),
            round: Some(round),
            delta: Some(delta),
            delta_kind: Some("thinking".to_string()),
            skill_id: Some(task_id.clone()),
            subagent: Some(SubagentProgressDetail {
                child_iteration: Some(iteration),
                agent_id: Some(agent_id),
                task_id: Some(task_id),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
}
