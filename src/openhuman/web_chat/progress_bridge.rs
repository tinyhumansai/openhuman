use serde_json::json;

use crate::core::socketio::{SubagentProgressDetail, WebChannelEvent};
use crate::openhuman::threads::turn_state::{TurnStateMirror, TurnStateStore};

use super::event_bus::publish_web_channel_event;
use super::types::ChatRequestMetadata;

/// Cadence of the `inference_heartbeat` liveness beat the bridge emits while a
/// turn is in flight (issue #4270). The frontend silence timer in
/// `Conversations.tsx` only fires after ~120s with NO progress signal of any
/// kind; a long prefill on a large context, or a reasoning-tier model that
/// buffers `reasoning_content` server-side, can legitimately stream nothing for
/// minutes — tripping a false "no response after 2 minutes" timeout that
/// discards the live turn. A wall-clock beat every 20s rides the same socket as
/// the real progress events, so it keeps the timer armed while work is genuinely
/// progressing yet stops the instant the socket/core dies — preserving the
/// genuine-disconnect error path (6 missed beats before the 120s window lapses).
const INFERENCE_HEARTBEAT_SECS: u64 = 20;

/// Minimum trimmed length for the parent agent's leading narration to be
/// surfaced as its own interim chat bubble. Below this a stray "Ok." / "Sure."
/// is left as transient streaming text rather than persisted as a message.
const MIN_INTERIM_NARRATION_CHARS: usize = 24;

/// Flush the parent agent's accumulated leading narration (streamed before a
/// tool call in the current round) as an interim `chat_interim` event, so it
/// persists as a chat bubble interleaved with the tool activity instead of
/// vanishing when the turn settles. Clears `buffer` unconditionally; emits
/// nothing for narration that is empty or too short to stand alone.
fn flush_interim_narration(
    buffer: &mut String,
    round: u32,
    client_id: &str,
    thread_id: &str,
    request_id: &str,
) {
    let text = std::mem::take(buffer);
    let Some(narration) = interim_narration_text(&text) else {
        return;
    };
    log::debug!(
        "[web_channel][bridge] chat_interim round={} chars={} request_id={}",
        round,
        narration.chars().count(),
        request_id,
    );
    publish_web_channel_event(WebChannelEvent {
        event: "chat_interim".to_string(),
        client_id: client_id.to_string(),
        thread_id: thread_id.to_string(),
        request_id: request_id.to_string(),
        full_response: Some(narration),
        round: Some(round),
        ..Default::default()
    });
}

/// The trimmed narration to surface as an interim bubble, or `None` when it is
/// empty or shorter than [`MIN_INTERIM_NARRATION_CHARS`]. Pure so the threshold
/// is unit-testable without the global event bus.
fn interim_narration_text(buffer: &str) -> Option<String> {
    let trimmed = buffer.trim();
    if trimmed.chars().count() < MIN_INTERIM_NARRATION_CHARS {
        return None;
    }
    Some(trimmed.to_string())
}

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

