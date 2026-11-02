//! One turn's progress, projected: the single consumer of a turn's
//! `AgentProgress` stream.
//!
//! Every event is mapped to the canonical output vocabulary
//! ([`progress_map`](super::progress_map)) and published through the web
//! adapter ([`Surface`]); the same pass mirrors the turn into the `TurnStates`
//! port, the run ledger and the tracing export. The turn runner calls
//! [`ProgressProjection::on_event`] inline and in order, and the Flow Canvas
//! copilot drives the same projection from a channel through
//! [`spawn_progress_bridge`].

use std::time::Instant;

use tinyagents_session::port::TurnStates;
use tinyagents_session::turn_state::TurnStateMirror;
use tinychannels_bus::web::{
    ChatRequestMetadata, TurnCostThrottle, TurnTimingSnapshot, TurnTimingState,
};

use super::progress_ledger::{for_turn, Ledger};
use super::progress_map as map;
use super::surface::{Surface, TurnIds};
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::threads::turn_state::mirror::ObserveProgress;
use tinyagents_harness::progress::AgentProgress;

#[path = "progress_subagent.rs"]
mod subagent;
#[path = "progress_trace.rs"]
mod trace;

/// Cadence of the `inference_heartbeat` liveness beat while a turn is in
/// flight (issue #4270). The frontend silence timer only fires after ~120s with
/// no progress signal of any kind; a long prefill, or a reasoning model that
/// buffers server-side, can legitimately stream nothing for minutes. A beat
/// every 20s rides the same socket as the real events, so it keeps the timer
/// armed while work is progressing yet stops the instant the socket dies.
pub(super) const INFERENCE_HEARTBEAT_SECS: u64 = 20;

/// Minimum trimmed length for the parent's leading narration to flush as a
/// `chat_interim` event when the round's first tool call starts. Any non-empty
/// narration flushes: `chat_interim` also resets the frontend's live preview
/// for the next round, so an unflushed "Let me check." would have the next
/// round's text appended straight onto it.
const MIN_INTERIM_NARRATION_CHARS: usize = 1;

/// Current wall-clock time as Unix-epoch milliseconds (tracing spans and
/// `WebChannelEvent.ts`). Saturates to `0` if the clock is before the epoch.
pub fn unix_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The trimmed narration to surface as an interim bubble, or `None` when it is
/// empty. Pure so the threshold is unit-testable.
fn interim_narration_text(buffer: &str) -> Option<String> {
    let trimmed = buffer.trim();
    (trimmed.chars().count() >= MIN_INTERIM_NARRATION_CHARS).then(|| trimmed.to_owned())
}

/// The state one turn's progress accumulates while it is projected.
pub(crate) struct ProgressProjection {
    surface: Surface,
    ids: TurnIds,
    config: Config,
    ctx: DispatchCtx,
    metadata: ChatRequestMetadata,
    started: Instant,
    round: u32,
    /// The parent's streamed narration for the current round, flushed as an
    /// interim bubble when a tool call closes the round.
    pending_narration: String,
    timing: TurnTimingState,
    cost_throttle: TurnCostThrottle,
    ledger: Ledger,
    turn_state: Option<TurnStateMirror>,
    journal_trace_ctx: Option<tinyagents_harness::observability::trace_export::TraceContext>,
    timing_snapshot: Option<TurnTimingSnapshot>,
    /// Set once `TurnStarted` arrives and cleared at `TurnCompleted`: the
    /// heartbeat only beats for a live turn.
    turn_active: bool,
}

