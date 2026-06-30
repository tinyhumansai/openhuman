use serde_json::json;

use crate::core::socketio::{SubagentProgressDetail, WebChannelEvent};
use crate::openhuman::threads::turn_state::{TurnStateMirror, TurnStateStore};

use super::event_bus::publish_web_channel_event;
use super::types::ChatRequestMetadata;

/// Current wall-clock time as Unix-epoch milliseconds, used to stamp tracing
/// spans (issue #3886). Saturates to `0` if the clock is before the epoch.
fn unix_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Upper bound on the sub-agent tool output forwarded to the drawer over
/// Socket.IO. The `SubagentToolCallCompleted` event carries the *pre-handoff*
/// tool result (the result-handoff path that stashes large toolkit payloads
/// behind a short placeholder runs later, in `SubagentToolSource`), so a raw
/// multi-MB integration result would otherwise ship in full to the socket /
/// Redux / DOM. Cap it here on a UTF-8 boundary with a truncation marker so the
/// drawer payload stays bounded while still showing what the tool returned.
const MAX_WIRE_SUBAGENT_OUTPUT: usize = 256 * 1024;

/// Bytes reserved within the cap for the truncation marker so the *final*
/// payload (content + marker) never exceeds [`MAX_WIRE_SUBAGENT_OUTPUT`].
/// Generous upper bound for `…[truncated <N> bytes of tool output]` at any
/// plausible `N` (the "…" is 3 UTF-8 bytes).
const TRUNCATION_MARKER_BUDGET: usize = 80;

/// Truncate `output` so the returned string stays within
/// [`MAX_WIRE_SUBAGENT_OUTPUT`] bytes, slicing on a char boundary and
/// appending a marker (which is itself counted against the cap) when content
/// was dropped. Returns the input unchanged when it's already within the cap.
fn cap_wire_output(output: String) -> String {
    if output.len() <= MAX_WIRE_SUBAGENT_OUTPUT {
        return output;
    }
    let mut end = MAX_WIRE_SUBAGENT_OUTPUT.saturating_sub(TRUNCATION_MARKER_BUDGET);
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    let omitted = output.len() - end;
    format!(
        "{}\n…[truncated {omitted} bytes of tool output]",
        &output[..end]
    )
}

pub(super) fn ledger_upsert_agent_run(
    config: &crate::openhuman::config::Config,
    upsert: crate::openhuman::session_db::run_ledger::AgentRunUpsert,
) {
    if let Err(err) = crate::openhuman::session_db::run_ledger::upsert_agent_run(config, upsert) {
        log::warn!("[run_ledger][web_channel] failed to upsert run: {err}");
    }
}

pub(super) fn ledger_append_event(
    config: &crate::openhuman::config::Config,
    event: crate::openhuman::session_db::run_ledger::RunEventAppend,
) {
    if let Err(err) = crate::openhuman::session_db::run_ledger::append_run_event(config, event) {
        log::warn!("[run_ledger][web_channel] failed to append event: {err}");
    }
}

pub(super) fn ledger_upsert_telemetry(
    config: &crate::openhuman::config::Config,
    telemetry: crate::openhuman::session_db::run_ledger::RunTelemetryUpsert,
) {
    if let Err(err) =
        crate::openhuman::session_db::run_ledger::upsert_run_telemetry(config, telemetry)
    {
        log::warn!("[run_ledger][web_channel] failed to upsert telemetry: {err}");
    }
}

/// Build the worktree-isolation slice of a `subagent_completed`
/// [`SubagentProgressDetail`] (#3376). An empty `changed_files` collapses to
/// `None` so the renderer omits an empty "changed files" list rather than
/// showing "0 files"; a non-empty list is forwarded verbatim. `worktree_path`
/// / `dirty_status` pass through (`None` for non-isolated workers). Split out
/// so the empty/non-empty branch is unit-testable without a live DB + channel.
fn subagent_worktree_detail(
    worktree_path: Option<String>,
    changed_files: Vec<String>,
    dirty_status: Option<bool>,
) -> SubagentProgressDetail {
    SubagentProgressDetail {
        worktree_path,
        changed_files: if changed_files.is_empty() {
            None
        } else {
            Some(changed_files)
        },
        dirty_status,
        ..Default::default()
    }
}