pub(super) fn ledger_get_telemetry(
    config: &crate::openhuman::config::Config,
    run_id: &str,
) -> Option<crate::openhuman::session_db::run_ledger::RunTelemetry> {
    match crate::openhuman::session_db::run_ledger::get_agent_run(config, run_id) {
        Ok(Some(run)) => {
            let telemetry = run.telemetry;
            log::debug!(
                "[run_ledger][web_channel] read telemetry run_id={} present={}",
                run_id,
                telemetry.is_some()
            );
            telemetry
        }
        Ok(None) => {
            log::debug!(
                "[run_ledger][web_channel] telemetry unavailable; run missing run_id={}",
                run_id
            );
            None
        }
        Err(err) => {
            log::warn!(
                "[run_ledger][web_channel] failed to read telemetry run_id={} err={err}",
                run_id
            );
            None
        }
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

/// Trace user attribution for a turn whose `auth_get_me` cache is cold
/// (headless / autonomous / freshly booted cores): read the on-disk
/// app-session profile and return the user's email (preferred) or backend
/// user id. `None` when signed out or the profile is unreadable — the caller
/// then falls back to the transport client id.
fn session_profile_user_attribution(config: &crate::openhuman::config::Config) -> Option<String> {
    let state = crate::openhuman::credentials::session_support::build_session_state(config).ok()?;
    state
        .user
        .as_ref()
        .and_then(|u| u.get("email"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or(state.user_id)
}

fn span_projection_signature(
    spans: &[crate::openhuman::agent::progress_tracing::TraceSpan],
) -> Vec<String> {
    spans
        .iter()
        .map(|span| {
            let attr_keys = span
                .attributes
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "{:?}|{}|{:?}|attrs:[{}]",
                span.kind, span.name, span.status, attr_keys
            )
        })
        .collect()
}

async fn shadow_compare_journal_projection(
    request_id: &str,
    trace_ctx: crate::openhuman::agent::progress_tracing::TraceContext,
    max_iterations: u32,
    live_spans: &[crate::openhuman::agent::progress_tracing::TraceSpan],
) -> Option<Vec<tinyagents::harness::observability::AgentObservation>> {
    let Some(journal_run_id) =
        crate::openhuman::tinyagents::journal::take_request_journal_run(request_id)
    else {
        log::debug!(
            "[agent-tracing][journal-shadow] no journal run registered request_id={}",
            request_id
        );
        return None;
    };

    let observations = match crate::openhuman::tinyagents::journal::read_run_events(
        &journal_run_id,
        0,
    )
    .await
    {
        Ok(observations) => observations,
        Err(err) => {
            log::warn!(
                    "[agent-tracing][journal-shadow] read failed request_id={} journal_run_id={} err={err}",
                    request_id,
                    journal_run_id
                );
            return None;
        }
    };
    if observations.is_empty() {
        log::warn!(
            "[agent-tracing][journal-shadow] journal empty request_id={} journal_run_id={}",
            request_id,
            journal_run_id
        );
        return None;
    }

    let projected =
        crate::openhuman::agent::progress_tracing::journal_projection::spans_from_observations(
            trace_ctx,
            max_iterations,
            &observations,
        );
    let live_sig = span_projection_signature(live_spans);
    let projected_sig = span_projection_signature(&projected);
    if live_sig == projected_sig {
        log::debug!(
            "[agent-tracing][journal-shadow] parity ok request_id={} journal_run_id={} spans={} observations={}",
            request_id,
            journal_run_id,
            live_spans.len(),
            observations.len()
        );
    } else {
        log::warn!(
            "[agent-tracing][journal-shadow] parity divergence request_id={} journal_run_id={} live_spans={} journal_spans={} observations={} live_sig={:?} journal_sig={:?}",
            request_id,
            journal_run_id,
            live_spans.len(),
            projected.len(),
            observations.len(),
            live_sig,
            projected_sig
        );
    }
    Some(observations)
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
        let mut parent_max_iterations: u32 = 0;
        // Accumulates the parent agent's streamed narration for the current
        // round. When a tool call closes the round, this leading narration is
        // flushed as an interim chat bubble (`chat_interim`) so it persists in
        // the thread instead of vanishing on settle — the final answer arrives
        // separately via `deliver_response` and is never part of this buffer
        // (it belongs to the terminal round, which ends with no tool call).
        let mut pending_narration = String::new();
        let mut events_seen: u64 = 0;
        let mut parent_completed = false;
        let mut parent_tool_count: u64 = 0;
        let mut child_tool_counts: HashMap<String, u64> = HashMap::new();
        let mut turn_state =
            TurnStateMirror::new(turn_state_store, thread_id.clone(), request_id.clone());

        // #3886: opt-in structured tracing export. When enabled, fold the same
        // progress stream into OTel/Langfuse-style spans correlated by session
        // id (falling back to the thread id for headless/autonomous runs).
        // `None` (disabled) is zero-cost.
        let mut journal_trace_ctx = None;
        let mut span_collector = if config.observability.share_usage_data
            || config.observability.agent_tracing.enabled
        {
            use crate::openhuman::agent::progress_tracing::{
                trace_session_id, RunType, SpanCollector, TraceContext,
            };
            // One trace per turn: the trace id is unique per request, while the
            // thread id rides along as the Langfuse `sessionId` so a
            // conversation's per-turn traces still group under one session.
            let base = trace_session_id(metadata.session_id, &thread_id);
            let trace_id = format!("{base}:{request_id}");
            // Attribute the trace to the *real* authenticated user (cached
            // `auth_get_me` identity: id, else email) — the transport client
            // id (socket client / "system") is NOT a user; it rides along as
            // the separate `client.id` metadata attribute. When no identity is
            // cached (signed-out / fresh install), fall back to the client id
            // so the trace still carries some attribution.
            let identity = crate::openhuman::app_state::peek_cached_current_user_identity();
            let user_attributed = identity.is_some();
            let user_id = identity
                .and_then(|i| i.id.or(i.email))
                .or_else(|| session_profile_user_attribution(&config))
                .unwrap_or_else(|| client_id.clone());
            // Run origin for trace metadata: the request's source tag
            // ("ptt"/"dictation"/"type"/"agentbox"/"autonomous"/…), else a
            // plain interactive chat turn.
            let run_type = RunType::from_source(metadata.source.as_deref());
            let channel_source = metadata
                .source
                .clone()
                .unwrap_or_else(|| "chat".to_string());
            // Storage-level privacy gate (#4454): capture_content (off by
            // default) rides on the TraceContext so the collector only attaches
            // prompt/reply content to spans when the operator opted in — no
            // exporter can serialize prompt/reply text otherwise.
            let capture_content = config.observability.agent_tracing.capture_content;
            log::debug!(
                "[web_channel][bridge] trace context trace_id={} user_attributed={} \
                 agent_id={:?} channel_source={} run_type={} capture_content={} request_id={}",
                trace_id,
                user_attributed,
                metadata.agent_id,
                channel_source,
                run_type.as_str(),
                capture_content,
                request_id,
            );
            let mut trace_ctx = TraceContext::new(trace_id, Some(user_id))
                .with_session_group(thread_id.clone())
                .with_client_id(client_id.clone())
                .with_channel_source(channel_source)
                .with_run_type(run_type)
                .with_capture_content(capture_content);
            if let Some(agent_id) = metadata.agent_id.clone() {
                trace_ctx = trace_ctx.with_agent_id(agent_id);
            }
            journal_trace_ctx = Some(trace_ctx.clone());
            Some(SpanCollector::new(trace_ctx))
        } else {
            None
        };

        // #4270: emit a periodic liveness beat for the whole in-flight turn so
        // the frontend silence timer never false-fires during a long prefill or
        // a buffered-reasoning phase that streams no progress events. The beat
        // is gated on `turn_active` (set once `TurnStarted` is observed) so we
        // never emit before the turn's `inference_start` has armed the timer.
        let mut heartbeat =
            tokio::time::interval(std::time::Duration::from_secs(INFERENCE_HEARTBEAT_SECS));
        // Wall-clock cadence: a slow turn must not produce a burst of catch-up
        // beats once it finally yields control.
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // `interval`'s first tick resolves immediately — consume it so the first
        // real beat lands one full interval after the turn begins.
        heartbeat.tick().await;
        let mut turn_active = false;

        loop {
            let event = tokio::select! {
                // Drain real progress events preferentially over the timer so a
                // busy turn never starves event handling to emit a beat.
                biased;
                maybe = rx.recv() => match maybe {
                    Some(ev) => ev,
                    None => break,
                },
                _ = heartbeat.tick() => {
                    if turn_active {
                        log::trace!(
                            "[web_channel][bridge] inference_heartbeat thread_id={} request_id={}",
                            thread_id,
                            request_id,
                        );
                        publish_web_channel_event(WebChannelEvent {
                            event: "inference_heartbeat".to_string(),
                            client_id: client_id.clone(),
                            thread_id: thread_id.clone(),
                            request_id: request_id.clone(),
                            ..Default::default()
                        });
                    }
                    continue;
                }
            };
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
                    // Turn is live — start emitting liveness beats (issue #4270).
                    turn_active = true;
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
                    parent_max_iterations = max_iterations;
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
                    // The parent's leading narration for this round is complete
                    // once it calls a tool — flush it as an interim bubble so it
                    // persists interleaved with the tool activity.
                    flush_interim_narration(
                        &mut pending_narration,
                        iteration,
                        &client_id,
                        &thread_id,
                        &request_id,
                    );
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
                    output,
                    elapsed_ms,
                    iteration,
                    failure,
                    ..
                } => {
                    // Serialize the classified failure (if any) for the UI + ledger.
                    let failure_json = failure.as_ref().and_then(|f| serde_json::to_value(f).ok());
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
                                "iteration": iteration,
                                "failure": failure_json,
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
                        // Forward the real tool result (size-capped) so the UI
                        // can render tool output — mirrors the subagent
                        // `subagent_tool_result` path. Frontends that only
                        // need size/timing read the ledger telemetry instead.
                        output: Some(cap_wire_output(output)),
                        success: Some(success),
                        round: Some(iteration),
                        tool_call_id: Some(call_id),
                        failure: failure_json,
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
                    ..
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
                    ..
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
                    failure,
                    ..
                } => {
                    // Serialize the classified failure (if any) so a failed
                    // sub-agent tool row carries its "why + next" copy on the
                    // wire + ledger, matching the main-agent path (#4459).
                    let failure_json = failure.as_ref().and_then(|f| serde_json::to_value(f).ok());
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
                                "iteration": iteration,
                                "failure": failure_json,
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
                    // Buffer the round's narration so it can be flushed as an
                    // interim bubble if a tool call closes this round.
                    pending_narration.push_str(&delta);
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
                    // Turn is done — stop liveness beats (issue #4270). The FE
                    // clears its silence timer on `chat_done`/`chat_error`; this
                    // also prevents a stray beat racing the channel close.
                    turn_active = false;
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
                AgentProgress::TurnContent { .. } => {
                    // Prompt/reply content is attached to the trace span by the
                    // span collector above; the ledger/telemetry bridge ignores it.
                }
                AgentProgress::ModelCallCompleted {
                    model,
                    iteration,
                    input_tokens,
                    output_tokens,
                    cost_usd,
                    ..
                } => {
                    // Per-call usage is consumed by the span collector above
                    // (per-call Langfuse generation); the socket/ledger surfaces
                    // stay on the cumulative TurnCostUpdated rollup.
                    log::debug!(
                        "[web_channel][bridge] model_call_completed model={model} iter={iteration} \
                         in={input_tokens} out={output_tokens} cost_usd={cost_usd:.6} request_id={request_id}"
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
            let live_spans = collector.spans().to_vec();
            let journal_export = if let Some(trace_ctx) = journal_trace_ctx.take() {
                shadow_compare_journal_projection(
                    &request_id,
                    trace_ctx.clone(),
                    parent_max_iterations,
                    &live_spans,
                )
                .await
                .map(|observations| (trace_ctx, observations))
            } else {
                None
            };
            if let Some((trace_ctx, observations)) = journal_export {
                let run_telemetry = ledger_get_telemetry(&config, &request_id);
                crate::openhuman::agent::progress_tracing::export_run_trace_from_journal(
                    &config,
                    &trace_ctx,
                    &observations,
                    run_telemetry.as_ref(),
                    &live_spans,
                )
                .await;
            } else {
                crate::openhuman::agent::progress_tracing::export_run_trace(&config, &live_spans)
                    .await;
            }
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
    use super::interim_narration_text;
    use super::session_profile_user_attribution;

    #[test]
    fn interim_narration_skips_empty_and_trivial() {
        assert_eq!(interim_narration_text(""), None);
        assert_eq!(interim_narration_text("   \n  "), None);
        // Below the min length → left as transient streaming text.
        assert_eq!(interim_narration_text("Ok."), None);
        assert_eq!(interim_narration_text("Sure, one sec"), None);
    }

    #[test]
    fn interim_narration_surfaces_and_trims_substantial_text() {
        let text = "  Let me check your calendar for conflicts first.  ";
        assert_eq!(
            interim_narration_text(text),
            Some("Let me check your calendar for conflicts first.".to_string())
        );
    }

    #[test]
    fn session_profile_attribution_none_when_signed_out() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = crate::openhuman::config::Config {
            workspace_dir: tmp.path().join("workspace"),
            action_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Default::default()
        };
        assert!(session_profile_user_attribution(&config).is_none());
    }

    #[test]
    fn session_profile_attribution_prefers_email_from_stored_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = crate::openhuman::config::Config {
            workspace_dir: tmp.path().join("workspace"),
            action_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Default::default()
        };
        let service = crate::openhuman::credentials::AuthService::from_config(&config);
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "user_json".to_string(),
            "{\"email\": \"steven@example.test\", \"_id\": \"u-1\"}".to_string(),
        );
        metadata.insert("user_id".to_string(), "u-1".to_string());
        service
            .store_provider_token(
                crate::openhuman::credentials::APP_SESSION_PROVIDER,
                crate::openhuman::credentials::DEFAULT_AUTH_PROFILE_NAME,
                "session-token",
                metadata,
                true,
            )
            .expect("store session profile");
        assert_eq!(
            session_profile_user_attribution(&config).as_deref(),
            Some("steven@example.test"),
            "cold-cache attribution must resolve the on-disk session email"
        );
    }

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

    /// The `tool_result` wire event must carry the tool's real (capped) output
    /// so the UI can render what the tool returned — not the legacy
    /// `{"output_chars", "elapsed_ms"}` metadata stub (which broke both the
    /// timeline result view and the `propose_workflow` proposal parser).
    #[tokio::test]
    async fn tool_call_completed_forwards_real_output_on_tool_result() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = crate::openhuman::config::Config {
            workspace_dir: tmp.path().join("workspace"),
            action_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Default::default()
        };
        let store = TurnStateStore::new(tmp.path().join("turn_states"));
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let mut bus = super::super::event_bus::subscribe_web_channel_events();
        spawn_progress_bridge(
            rx,
            "client-out".into(),
            "thread-out".into(),
            "req-out".into(),
            store,
            ChatRequestMetadata::default(),
            config,
        );

        tx.send(
            crate::openhuman::agent::progress::AgentProgress::ToolCallCompleted {
                call_id: "call-1".into(),
                tool_name: "web_search".into(),
                success: true,
                output_chars: 12,
                output: "real payload".into(),
                arguments: None,
                elapsed_ms: 42,
                iteration: 1,
                failure: None,
            },
        )
        .await
        .expect("send progress");

        // The bus is process-global — skip unrelated events from concurrent
        // tests and wait (bounded) for our thread's tool_result.
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match bus.recv().await {
                    Ok(ev) if ev.thread_id == "thread-out" && ev.event == "tool_result" => {
                        return ev;
                    }
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(err) => panic!("bus closed: {err}"),
                }
            }
        })
        .await
        .expect("tool_result within timeout");

        assert_eq!(event.output.as_deref(), Some("real payload"));
        assert_eq!(event.success, Some(true));
        assert_eq!(event.tool_call_id.as_deref(), Some("call-1"));
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

    // ── #4270 inference heartbeat ────────────────────────────────────────────

    use crate::openhuman::agent::progress::AgentProgress;
    use crate::openhuman::config::Config;
    use std::time::Duration;
    use tokio::sync::broadcast::error::TryRecvError;

    /// Await the next web-channel event published for `thread_id`, skipping
    /// events for other threads (the bus is a process-global broadcast) and
    /// tolerating broadcast lag. Panics if the channel closes first.
    async fn recv_for_thread(
        rx: &mut tokio::sync::broadcast::Receiver<WebChannelEvent>,
        thread_id: &str,
    ) -> WebChannelEvent {
        loop {
            match rx.recv().await {
                Ok(ev) if ev.thread_id == thread_id => return ev,
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(err) => panic!("web-channel bus closed before event: {err}"),
            }
        }
    }

    fn spawn_test_bridge(
        thread_id: &str,
        request_id: &str,
    ) -> tokio::sync::mpsc::Sender<AgentProgress> {
        let (tx, rx) = tokio::sync::mpsc::channel::<AgentProgress>(16);
        let dir = tempfile::tempdir().expect("tempdir");
        let store = TurnStateStore::new(dir.path().to_path_buf());
        // Keep the tempdir alive for the bridge task's lifetime by leaking it —
        // a test-only allocation; the OS reclaims it on process exit.
        std::mem::forget(dir);
        spawn_progress_bridge(
            rx,
            "client-hb-4270".to_string(),
            thread_id.to_string(),
            request_id.to_string(),
            store,
            ChatRequestMetadata::default(),
            Config::default(),
        );
        tx
    }

    /// Repro-gone guard: once a turn is in flight, the bridge emits a periodic
    /// `inference_heartbeat` even though no other progress event has streamed —
    /// this is the signal the FE silence timer rearms on to avoid the false
    /// "no response after 2 minutes" timeout (#4270).
    #[tokio::test(start_paused = true)]
    async fn emits_inference_heartbeat_while_turn_in_flight() {
        let mut events = super::super::event_bus::subscribe_web_channel_events();
        let thread_id = "thread-hb-emit-4270";
        let request_id = "req-hb-emit-4270";
        let tx = spawn_test_bridge(thread_id, request_id);

        // Turn begins — arms the liveness beat.
        tx.send(AgentProgress::TurnStarted).await.unwrap();

        // inference_start first, then a heartbeat after the interval elapses
        // (the paused clock auto-advances while the test awaits the bus).
        let start = recv_for_thread(&mut events, thread_id).await;
        assert_eq!(start.event, "inference_start");

        let beat = recv_for_thread(&mut events, thread_id).await;
        assert_eq!(beat.event, "inference_heartbeat");
        assert_eq!(beat.thread_id, thread_id);
        assert_eq!(beat.request_id, request_id);

        drop(tx);
    }

    /// Lifecycle: once `TurnCompleted` lands the bridge stops beating, so a beat
    /// can't race the channel close after the FE has already cleared its timer
    /// on `chat_done`/`chat_error`. Exercises the `turn_active = false` arm and
    /// the channel-closed `break`.
    #[tokio::test(start_paused = true)]
    async fn stops_heartbeat_after_turn_completed() {
        let mut events = super::super::event_bus::subscribe_web_channel_events();
        let thread_id = "thread-hb-stop-4270";
        let tx = spawn_test_bridge(thread_id, "req-hb-stop-4270");

        tx.send(AgentProgress::TurnStarted).await.unwrap();
        // Drain through the first heartbeat to prove the turn was beating.
        loop {
            if recv_for_thread(&mut events, thread_id).await.event == "inference_heartbeat" {
                break;
            }
        }

        // Complete the turn, then drop the sender so the bridge loop breaks.
        tx.send(AgentProgress::TurnCompleted { iterations: 1 })
            .await
            .unwrap();
        drop(tx);

        // Let the bridge process TurnCompleted + observe the closed channel.
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        // Advance well past several intervals — no further beats must appear.
        tokio::time::advance(Duration::from_secs(INFERENCE_HEARTBEAT_SECS * 4)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }

        loop {
            match events.try_recv() {
                Ok(ev) => assert_ne!(
                    (ev.thread_id.as_str(), ev.event.as_str()),
                    (thread_id, "inference_heartbeat"),
                    "heartbeat emitted after TurnCompleted"
                ),
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
                Err(TryRecvError::Lagged(_)) => continue,
            }
        }
    }

    /// Gate check: before `TurnStarted` the bridge must NOT beat — otherwise a
    /// beat could land before the FE has armed its timer. Exercises the
    /// `turn_active == false` branch of the heartbeat tick.
    #[tokio::test(start_paused = true)]
    async fn no_heartbeat_before_turn_started() {
        let mut events = super::super::event_bus::subscribe_web_channel_events();
        let thread_id = "thread-hb-gate-4270";
        let tx = spawn_test_bridge(thread_id, "req-hb-gate-4270");

        // Advance well past several heartbeat intervals with no TurnStarted.
        tokio::time::advance(Duration::from_secs(INFERENCE_HEARTBEAT_SECS * 4)).await;
        // Let the bridge task run its (no-op) heartbeat ticks.
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }

        // No event of any kind should have been published for this thread.
        loop {
            match events.try_recv() {
                Ok(ev) => assert_ne!(
                    ev.thread_id, thread_id,
                    "unexpected pre-turn event {} for {thread_id}",
                    ev.event
                ),
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
                Err(TryRecvError::Lagged(_)) => continue,
            }
        }

        drop(tx);
    }
}