impl ProgressProjection {
    pub(crate) async fn new(
        ctx: &DispatchCtx,
        ids: TurnIds,
        metadata: ChatRequestMetadata,
        config: Config,
        turn_states: std::sync::Arc<dyn TurnStates>,
    ) -> Self {
        log::debug!(
            "[web_channel][bridge] spawned client_id={} thread_id={} request_id={} speak_reply={:?} source={:?} session_id={:?} hold_text_stream={}",
            ids.client_id,
            ids.thread_id,
            ids.request_id,
            metadata.speak_reply,
            metadata.source,
            metadata.session_id,
            metadata.hold_text_stream,
        );
        let journal_trace_ctx = trace::trace_context(
            &config,
            &metadata,
            &ids.client_id,
            &ids.thread_id,
            &ids.request_id,
        );
        let ledger = for_turn(
            &config.workspace_dir,
            &ids.client_id,
            &ids.thread_id,
            &ids.request_id,
        );
        Self {
            surface: Surface::new(ctx, ids.clone(), config.clone()).await,
            turn_state: Some(TurnStateMirror::new(
                turn_states,
                ids.thread_id.clone(),
                ids.request_id.clone(),
            )),
            ids,
            config,
            ctx: ctx.clone(),
            metadata,
            started: Instant::now(),
            round: 0,
            pending_narration: String::new(),
            timing: TurnTimingState::default(),
            cost_throttle: TurnCostThrottle::default(),
            ledger,
            journal_trace_ctx,
            timing_snapshot: None,
            turn_active: false,
        }
    }

    /// The turn's surface, for the terminal event the runner owes it.
    pub(crate) fn surface(&self) -> Surface {
        self.surface.clone()
    }

    /// The first-token / first-tool / total timings, once `TurnCompleted` has
    /// been seen.
    pub(crate) fn timing_snapshot(&self) -> Option<TurnTimingSnapshot> {
        self.timing_snapshot
    }

    fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// One liveness beat; nothing before `TurnStarted` or after the parent
    /// completed.
    pub(crate) async fn heartbeat(&self) {
        if self.turn_active {
            log::trace!(
                "[web_channel][bridge] inference_heartbeat thread_id={} request_id={}",
                self.ids.thread_id,
                self.ids.request_id,
            );
            self.surface.output(map::inference_heartbeat()).await;
        }
    }

    async fn flush_interim_narration(&mut self, round: u32) {
        let text = std::mem::take(&mut self.pending_narration);
        let Some(narration) = interim_narration_text(&text) else {
            return;
        };
        log::debug!(
            "[web_channel][bridge] chat_interim round={} chars={} request_id={}",
            round,
            narration.chars().count(),
            self.ids.request_id,
        );
        self.surface.interim(round, narration).await;
    }

