//! Forwards a running turn's `AgentProgress` stream into `WebChannelEvent`
//! socket events and mirrors it into the `TurnStates` port. Also emits the
//! `inference_heartbeat` liveness beat (see [`INFERENCE_HEARTBEAT_SECS`])
//! so a long silent prefill can't trip the frontend's ~120s silence timeout.

use serde_json::json;

use crate::threads::turn_state::mirror::ObserveProgress;
use crate::web_chat::{SubagentProgressDetail, WebChannelEvent};
use tinyagents_session::{port::TurnStates, turn_state::TurnStateMirror};

use super::event_bus::publish_web_channel_event_in;
use super::types::ChatRequestMetadata;
use crate::core::runtime::DispatchCtx;

#[path = "progress_bridge_subagent_events.rs"]
mod subagent_events;
#[path = "progress_bridge_text_events.rs"]
mod text_events;
#[path = "progress_bridge_tool_progress.rs"]
mod tool_progress;
#[path = "progress_bridge_trace.rs"]
mod trace;
use subagent_events::BridgeCtx;

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
/// flushed as a `chat_interim` event when the round's first tool call starts.
///
/// Any non-empty narration flushes. This used to be 24 characters, meant to
/// keep a stray "Ok." from persisting as a bubble — but the frontend no longer
/// promotes narration to a message, and `chat_interim` is also the signal that
/// resets its live preview for the next round. A short "Let me check." that
/// was never flushed stayed in the preview and the next round's text was
/// appended straight onto it ("Let me check.Here's the answer").
const MIN_INTERIM_NARRATION_CHARS: usize = 1;

/// How long a finished turn waits for its progress bridge to forward every
/// event the turn queued before the terminal `chat_done`/`chat_error` is
/// published. Bounded: a detached sub-agent can hold a sender clone and keep
/// the channel open past the turn, and a missing `TurnCompleted` (failed
/// turn) must not stall delivery.
pub(crate) const BRIDGE_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Handle to a spawned progress bridge, used to wait until it has forwarded
/// the parent turn's events.
///
/// The bridge runs on its own task, fed by a bounded channel. The turn's
/// caller used to publish `chat_done` as soon as the turn returned, while the
/// bridge could still be holding queued `tool_result`/`chat_interim` events —
/// so the terminal event overtook them and the UI settled rows that were
/// about to be settled correctly (or never saw their results at all).
#[derive(Clone)]
pub(crate) struct ProgressBridgeHandle {
    drained: tokio::sync::watch::Receiver<bool>,
    /// Set once, from inside the bridge task, when the turn's
    /// `AgentProgress::TurnCompleted` arrives (`TurnTiming::snapshot()`).
    /// Read by the caller after `wait_drained` so the same numbers the
    /// `time-to-first-visible` log line reports reach `chat_done.timing`.
    timing: std::sync::Arc<std::sync::Mutex<Option<super::turn_timing::TurnTimingSnapshot>>>,
}

impl ProgressBridgeHandle {
    /// Wait until the bridge has handled the parent's `TurnCompleted` (every
    /// event queued before it has been forwarded, in order) or its channel
    /// closed — at most `timeout`. Returns whether it drained in time.
    pub(crate) async fn wait_drained(&self, timeout: std::time::Duration) -> bool {
        let mut drained = self.drained.clone();
        let result = tokio::time::timeout(timeout, drained.wait_for(|done| *done)).await;
        // A dropped sender means the bridge task ended, which is drained too.
        matches!(result, Ok(Ok(_)) | Ok(Err(_)))
    }

    /// The turn's timing snapshot, if the bridge saw a `TurnCompleted` before
    /// its channel closed. `None` for a turn that errored/was interrupted
    /// before completing a round, or was never polled after completion.
    pub(crate) fn timing_snapshot(&self) -> Option<super::turn_timing::TurnTimingSnapshot> {
        self.timing.lock().ok().and_then(|guard| *guard)
    }
}

