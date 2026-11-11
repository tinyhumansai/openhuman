use super::*;

/// Spawn a background task that reads [`AgentProgress`] events from the
/// agent turn loop and translates them into [`WebChannelEvent`]s tagged
/// with the correct client/thread/request IDs. The task runs until the
/// sender is dropped (i.e. when the agent turn finishes).
pub(crate) fn spawn_progress_bridge(
    mut rx: tokio::sync::mpsc::Receiver<crate::progress::AgentProgress>,
    client_id: String,
    thread_id: String,
    request_id: String,
    turn_state_store: std::sync::Arc<dyn TurnStates>,
    metadata: ChatRequestMetadata,
    config: crate::config::Config,
    dispatch_ctx: crate::core::runtime::DispatchCtx,
) -> ProgressBridgeHandle {
    use crate::progress::AgentProgress;
    use std::collections::HashMap;
    use tinyagents_session::run_ledger::{
        AgentRunKind, AgentRunStatus, AgentRunUpsert, RunEventAppend, RunTelemetryUpsert,
    };

    let (drained_tx, drained_rx) = tokio::sync::watch::channel(false);
    let timing_snapshot: std::sync::Arc<
        std::sync::Mutex<Option<crate::web_chat::turn_timing::TurnTimingSnapshot>>,
    > = std::sync::Arc::new(std::sync::Mutex::new(None));
    let timing_snapshot_for_task = timing_snapshot.clone();
    dispatch_ctx.clone().spawn(async move {
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
        // Accumulates the parent agent's streamed narration for the current
        // round. When a tool call closes the round, this leading narration is
        // flushed as an interim chat bubble (`chat_interim`) so it persists in
        // the thread instead of vanishing on settle — the final answer arrives
        // separately via `deliver_response` and is never part of this buffer
        // (it belongs to the terminal round, which ends with no tool call).
        let mut pending_narration = String::new();
        let mut timing = crate::web_chat::turn_timing::TurnTiming::start();
        let mut turn_cost_throttle = crate::web_chat::turn_timing::TurnCostThrottle::new();
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
        let mut journal_trace_ctx =
            trace::trace_context(&config, &metadata, &client_id, &thread_id, &request_id).await;

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
            include!("progress_bridge_event_match.rs");
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
        if let Some(trace_ctx) = journal_trace_ctx.take() {
            crate::core::observability::progress_tracing::export_run_trace_from_journal(
                &dispatch_ctx,
                &config,
                &request_id,
                trace_ctx,
            )
            .await;
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