    /// Project one event of the turn, in order.
    pub(crate) async fn on_event(&mut self, event: AgentProgress) {
        if let Some(mirror) = self.turn_state.as_mut() {
            mirror.observe(&event);
        }
        if let AgentProgress::ToolCallStarted { iteration, .. } = &event {
            if self.timing.tool_call(self.elapsed_ms()) {
                log::info!(
                    "[web_channel][bridge] time-to-first-visible kind=tool_call first_tool_ms={} round={iteration} request_id={}",
                    self.elapsed_ms(),
                    self.ids.request_id
                );
            }
            self.flush_interim_narration(*iteration).await;
        }
        let ledger_context = self.ledger.observe(&event).await;
        match event {
            AgentProgress::TurnStarted => {
                self.turn_active = true;
                self.surface.output(map::inference_start()).await;
            }
            AgentProgress::IterationStarted {
                iteration,
                max_iterations,
            } => {
                self.round = iteration;
                self.surface
                    .output(map::iteration_start(iteration, max_iterations))
                    .await;
            }
            AgentProgress::ToolCallStarted {
                call_id,
                tool_name,
                arguments,
                iteration,
                display_label,
                display_detail,
            } => {
                log::debug!(
                    "[web_channel][bridge] tool_call round={iteration} tool={tool_name} call_id={call_id} request_id={}",
                    self.ids.request_id
                );
                self.surface
                    .output(map::tool_started(
                        call_id,
                        tool_name,
                        arguments,
                        iteration,
                        display_label,
                        display_detail,
                    ))
                    .await;
            }
            AgentProgress::ToolCallCompleted { .. } => self.on_tool_completed(event).await,
            AgentProgress::TextDelta { delta, iteration } => {
                if self.timing.text_delta(self.elapsed_ms(), &delta, iteration) {
                    log::info!(
                        "[web_channel][bridge] time-to-first-visible kind=text first_text_ms={} round={iteration} request_id={}",
                        self.elapsed_ms(),
                        self.ids.request_id
                    );
                }
                self.pending_narration.push_str(&delta);
                if !self.metadata.hold_text_stream {
                    self.surface.output(map::text_delta(delta, iteration)).await;
                }
            }
            AgentProgress::ThinkingDelta { delta, iteration } => {
                self.surface
                    .output(map::thinking_delta(delta, iteration))
                    .await;
            }
            AgentProgress::ToolCallArgsDelta {
                call_id,
                tool_name,
                delta,
                iteration,
            } => {
                self.surface
                    .output(
                        map::turn_event(AgentProgress::ToolCallArgsDelta {
                            call_id,
                            tool_name,
                            delta,
                            iteration,
                        })
                        .expect("tool argument progress maps to a channel event"),
                    )
                    .await;
            }
            AgentProgress::ToolCallProgress { call_id, message } => {
                log::trace!(
                    "[web_channel][bridge] tool_progress call_id={call_id} request_id={}",
                    self.ids.request_id
                );
                self.surface
                    .output(map::tool_progress(call_id, message))
                    .await;
            }
            AgentProgress::TurnCompleted { iterations, .. } => {
                self.on_turn_completed(iterations).await;
            }
            AgentProgress::TurnCostUpdated {
                model,
                iteration,
                input_tokens,
                output_tokens,
                cached_input_tokens,
                total_usd,
            } => {
                log::debug!(
                    "[web_channel] turn cost update model={model} iter={iteration} in={input_tokens} out={output_tokens} cached_in={cached_input_tokens} total_usd={total_usd:.4} thread_id={}",
                    self.ids.thread_id
                );
                // A live readout, throttled so a fast multi-round turn does not
                // flood the socket. The turn's final figure arrives on
                // `chat_done`.
                if self.cost_throttle.should_emit(self.elapsed_ms()) {
                    self.surface
                        .output(map::cost_update(
                            model,
                            iteration,
                            input_tokens,
                            output_tokens,
                            cached_input_tokens,
                            total_usd,
                        ))
                        .await;
                }
            }
            AgentProgress::TurnContent { .. } => {
                // Prompt/reply content is attached to the trace span by the
                // collector above.
            }
            AgentProgress::ModelCallCompleted {
                model,
                iteration,
                input_tokens,
                output_tokens,
                cost_usd,
                ..
            } => {
                // Per-call usage belongs to the span collector (one Langfuse
                // generation per call); the socket stays on the cumulative
                // `TurnCostUpdated` rollup.
                log::debug!(
                    "[web_channel][bridge] model_call_completed model={model} iter={iteration} in={input_tokens} out={output_tokens} cost_usd={cost_usd:?} request_id={}",
                    self.ids.request_id
                );
            }
            AgentProgress::ContextCompacting => {
                let round = (self.round > 0).then_some(self.round);
                self.surface.output(map::context_compacting(round)).await;
            }
            other => self.on_subagent_event(other, ledger_context).await,
        }
    }

    async fn on_tool_completed(&mut self, event: AgentProgress) {
        if let AgentProgress::ToolCallCompleted {
            call_id,
            tool_name,
            success,
            elapsed_ms,
            iteration,
            structured,
            ..
        } = &event
        {
            log::debug!(
                "[web_channel][bridge] tool_result round={iteration} tool={tool_name} call_id={call_id} success={success} elapsed_ms={elapsed_ms} has_structured={} request_id={}",
                structured.is_some(),
                self.ids.request_id
            );
        }
        if let Some(output) = map::tool_completed(event) {
            self.surface.output(output).await;
        }
    }

    async fn on_turn_completed(&mut self, iterations: u32) {
        let snapshot = self.timing.snapshot(self.elapsed_ms());
        log::info!(
            "[web_channel][bridge] time-to-first-visible kind=turn_done total_ms={} first_text_ms={:?} first_tool_ms={:?} round_one_narration_chars={} interim_threshold={MIN_INTERIM_NARRATION_CHARS} iterations={iterations} request_id={}",
            self.elapsed_ms(),
            snapshot.first_token_ms,
            snapshot.first_tool_ms,
            snapshot.round_one_narration_chars,
            self.ids.request_id
        );
        self.timing_snapshot = Some(snapshot);
        // The turn is done: stop liveness beats.
        self.turn_active = false;
        log::debug!(
            "[web_channel] turn completed after {iterations} iteration(s) thread_id={} request_id={} speak_reply={:?} source={:?} session_id={:?}",
            self.ids.thread_id,
            self.ids.request_id,
            self.metadata.speak_reply,
            self.metadata.source,
            self.metadata.session_id,
        );
    }