/// Spawn a background task that reads [`AgentProgress`] events from the
/// agent turn loop and translates them into [`WebChannelEvent`]s tagged
/// with the correct client/thread/request IDs. The task runs until the
/// sender is dropped (i.e. when the agent turn finishes).
pub(crate) fn spawn_progress_bridge(
    mut rx: tokio::sync::mpsc::Receiver<crate::openhuman::agent::progress::AgentProgress>,
    client_id: String,
    thread_id: String,
    request_id: String,
    turn_state_store: TurnStateStore,
    metadata: ChatRequestMetadata,
    config: crate::openhuman::config::Config,
) {
    use crate::openhuman::agent::progress::AgentProgress;
    use crate::openhuman::session_db::run_ledger::{
        AgentRunKind, AgentRunStatus, AgentRunUpsert, RunEventAppend, RunTelemetryUpsert,
    };
    use std::collections::HashMap;

    tokio::spawn(async move {
        log::debug!(
            "[web_channel][bridge] spawned client_id={} thread_id={} request_id={} speak_reply={:?} source={:?} session_id={:?}",
            client_id,
            thread_id,
            request_id,
            metadata.speak_reply,
            metadata.source,
            metadata.session_id,
        );
        let mut round: u32 = 0;
        let mut events_seen: u64 = 0;
        let mut parent_completed = false;
        let mut parent_tool_count: u64 = 0;
        let mut child_tool_counts: HashMap<String, u64> = HashMap::new();
        let mut turn_state =
            TurnStateMirror::new(turn_state_store, thread_id.clone(), request_id.clone());

        // #3886: opt-in structured tracing export. When enabled, fold the same
        // progress stream into OTel/Langfuse-style spans correlated by session
        // id (falling back to the thread id for headless/autonomous runs) with
        // the client id as user attribution. `None` (disabled) is zero-cost.
        let mut span_collector = if config.observability.agent_tracing.enabled {
            use crate::openhuman::agent::progress_tracing::{
                trace_session_id, SpanCollector, TraceContext,
            };
            let session_id = trace_session_id(metadata.session_id, &thread_id);
            Some(SpanCollector::new(TraceContext::new(
                session_id,
                Some(client_id.clone()),
            )))
        } else {
            None
        };

        while let Some(event) = rx.recv().await {
            events_seen += 1;
            turn_state.observe(&event);
            if let Some(collector) = span_collector.as_mut() {
                collector.record(&event, unix_epoch_ms());
            }
            match &event {
                AgentProgress::TextDelta { delta, iteration } => {
                    log::trace!(
                        "[web_channel][bridge] text_delta round={} chars={} request_id={}",
                        iteration,
                        delta.len(),
                        request_id,
                    );
                }
                AgentProgress::ThinkingDelta { delta, iteration } => {
                    log::trace!(
                        "[web_channel][bridge] thinking_delta round={} chars={} request_id={}",
                        iteration,
                        delta.len(),
                        request_id,
                    );
                }
                AgentProgress::ToolCallArgsDelta {
                    call_id,
                    tool_name,
                    delta,
                    iteration,
                } => {
                    log::trace!(
                        "[web_channel][bridge] tool_args_delta round={} tool={} call_id={} chars={} request_id={}",
                        iteration,
                        tool_name,
                        call_id,
                        delta.len(),
                        request_id,
                    );
                }
                AgentProgress::ToolCallStarted {
                    call_id,
                    tool_name,
                    iteration,
                    ..
                } => {
                    log::debug!(
                        "[web_channel][bridge] tool_call round={} tool={} call_id={} request_id={}",
                        iteration,
                        tool_name,
                        call_id,
                        request_id,
                    );
                }
                AgentProgress::ToolCallCompleted {
                    call_id,
                    tool_name,
                    success,
                    iteration,
                    ..
                } => {
                    log::debug!(
                        "[web_channel][bridge] tool_result round={} tool={} call_id={} success={} request_id={}",
                        iteration,
                        tool_name,
                        call_id,
                        success,
                        request_id,
                    );
                }
                AgentProgress::SubagentFailed {
                    agent_id, error, ..
                } => {
                    log::warn!(
                        "[web_channel][bridge] subagent_failed agent_id={} err={} client_id={} thread_id={} request_id={}",
                        agent_id,
                        error,
                        client_id,
                        thread_id,
                        request_id,
                    );
                }
                other => {
                    log::debug!(
                        "[web_channel][bridge] lifecycle event={:?} request_id={}",
                        std::mem::discriminant(other),
                        request_id,
                    );
                }
            }
            match event {
                AgentProgress::TurnStarted => {
                    ledger_upsert_agent_run(
                        &config,
                        AgentRunUpsert {
                            id: request_id.clone(),
                            kind: AgentRunKind::BackgroundAgent,
                            parent_run_id: None,
                            parent_thread_id: Some(thread_id.clone()),
                            agent_id: Some("orchestrator".to_string()),
                            status: AgentRunStatus::Running,
                            prompt_ref: Some(format!("thread:{thread_id}:request:{request_id}")),
                            worker_thread_id: None,
                            task_board_id: Some(thread_id.clone()),
                            task_card_id: None,
                            checkpoint_path: None,
                            checkpoint: None,
                            summary: None,
                            error: None,
                            metadata: json!({
                                "clientId": client_id,
                                "source": "web_channel",
                                "schemaVersion": 1
                            }),
                            started_at: None,
                            completed_at: None,
                        },
                    );
                    ledger_append_event(
                        &config,
                        RunEventAppend {
                            run_id: request_id.clone(),
                            event_type: "turn_started".to_string(),
                            payload: json!({ "threadId": thread_id, "clientId": client_id }),
                        },
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "inference_start".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        ..Default::default()
                    });
                }
                AgentProgress::IterationStarted {
                    iteration,
                    max_iterations,
                } => {
                    round = iteration;
                    publish_web_channel_event(WebChannelEvent {
                        event: "iteration_start".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        message: Some(format!("Iteration {iteration}/{max_iterations}")),
                        round: Some(iteration),
                        ..Default::default()
                    });
                }
                AgentProgress::ToolCallStarted {
                    call_id,
                    tool_name,
                    arguments,
                    iteration,
                    display_label,
                    display_detail,
                } => {
                    parent_tool_count += 1;
                    ledger_append_event(
                        &config,
                        RunEventAppend {
                            run_id: request_id.clone(),
                            event_type: "tool_call_started".to_string(),
                            payload: json!({
                                "callId": call_id,
                                "toolName": tool_name,
                                "iteration": iteration
                            }),
                        },
                    );
                    ledger_upsert_telemetry(
                        &config,
                        RunTelemetryUpsert {
                            run_id: request_id.clone(),
                            tool_count: Some(parent_tool_count),
                            ..Default::default()
                        },
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "tool_call".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some("web_channel".to_string()),
                        args: Some(arguments),
                        round: Some(iteration),
                        tool_call_id: Some(call_id),
                        tool_display_label: display_label,
                        tool_display_detail: display_detail,
                        ..Default::default()
                    });
                }
                AgentProgress::ToolCallCompleted {
                    call_id,
                    tool_name,
                    success,
                    output_chars,
                    elapsed_ms,
                    iteration,
                } => {
                    ledger_append_event(
                        &config,
                        RunEventAppend {
                            run_id: request_id.clone(),
                            event_type: "tool_call_completed".to_string(),
                            payload: json!({
                                "callId": call_id,
                                "toolName": tool_name,
                                "success": success,
                                "outputChars": output_chars,
                                "elapsedMs": elapsed_ms,
                                "iteration": iteration
                            }),
                        },
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "tool_result".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some("web_channel".to_string()),
                        output: Some(
                            json!({"output_chars": output_chars, "elapsed_ms": elapsed_ms})
                                .to_string(),
                        ),
                        success: Some(success),
                        round: Some(iteration),
                        tool_call_id: Some(call_id),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentSpawned {
                    agent_id,
                    task_id,
                    mode,
                    dedicated_thread,
                    prompt_chars,
                    worker_thread_id,
                    display_name,
                } => {
                    let label = display_name.as_deref().unwrap_or(&agent_id);
                    let kind = if worker_thread_id.is_some() {
                        AgentRunKind::WorkerThread
                    } else {
                        AgentRunKind::Subagent
                    };
                    ledger_upsert_agent_run(
                        &config,
                        AgentRunUpsert {
                            id: task_id.clone(),
                            kind,
                            parent_run_id: Some(request_id.clone()),
                            parent_thread_id: Some(thread_id.clone()),
                            agent_id: Some(agent_id.clone()),
                            status: AgentRunStatus::Running,
                            prompt_ref: worker_thread_id
                                .as_ref()
                                .map(|id| format!("thread:{id}:message:seed")),
                            worker_thread_id: worker_thread_id.clone(),
                            task_board_id: Some(thread_id.clone()),
                            task_card_id: None,
                            checkpoint_path: None,
                            checkpoint: None,
                            summary: None,
                            error: None,
                            metadata: json!({
                                "mode": mode,
                                "dedicatedThread": dedicated_thread,
                                "promptChars": prompt_chars,
                                "displayName": display_name,
                                "source": "agent_progress",
                                "schemaVersion": 1
                            }),
                            started_at: None,
                            completed_at: None,
                        },
                    );
                    ledger_append_event(
                        &config,
                        RunEventAppend {
                            run_id: task_id.clone(),
                            event_type: "subagent_spawned".to_string(),
                            payload: json!({
                                "agentId": agent_id,
                                "parentRunId": request_id,
                                "threadId": thread_id,
                                "workerThreadId": worker_thread_id,
                                "mode": mode,
                                "dedicatedThread": dedicated_thread,
                                "promptChars": prompt_chars,
                                "displayName": display_name
                            }),
                        },
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_spawned".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
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
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentCompleted {
                    agent_id,
                    task_id,
                    elapsed_ms,
                    iterations,
                    output_chars,
                    worktree_path,
                    changed_files,
                    dirty_status,
                } => {
                    let completed_at = chrono::Utc::now();
                    ledger_upsert_agent_run(
                        &config,
                        AgentRunUpsert {
                            id: task_id.clone(),
                            kind: AgentRunKind::Subagent,
                            parent_run_id: Some(request_id.clone()),
                            parent_thread_id: Some(thread_id.clone()),
                            agent_id: Some(agent_id.clone()),
                            status: AgentRunStatus::Completed,
                            prompt_ref: None,
                            worker_thread_id: None,
                            task_board_id: Some(thread_id.clone()),
                            task_card_id: None,
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
                        &config,
                        RunTelemetryUpsert {
                            run_id: task_id.clone(),
                            elapsed_ms: Some(elapsed_ms),
                            tool_count: child_tool_counts.get(&task_id).copied(),
                            ..Default::default()
                        },
                    );
                    ledger_append_event(
                        &config,
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
                                "dirtyStatus": dirty_status
                            }),
                        },
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_completed".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
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
                            // Worktree isolation metadata (#3376) — drives the
                            // inline subagent worktree row's open/diff/remove
                            // actions. All `None`/absent for non-isolated workers.
                            ..subagent_worktree_detail(worktree_path, changed_files, dirty_status)
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentFailed {
                    agent_id,
                    task_id,
                    error,
                } => {
                    let completed_at = chrono::Utc::now();
                    ledger_upsert_agent_run(
                        &config,
                        AgentRunUpsert {
                            id: task_id.clone(),
                            kind: AgentRunKind::Subagent,
                            parent_run_id: Some(request_id.clone()),
                            parent_thread_id: Some(thread_id.clone()),
                            agent_id: Some(agent_id.clone()),
                            status: AgentRunStatus::Failed,
                            prompt_ref: None,
                            worker_thread_id: None,
                            task_board_id: Some(thread_id.clone()),
                            task_card_id: None,
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
                        &config,
                        RunTelemetryUpsert {
                            run_id: task_id.clone(),
                            tool_count: child_tool_counts.get(&task_id).copied(),
                            error: Some(error.clone()),
                            ..Default::default()
                        },
                    );
                    ledger_append_event(
                        &config,
                        RunEventAppend {
                            run_id: task_id.clone(),
                            event_type: "subagent_failed".to_string(),
                            payload: json!({ "agentId": agent_id, "error": error }),
                        },
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_failed".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        message: Some(error),
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        success: Some(false),
                        round: Some(round),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentAwaitingUser {
                    agent_id,
                    task_id,
                    question,
                    worker_thread_id,
                } => {
                    log::debug!(
                        "[web_channel][bridge] subagent_awaiting_user agent_id={} task_id={} client_id={} thread_id={} request_id={}",
                        agent_id,
                        task_id,
                        client_id,
                        thread_id,
                        request_id,
                    );
                    let checkpoint_path = config
                        .workspace_dir
                        .join(".openhuman/subagent_checkpoints")
                        .join(format!("{task_id}.json"));
                    ledger_upsert_agent_run(
                        &config,
                        AgentRunUpsert {
                            id: task_id.clone(),
                            kind: if worker_thread_id.is_some() {
                                AgentRunKind::WorkerThread
                            } else {
                                AgentRunKind::Subagent
                            },
                            parent_run_id: Some(request_id.clone()),
                            parent_thread_id: Some(thread_id.clone()),
                            agent_id: Some(agent_id.clone()),
                            status: AgentRunStatus::AwaitingUser,
                            prompt_ref: None,
                            worker_thread_id: worker_thread_id.clone(),
                            task_board_id: Some(thread_id.clone()),
                            task_card_id: None,
                            checkpoint_path: Some(checkpoint_path.to_string_lossy().to_string()),
                            checkpoint: Some(json!({
                                "resumeTool": "continue_subagent",
                                "taskId": task_id,
                                "agentId": agent_id,
                                "question": question,
                                "workerThreadId": worker_thread_id
                            })),
                            summary: Some(question.clone()),
                            error: None,
                            metadata: json!({}),
                            started_at: None,
                            completed_at: None,
                        },
                    );
                    ledger_append_event(
                        &config,
                        RunEventAppend {
                            run_id: task_id.clone(),
                            event_type: "subagent_awaiting_user".to_string(),
                            payload: json!({
                                "agentId": agent_id,
                                "question": question,
                                "workerThreadId": worker_thread_id
                            }),
                        },
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_awaiting_user".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        message: Some(question),
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        success: Some(true),
                        round: Some(round),
                        subagent: Some(SubagentProgressDetail {
                            worker_thread_id,
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentIterationStarted {
                    agent_id,
                    task_id,
                    iteration,
                    max_iterations,
                    extended_policy,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_iteration_start".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
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
                    });
                }
                AgentProgress::SubagentToolCallStarted {
                    agent_id,
                    task_id,
                    call_id,
                    tool_name,
                    arguments,
                    iteration,
                    display_label,
                    display_detail,
                } => {
                    let count = child_tool_counts.entry(task_id.clone()).or_insert(0);
                    *count += 1;
                    ledger_upsert_telemetry(
                        &config,
                        RunTelemetryUpsert {
                            run_id: task_id.clone(),
                            tool_count: Some(*count),
                            ..Default::default()
                        },
                    );
                    ledger_append_event(
                        &config,
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
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_tool_call".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some(task_id.clone()),
                        // The child's tool arguments, so the UI can show what
                        // the sub-agent actually did (issue: subagent drawer
                        // detail). Skipped from the wire when `null`.
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
                    });
                }
                AgentProgress::SubagentToolCallCompleted {
                    agent_id,
                    task_id,
                    call_id,
                    tool_name,
                    success,
                    output_chars,
                    output,
                    elapsed_ms,
                    iteration,
                } => {
                    ledger_append_event(
                        &config,
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
                                "iteration": iteration
                            }),
                        },
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_tool_result".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some(task_id.clone()),
                        success: Some(success),
                        round: Some(round),
                        tool_call_id: Some(call_id),
                        // The child's actual tool output, so the drawer can show
                        // *what came back* (not just a char count). Capped to a
                        // bounded size for the wire (#4007); `output_chars` +
                        // `elapsed_ms` still ride along in `subagent` below.
                        output: Some(cap_wire_output(output)),
                        subagent: Some(SubagentProgressDetail {
                            child_iteration: Some(iteration),
                            agent_id: Some(agent_id),
                            task_id: Some(task_id),
                            elapsed_ms: Some(elapsed_ms),
                            output_chars: Some(output_chars as u64),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentTextDelta {
                    agent_id,
                    task_id,
                    delta,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_text_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
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
                    });
                }
                AgentProgress::SubagentThinkingDelta {
                    agent_id,
                    task_id,
                    delta,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_thinking_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
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
                    });
                }
                AgentProgress::TaskBoardUpdated { board } => {
                    log::debug!(
                        "[web_channel][bridge] task_board_updated client_id={} thread_id={} request_id={} cards={}",
                        client_id,
                        thread_id,
                        request_id,
                        board.cards.len()
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "task_board_updated".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        task_board: Some(serde_json::to_value(board).unwrap_or_else(
                            |_| serde_json::json!({ "threadId": thread_id, "cards": [] }),
                        )),
                        ..Default::default()
                    });
                }
                AgentProgress::TextDelta { delta, iteration } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "text_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        round: Some(iteration),
                        delta: Some(delta),
                        delta_kind: Some("text".to_string()),
                        ..Default::default()
                    });
                }
                AgentProgress::ThinkingDelta { delta, iteration } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "thinking_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        round: Some(iteration),
                        delta: Some(delta),
                        delta_kind: Some("thinking".to_string()),
                        ..Default::default()
                    });
                }
                AgentProgress::ToolCallArgsDelta {
                    call_id,
                    tool_name,
                    delta,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "tool_args_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: if tool_name.is_empty() {
                            None
                        } else {
                            Some(tool_name)
                        },
                        skill_id: Some("web_channel".to_string()),
                        round: Some(iteration),
                        delta: Some(delta),
                        delta_kind: Some("tool_args".to_string()),
                        tool_call_id: Some(call_id),
                        ..Default::default()
                    });
                }
                AgentProgress::TurnCompleted { iterations } => {
                    parent_completed = true;
                    let completed_at = chrono::Utc::now();
                    ledger_upsert_agent_run(
                        &config,
                        AgentRunUpsert {
                            id: request_id.clone(),
                            kind: AgentRunKind::BackgroundAgent,
                            parent_run_id: None,
                            parent_thread_id: Some(thread_id.clone()),
                            agent_id: Some("orchestrator".to_string()),
                            status: AgentRunStatus::Completed,
                            prompt_ref: Some(format!("thread:{thread_id}:request:{request_id}")),
                            worker_thread_id: None,
                            task_board_id: Some(thread_id.clone()),
                            task_card_id: None,
                            checkpoint_path: None,
                            checkpoint: None,
                            summary: Some(format!("Completed in {iterations} iteration(s)")),
                            error: None,
                            metadata: json!({}),
                            started_at: None,
                            completed_at: Some(completed_at),
                        },
                    );
                    ledger_append_event(
                        &config,
                        RunEventAppend {
                            run_id: request_id.clone(),
                            event_type: "turn_completed".to_string(),
                            payload: json!({ "iterations": iterations }),
                        },
                    );
                    log::debug!(
                        "[web_channel] turn completed after {iterations} iteration(s) \
                         client_id={client_id} thread_id={thread_id} request_id={request_id} \
                         speak_reply={:?} source={:?} session_id={:?}",
                        metadata.speak_reply,
                        metadata.source,
                        metadata.session_id,
                    );
                }
                AgentProgress::TurnCostUpdated {
                    model,
                    iteration,
                    input_tokens,
                    output_tokens,
                    cached_input_tokens,
                    total_usd,
                } => {
                    ledger_upsert_telemetry(
                        &config,
                        RunTelemetryUpsert {
                            run_id: request_id.clone(),
                            input_tokens: Some(input_tokens),
                            output_tokens: Some(output_tokens),
                            cached_input_tokens: Some(cached_input_tokens),
                            cost_usd: Some(total_usd),
                            model: Some(model.clone()),
                            ..Default::default()
                        },
                    );
                    log::debug!(
                        "[web_channel] turn cost update model={model} iter={iteration} \
                         in={input_tokens} out={output_tokens} cached_in={cached_input_tokens} \
                         total_usd={total_usd:.4} client_id={client_id} thread_id={thread_id}"
                    );
                }
            }
        }
        turn_state.finish();
        if !parent_completed {
            ledger_upsert_agent_run(
                &config,
                AgentRunUpsert {
                    id: request_id.clone(),
                    kind: AgentRunKind::BackgroundAgent,
                    parent_run_id: None,
                    parent_thread_id: Some(thread_id.clone()),
                    agent_id: Some("orchestrator".to_string()),
                    status: AgentRunStatus::Interrupted,
                    prompt_ref: Some(format!("thread:{thread_id}:request:{request_id}")),
                    worker_thread_id: None,
                    task_board_id: Some(thread_id.clone()),
                    task_card_id: None,
                    checkpoint_path: None,
                    checkpoint: None,
                    summary: None,
                    error: Some("progress bridge exited before turn completion".to_string()),
                    metadata: json!({}),
                    started_at: None,
                    completed_at: Some(chrono::Utc::now()),
                },
            );
            ledger_append_event(
                &config,
                RunEventAppend {
                    run_id: request_id.clone(),
                    event_type: "turn_interrupted".to_string(),
                    payload: json!({ "eventsSeen": events_seen }),
                },
            );
        }
        // #3886: seal any spans still open after the stream closed and hand the
        // run's trace to the configured tracing sink. Best-effort and gated;
        // never affects the turn outcome.
        if let Some(mut collector) = span_collector.take() {
            collector.finish(unix_epoch_ms());
            crate::openhuman::agent::progress_tracing::export_spans(
                &config.observability.agent_tracing,
                collector.spans(),
            );
        }

        log::debug!(
            "[web_channel][bridge] exit client_id={} thread_id={} request_id={} round={} events_seen={}",
            client_id,
            thread_id,
            request_id,
            round,
            events_seen,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_wire_output_passes_through_small_payloads() {
        let s = "small tool result".to_string();
        assert_eq!(cap_wire_output(s.clone()), s);
    }

    #[test]
    fn cap_wire_output_truncates_large_payloads_on_char_boundary() {
        // A multibyte payload past the cap: result stays valid UTF-8, is shorter
        // than the input, and carries the truncation marker.
        let big = "é".repeat(MAX_WIRE_SUBAGENT_OUTPUT); // 2 bytes each → well over cap
        let capped = cap_wire_output(big.clone());
        assert!(capped.len() < big.len());
        assert!(capped.contains("[truncated"));
        // Truncation landed on a char boundary (no replacement char / panic).
        assert!(capped.starts_with('é'));
        // The final payload (content + marker) must honor the wire cap.
        assert!(capped.len() <= MAX_WIRE_SUBAGENT_OUTPUT);
    }

    #[test]
    fn worktree_detail_collapses_empty_changed_files_to_none() {
        // Non-isolated / clean worker: empty list → `None` so the renderer
        // omits the "changed files" section instead of showing an empty one.
        let d = subagent_worktree_detail(None, vec![], None);
        assert_eq!(d.worktree_path, None);
        assert_eq!(d.changed_files, None);
        assert_eq!(d.dirty_status, None);
    }

    #[test]
    fn worktree_detail_forwards_isolated_worker_fields() {
        // Isolated worker with uncommitted changes: fields pass through and a
        // non-empty list is wrapped in `Some`.
        let d = subagent_worktree_detail(
            Some("/repo/.claude/worktrees/run-1".to_string()),
            vec!["src/lib.rs".to_string(), "README.md".to_string()],
            Some(true),
        );
        assert_eq!(
            d.worktree_path.as_deref(),
            Some("/repo/.claude/worktrees/run-1")
        );
        assert_eq!(
            d.changed_files,
            Some(vec!["src/lib.rs".to_string(), "README.md".to_string()])
        );
        assert_eq!(d.dirty_status, Some(true));
    }
}