/// Flush the parent agent's accumulated leading narration (streamed before a
/// tool call in the current round) as an interim `chat_interim` event, so it
/// persists as a chat bubble interleaved with the tool activity instead of
/// vanishing when the turn settles. Clears `buffer` unconditionally; emits
/// nothing for narration that is empty or too short to stand alone.
fn flush_interim_narration(
    ctx: &crate::core::runtime::DispatchCtx,
    buffer: &mut String,
    round: u32,
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    emit_seq: &mut u64,
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
    publish_seq_stamped(
        ctx,
        emit_seq,
        WebChannelEvent {
            event: "chat_interim".to_string(),
            client_id: client_id.to_string(),
            thread_id: thread_id.to_string(),
            request_id: request_id.to_string(),
            full_response: Some(narration),
            round: Some(round),
            ..Default::default()
        },
    );
}

/// Stamp a per-request monotonic sequence number on an outgoing web-channel
/// event and publish it. `request_id` is already carried on every
/// [`WebChannelEvent`]; `seq` is the additive ordering key the frontend uses to
/// dedup replayed vs live events by `(request_id, seq)` and to order them
/// identically to the persisted turn-state snapshot
/// (conversations-timeline-refactor, Phase 4). The counter advances once per
/// emitted event so each `(request_id, seq)` pair is unique within a turn.
fn publish_seq_stamped(ctx: &DispatchCtx, next_seq: &mut u64, mut event: WebChannelEvent) {
    event.seq = Some(*next_seq);
    *next_seq = next_seq.saturating_add(1);
    publish_web_channel_event_in(ctx, event);
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
///
/// `pub(crate)` so `web_chat::event_bus` and `openhuman_rpc::server::socketio` can stamp
/// `WebChannelEvent.ts` with the same clock instead of keeping a second
/// epoch-ms helper in step by hand.
pub fn unix_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[path = "progress_bridge_wire_caps.rs"]
mod wire_caps;
#[cfg(test)]
use wire_caps::MAX_WIRE_SUBAGENT_OUTPUT;
use wire_caps::{cap_wire_args, cap_wire_output};

use crate::agent::session_store::ledger_sync;

pub(super) fn ledger_upsert_agent_run(
    config: &crate::config::Config,
    upsert: tinyagents_session::run_ledger::AgentRunUpsert,
) {
    if let Err(err) = ledger_sync::upsert_agent_run(&config.workspace_dir, upsert) {
        log::warn!("[run_ledger][web_channel] failed to upsert run: {err}");
    }
}

pub(super) fn ledger_append_event(
    config: &crate::config::Config,
    event: tinyagents_session::run_ledger::RunEventAppend,
) {
    if let Err(err) = ledger_sync::append_run_event(&config.workspace_dir, event) {
        log::warn!("[run_ledger][web_channel] failed to append event: {err}");
    }
}

pub(super) fn ledger_upsert_telemetry(
    config: &crate::config::Config,
    telemetry: tinyagents_session::run_ledger::RunTelemetryUpsert,
) {
    if let Err(err) = ledger_sync::upsert_run_telemetry(&config.workspace_dir, telemetry) {
        log::warn!("[run_ledger][web_channel] failed to upsert telemetry: {err}");
    }
}

pub(super) fn ledger_get_telemetry(
    config: &crate::config::Config,
    run_id: &str,
) -> Option<tinyagents_session::run_ledger::RunTelemetry> {
    match ledger_sync::get_agent_run(&config.workspace_dir, run_id) {
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

/// Trace user attribution for a turn whose stored user payload carries no identity
/// (headless / autonomous / freshly booted cores): read the on-disk
/// app-session profile and return the user's email (preferred) or backend
/// user id. `None` when signed out or the profile is unreadable.
fn session_profile_user_attribution(config: &crate::config::Config) -> Option<String> {
    let state = crate::security::credentials::session_support::build_session_state(config).ok()?;
    state
        .user
        .as_ref()
        .and_then(|u| u.get("email"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or(state.user_id)
}

/// Spawn a background task that reads [`AgentProgress`] events from the
/// agent turn loop and translates them into [`WebChannelEvent`]s tagged
/// with the correct client/thread/request IDs. The task runs until the
/// sender is dropped (i.e. when the agent turn finishes).
pub(crate) fn spawn_progress_bridge(
    mut rx: tokio::sync::mpsc::Receiver<crate::agent::progress::AgentProgress>,
    client_id: String,
    thread_id: String,
    request_id: String,
    turn_state_store: std::sync::Arc<dyn TurnStates>,
    metadata: ChatRequestMetadata,
    config: crate::config::Config,
    dispatch_ctx: crate::core::runtime::DispatchCtx,
) -> ProgressBridgeHandle {
    use crate::agent::progress::AgentProgress;
    use std::collections::HashMap;
    use tinyagents_session::run_ledger::{
        AgentRunKind, AgentRunStatus, AgentRunUpsert, RunEventAppend, RunTelemetryUpsert,
    };

    let (drained_tx, drained_rx) = tokio::sync::watch::channel(false);
    let timing_snapshot: std::sync::Arc<
        std::sync::Mutex<Option<super::turn_timing::TurnTimingSnapshot>>,
    > = std::sync::Arc::new(std::sync::Mutex::new(None));
    let timing_snapshot_for_task = timing_snapshot.clone();
    dispatch_ctx.clone().spawn(async move {
        log::debug!(
            "[web_channel][bridge] spawned client_id={} thread_id={} request_id={} speak_reply={:?} source={:?} session_id={:?} hold_text_stream={}",
            client_id,
            thread_id,
            request_id,
            metadata.speak_reply,
            metadata.source,
            metadata.session_id,
            metadata.hold_text_stream,
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
        let mut timing = super::turn_timing::TurnTiming::start();
        let mut turn_cost_throttle = super::turn_timing::TurnCostThrottle::new();
        let mut events_seen: u64 = 0;
        // Per-request monotonic ordering key stamped on every emitted
        // web-channel event (see `publish_seq_stamped`). Unique per emission so
        // the frontend can dedup by `(request_id, seq)`.
        let mut emit_seq: u64 = 0;
        let mut parent_completed = false;
        let mut parent_tool_count: u64 = 0;
        let mut child_tool_counts: HashMap<String, u64> = HashMap::new();
        // task_id -> the parent tool-call id that spawned it, remembered from
        // `SubagentSpawned` so later lifecycle events for the same task
        // (`subagent_completed`/`_failed`/`_awaiting_user`) can still carry
        // it even though those `AgentProgress` variants don't repeat it.
        let mut subagent_parent_call_ids: HashMap<String, Option<String>> = HashMap::new();
        let mut turn_state =
            TurnStateMirror::new(turn_state_store, thread_id.clone(), request_id.clone());

        // #3886: structured tracing export (`progress_bridge_trace.rs`).
        let (mut span_collector, mut journal_trace_ctx) =
            trace::span_collector(&config, &metadata, &client_id, &thread_id, &request_id).unzip();

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
        let bridge_ctx =
            BridgeCtx::new(&client_id, &thread_id, &request_id, &config, &dispatch_ctx);

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
                        publish_seq_stamped(&dispatch_ctx, &mut emit_seq, WebChannelEvent {
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
                    publish_seq_stamped(
                        &dispatch_ctx,
                        &mut emit_seq,
                        WebChannelEvent {
                            event: "inference_start".to_string(),
                            client_id: client_id.clone(),
                            thread_id: thread_id.clone(),
                            request_id: request_id.clone(),
                            ..Default::default()
                        },
                    );
                }
                AgentProgress::IterationStarted {
                    iteration,
                    max_iterations,
                } => {
                    round = iteration;
                    parent_max_iterations = max_iterations;
                    publish_seq_stamped(
                        &dispatch_ctx,
                        &mut emit_seq,
                        WebChannelEvent {
                            event: "iteration_start".to_string(),
                            client_id: client_id.clone(),
                            thread_id: thread_id.clone(),
                            request_id: request_id.clone(),
                            message: Some(format!("Iteration {iteration}/{max_iterations}")),
                            round: Some(iteration),
                            ..Default::default()
                        },
                    );
                }
                AgentProgress::ToolCallStarted {
                    call_id,
                    tool_name,
                    arguments,
                    iteration,
                    display_label,
                    display_detail,
                } => {
                    timing.tool_call(&tool_name, iteration, &request_id);
                    // The parent's leading narration for this round is complete
                    // once it calls a tool — flush it as an interim bubble so it
                    // persists interleaved with the tool activity.
                    flush_interim_narration(
                        &dispatch_ctx,
                        &mut pending_narration,
                        iteration,
                        &client_id,
                        &thread_id,
                        &request_id,
                        &mut emit_seq,
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
                    publish_seq_stamped(
                        &dispatch_ctx,
                        &mut emit_seq,
                        WebChannelEvent {
                            event: "tool_call".to_string(),
                            client_id: client_id.clone(),
                            thread_id: thread_id.clone(),
                            request_id: request_id.clone(),
                            tool_name: Some(tool_name),
                            skill_id: Some("web_channel".to_string()),
                            args: cap_wire_args(Some(arguments)),
                            round: Some(iteration),
                            tool_call_id: Some(call_id),
                            tool_display_label: display_label,
                            tool_display_detail: display_detail,
                            ..Default::default()
                        },
                    );
                }
                AgentProgress::ToolCallCompleted {
                    call_id,
                    tool_name,
                    success,
                    output_chars,
                    output,
                    arguments,
                    elapsed_ms,
                    iteration,
                    failure,
                    display_label,
                    display_detail,
                    structured,
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
                    log::debug!(
                        "[web_channel][bridge] tool_result round={} tool={} call_id={} \
                         success={} elapsed_ms={} has_structured={} request_id={}",
                        iteration,
                        tool_name,
                        call_id,
                        success,
                        elapsed_ms,
                        structured.is_some(),
                        request_id
                    );
                    publish_seq_stamped(
                        &dispatch_ctx,
                        &mut emit_seq,
                        WebChannelEvent {
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
                            // The call arguments the harness captured at
                            // completion (`ToolCallStarted.arguments` is
                            // always `Null` on this path). Omitted when the
                            // harness ran with payload capture off.
                            args: cap_wire_args(arguments),
                            success: Some(success),
                            round: Some(iteration),
                            tool_call_id: Some(call_id),
                            failure: failure_json,
                            elapsed_ms: Some(elapsed_ms),
                            structured,
                            // Recomputed from the real arguments (unlike the
                            // started event's args-free computation), so a
                            // completed row can pick up a detail that only
                            // became knowable once the arguments existed.
                            tool_display_label: display_label,
                            tool_display_detail: display_detail,
                            ..Default::default()
                        },
                    );
                }
                AgentProgress::SubagentSpawned {
                    agent_id,
                    task_id,
                    mode,
                    dedicated_thread,
                    prompt_chars,
                    worker_thread_id,
                    display_name,
                    parent_call_id,
                    ..
                } => {
                    subagent_events::on_subagent_spawned(
                        &bridge_ctx,
                        &mut emit_seq,
                        &mut subagent_parent_call_ids,
                        round,
                        agent_id,
                        task_id,
                        mode,
                        dedicated_thread,
                        prompt_chars,
                        worker_thread_id,
                        display_name,
                        parent_call_id,
                    );
                }
                AgentProgress::SubagentCompleted {
                    agent_id,
                    task_id,
                    elapsed_ms,
                    iterations,
                    output_chars,
                    output,
                    usage,
                    worktree_path,
                    changed_files,
                    dirty_status,
                    ..
                } => {
                    subagent_events::on_subagent_completed(
                        &bridge_ctx,
                        &mut emit_seq,
                        &mut subagent_parent_call_ids,
                        &child_tool_counts,
                        round,
                        agent_id,
                        task_id,
                        elapsed_ms,
                        iterations,
                        output_chars,
                        output,
                        usage,
                        worktree_path,
                        changed_files,
                        dirty_status,
                    );
                }
                AgentProgress::SubagentFailed {
                    agent_id,
                    task_id,
                    error,
                } => {
                    subagent_events::on_subagent_failed(
                        &bridge_ctx,
                        &mut emit_seq,
                        &mut subagent_parent_call_ids,
                        &child_tool_counts,
                        round,
                        agent_id,
                        task_id,
                        error,
                    );
                }
                AgentProgress::SubagentAwaitingUser {
                    agent_id,
                    task_id,
                    question,
                    worker_thread_id,
                    checkpoint_path,
                } => {
                    subagent_events::on_subagent_awaiting_user(
                        &bridge_ctx,
                        &mut emit_seq,
                        &subagent_parent_call_ids,
                        round,
                        agent_id,
                        task_id,
                        question,
                        worker_thread_id,
                        checkpoint_path,
                    );
                }
                AgentProgress::SubagentIterationStarted {
                    agent_id,
                    task_id,
                    iteration,
                    max_iterations,
                    extended_policy,
                } => {
                    subagent_events::on_subagent_iteration_started(
                        &bridge_ctx,
                        &mut emit_seq,
                        round,
                        agent_id,
                        task_id,
                        iteration,
                        max_iterations,
                        extended_policy,
                    );
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
                    subagent_events::on_subagent_tool_call_started(
                        &bridge_ctx,
                        &mut emit_seq,
                        &mut child_tool_counts,
                        round,
                        agent_id,
                        task_id,
                        call_id,
                        tool_name,
                        arguments,
                        iteration,
                        display_label,
                        display_detail,
                    );
                }
                AgentProgress::SubagentToolCallCompleted {
                    agent_id,
                    task_id,
                    call_id,
                    tool_name,
                    success,
                    output_chars,
                    output,
                    arguments,
                    elapsed_ms,
                    iteration,
                    failure,
                    display_label,
                    display_detail,
                    structured,
                } => {
                    subagent_events::on_subagent_tool_call_completed(
                        &bridge_ctx,
                        &mut emit_seq,
                        round,
                        agent_id,
                        task_id,
                        call_id,
                        tool_name,
                        success,
                        output_chars,
                        output,
                        arguments,
                        elapsed_ms,
                        iteration,
                        failure,
                        display_label,
                        display_detail,
                        structured,
                    );
                }
                AgentProgress::SubagentTextDelta {
                    agent_id,
                    task_id,
                    delta,
                    iteration,
                } => {
                    subagent_events::on_subagent_text_delta(
                        &bridge_ctx,
                        &mut emit_seq,
                        round,
                        agent_id,
                        task_id,
                        delta,
                        iteration,
                    );
                }
                AgentProgress::SubagentThinkingDelta {
                    agent_id,
                    task_id,
                    delta,
                    iteration,
                } => {
                    subagent_events::on_subagent_thinking_delta(
                        &bridge_ctx,
                        &mut emit_seq,
                        round,
                        agent_id,
                        task_id,
                        delta,
                        iteration,
                    );
                }
                AgentProgress::TextDelta { delta, iteration } => {
                    timing.text_delta(&delta, iteration, &request_id);
                    // Buffer the round's narration so it can be flushed as an
                    // interim bubble if a tool call closes this round.
                    pending_narration.push_str(&delta);
                    if !metadata.hold_text_stream {
                        text_events::publish_text_delta(
                            &dispatch_ctx,
                            &mut emit_seq,
                            &client_id,
                            &thread_id,
                            &request_id,
                            iteration,
                            delta,
                        );
                    }
                }
                AgentProgress::ThinkingDelta { delta, iteration } => {
                    publish_seq_stamped(
                        &dispatch_ctx,
                        &mut emit_seq,
                        WebChannelEvent {
                            event: "thinking_delta".to_string(),
                            client_id: client_id.clone(),
                            thread_id: thread_id.clone(),
                            request_id: request_id.clone(),
                            round: Some(iteration),
                            delta: Some(delta),
                            delta_kind: Some("thinking".to_string()),
                            ..Default::default()
                        },
                    );
                }
                AgentProgress::ToolCallArgsDelta {
                    call_id,
                    tool_name,
                    delta,
                    iteration,
                } => {
                    publish_seq_stamped(
                        &dispatch_ctx,
                        &mut emit_seq,
                        WebChannelEvent {
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
                        },
                    );
                }
                AgentProgress::TurnCompleted { iterations, .. } => {
                    parent_completed = true;
                    timing.done(iterations, MIN_INTERIM_NARRATION_CHARS, &request_id);
                    if let Ok(mut guard) = timing_snapshot_for_task.lock() {
                        *guard = Some(timing.snapshot());
                    }
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
                    // Every event the parent queued before `TurnCompleted` has
                    // now been forwarded, in order: release a caller waiting
                    // to publish the terminal `chat_done`.
                    let _ = drained_tx.send(true);
                    log::debug!(
                        "[web_channel][bridge] drained parent events_seen={events_seen} request_id={request_id}"
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

                    // Live cost readout: throttled so a fast multi-round turn
                    // doesn't flood the socket with one `turn_cost` per model
                    // call. `TurnCostUpdated` is the parent's cumulative
                    // rollup only — it carries no per-sub-agent breakdown, so
                    // `subagents` stays empty here; the final `chat_done.usage`
                    // (built from `LastTurnUsage` at delivery) is still where
                    // sub-agent attribution shows up.
                    if turn_cost_throttle.should_emit() {
                        publish_seq_stamped(
                            &dispatch_ctx,
                            &mut emit_seq,
                            WebChannelEvent {
                                event: "turn_cost".to_string(),
                                client_id: client_id.clone(),
                                thread_id: thread_id.clone(),
                                request_id: request_id.clone(),
                                round: Some(iteration),
                                usage: Some(crate::web_chat::TurnUsagePayload {
                                    input_tokens,
                                    output_tokens,
                                    cached_input_tokens,
                                    // A live rollup of the known spend so far;
                                    // the turn's final figure (or `null`)
                                    // arrives on `chat_done`.
                                    cost_usd: Some(total_usd),
                                    cost_source: tinyagents_harness::cost::CostSource::Charged,
                                    context_window: 0,
                                    context_tokens: 0,
                                    subagents: Vec::new(),
                                }),
                                ..Default::default()
                            },
                        );
                    }
                }
                AgentProgress::ToolCallProgress { call_id, message } => {
                    let ids = (client_id.as_str(), thread_id.as_str(), request_id.as_str());
                    tool_progress::publish(&dispatch_ctx, &mut emit_seq, ids, call_id, message);
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
                         in={input_tokens} out={output_tokens} cost_usd={cost_usd:?} request_id={request_id}"
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
        // The response presenter waits only briefly for this signal before
        // publishing an error. Trace export is best-effort I/O and must not
        // hold up terminal delivery after the progress stream closed.
        let _ = drained_tx.send(true);
        if let Some(mut collector) = span_collector.take() {
            collector.finish(unix_epoch_ms());
            let live_spans = collector.spans().to_vec();
            let journal_export = if let Some(trace_ctx) = journal_trace_ctx.take() {
                super::journal_shadow::shadow_compare_journal_projection(
                    &dispatch_ctx,
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
                crate::agent::progress_tracing::export_run_trace_from_journal(
                    &config,
                    &trace_ctx,
                    &observations,
                    run_telemetry.as_ref(),
                    &live_spans,
                )
                .await;
            } else {
                crate::agent::progress_tracing::export_run_trace(&config, &live_spans).await;
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
    ProgressBridgeHandle {
        drained: drained_rx,
        timing: timing_snapshot,
    }
}

#[cfg(test)]
#[path = "progress_bridge_tests.rs"]
mod tests;