    /// The stream is over: close the turn-state mirror, record an interrupted
    /// run if the parent never completed, and hand the run's trace to the
    /// configured sink. Best-effort and gated; never affects the turn outcome.
    pub(crate) async fn finish(&mut self) {
        if let Some(mirror) = self.turn_state.take() {
            mirror.finish();
        }
        self.ledger.finish().await;
        // #3886: hand the run's trace, reprojected from the durable journal,
        // to the configured tracing sink.
        if let Some(trace_ctx) = self.journal_trace_ctx.take() {
            crate::core::observability::progress_tracing::export_run_trace_from_journal(
                &self.ctx,
                &self.config,
                &self.ids.request_id,
                trace_ctx,
            )
            .await;
        }
        log::debug!(
            "[web_channel][bridge] exit client_id={} thread_id={} request_id={} round={} events_seen={}",
            self.ids.client_id,
            self.ids.thread_id,
            self.ids.request_id,
            self.round,
            self.ledger.events_seen(),
        );
        self.surface.close().await;
    }
}

/// Drive a projection from a progress channel until the turn's sender drops.
///
/// For turns that run outside the turn runner (the Flow Canvas copilot's
/// builder and scout turns) and want the same live stream a chat turn has.
pub(crate) fn spawn_progress_bridge(
    rx: tokio::sync::mpsc::Receiver<AgentProgress>,
    client_id: String,
    thread_id: String,
    request_id: String,
    turn_state_store: std::sync::Arc<dyn TurnStates>,
    metadata: ChatRequestMetadata,
    config: Config,
    dispatch_ctx: DispatchCtx,
) {
    drop(spawn_progress_bridge_with_heartbeat_interval(
        rx,
        client_id,
        thread_id,
        request_id,
        turn_state_store,
        metadata,
        config,
        dispatch_ctx,
        std::time::Duration::from_secs(INFERENCE_HEARTBEAT_SECS),
    ));
}

#[allow(clippy::too_many_arguments)]
fn spawn_progress_bridge_with_heartbeat_interval(
    mut rx: tokio::sync::mpsc::Receiver<AgentProgress>,
    client_id: String,
    thread_id: String,
    request_id: String,
    turn_state_store: std::sync::Arc<dyn TurnStates>,
    metadata: ChatRequestMetadata,
    config: Config,
    dispatch_ctx: DispatchCtx,
    heartbeat_interval: std::time::Duration,
) -> tokio::task::JoinHandle<()> {
    dispatch_ctx.clone().spawn(async move {
        let ids = TurnIds {
            client_id,
            thread_id,
            request_id,
        };
        let mut projection =
            ProgressProjection::new(&dispatch_ctx, ids, metadata, config, turn_state_store).await;
        let mut beat = tokio::time::interval(heartbeat_interval);
        beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        beat.tick().await;
        enum Next {
            Progress(Option<AgentProgress>),
            Heartbeat,
        }
        loop {
            let next = tokio::select! {
                // Real events before the timer, so a busy turn never starves
                // event handling to emit a beat.
                biased;
                event = rx.recv() => Next::Progress(event),
                _ = beat.tick() => Next::Heartbeat,
            };
            match next {
                // Keep the projection future outside `select!`: module calls
                // can take longer than one heartbeat interval, and selecting
                // the timer while awaiting them would cancel the progress
                // event that already won the receive branch.
                Next::Progress(Some(event)) => {
                    projection.on_event(event).await;
                }
                Next::Progress(None) => break,
                Next::Heartbeat => projection.heartbeat().await,
            }
        }
        projection.finish().await;
    })
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
