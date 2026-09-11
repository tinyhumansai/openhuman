//! Host capability: turns the crate's coarse [`ProgressEvent`] stream into
//! OpenHuman's richer [`AgentProgress`] events.
//!
//! Adapts `crate::openhuman::agent::progress` (the `AgentProgress` UI contract)
//! for `tinyagents_harness::host::ProgressSink`. This is Phase 4 of
//! `docs/specs/plan-agents.md`.
//!
//! # Why this file is the boundary
//!
//! `AgentProgress` is the **host's** UI contract — the chat processing
//! timeline, the subagent drawer, the cost footer and the trace exporter all
//! read it. The crate deliberately keeps `ProgressEvent` at five variants and
//! says so in its own module docs: a presentation-shaped field added there
//! becomes a compatibility surface for every consumer of a redistributed
//! crate. So the projection runs here, host-side, and `AgentProgress` never
//! crosses into `vendor/tinyagents`.
//!
//! # Why it forwards to an mpsc channel and not to `publish_web_channel_event`
//!
//! The obvious-looking shortcut — build a `WebChannelEvent` and publish it
//! straight onto the web-channel bus — is wrong here. Everything that makes a
//! web-channel progress event correct is owned by
//! [`crate::openhuman::web_chat::progress_bridge::spawn_progress_bridge`]: the
//! per-request monotonic `seq` stamp the frontend dedups on, the
//! `TurnStateMirror` snapshot, the run-ledger upserts, the tracing span
//! collector, and the client/thread/request routing ids that a `ProgressEvent`
//! does not carry at all. A second independent producer of "what is happening
//! now" would drift from the bridge's ordering and from the persisted turn
//! state, and the drift would only show up as a mis-rendered timeline.
//!
//! So this sink writes `AgentProgress` into the same
//! `mpsc::Sender<AgentProgress>` the existing agent turn loop uses
//! (`Agent::set_on_progress`), and the established bridge does the publishing.
//! `publish_web_channel_event` is still the terminal step — one hop further
//! down, where it already lives.
//!
//! # Contract mismatches, and how they are resolved
//!
//! * **`emit` cannot fail, but not every event is equally droppable.** Token
//!   deltas use `try_send` and are *dropped* when the bridge is behind, per the
//!   crate's "dropping progress events is always preferable to slowing the
//!   turn" rule. Lifecycle events (`TurnStarted`, `ToolCallStarted`,
//!   `TurnCompleted`) are not interchangeable with them: the bridge is a state
//!   machine, so a lost one leaves a tool row stuck in `running` forever rather
//!   than costing a UI tick. They therefore wait up to
//!   [`LIFECYCLE_SEND_GRACE`] for room — bounded, never indefinite, because an
//!   unbounded await on this shared channel is the documented subagent-stall
//!   flake (`tool_progress.rs::emit`). Nothing here blocks a turn or panics.
//! * **The coarse stream has no iteration boundary.** `AgentProgress` carries a
//!   1-based `iteration` on almost every variant; `ProgressEvent` has no
//!   equivalent. The sink derives one **per run**: a *batch* of consecutive
//!   `ToolCall`s is one iteration, and model output (`Token`) closes the batch
//!   so the next call opens a new one. Counting every call instead would report a
//!   turn that requested two tools in parallel as three iterations. Still a
//!   *lower bound* — two sequential tool batches with no tokens between them are
//!   indistinguishable from one parallel batch. See the `TODO(phase4)` below.
//! * **One sink may serve several runs.** `Arc<OpenHumanProgressSink>` is
//!   explicitly shared across concurrent sub-runs, so all counters are keyed by
//!   [`RunId`] and only the **first run seen** projects top-level `TurnStarted` /
//!   `TurnCompleted`. A shared counter would let a child's tool calls renumber
//!   the parent's iterations, and a child's `Finished` would tell the progress
//!   bridge the whole request had completed while other runs were still
//!   emitting.
//! * **`ToolCallCompleted` is emitted from `ProgressEvent::ToolCallFinished`.**
//!   This previously read "none is ever emitted, and none can be": the crate's
//!   stream had no tool-completion milestone, so a row opened by `ToolCall`
//!   could never be closed truthfully, and synthesising one would have asserted
//!   `success: true` for a tool that may have failed — corrupting the timeline
//!   and the trace exporter rather than merely leaving them incomplete. That
//!   gap was filed as `tinyagents#88` and is now closed.
//!
//!   Two of the three fields `AgentProgress::ToolCallCompleted` needs still do
//!   not travel on the closing event — the tool *name* and the *duration* — so
//!   both are captured when the call opens (`RunState::open_calls`) and
//!   recovered on close. The recorded iteration is used rather than the live
//!   counter, because model output between a call and its result advances the
//!   round and would otherwise file the tool under the wrong one.
//!
//!   A close with no matching open call is **dropped, not emitted**: without
//!   the opening record there is no honest tool name or duration to report, and
//!   the original reasoning still applies — a missing row is recoverable, a
//!   fabricated one is not.
//!
//!   `arguments` stays `None`. The crate emits arguments on neither event, so
//!   there is nothing to backfill the span input with, and `None` says "not
//!   captured" rather than "there were none".
//! * **`ProgressEvent::Error` has no `AgentProgress` counterpart.** The host
//!   enum models a turn's failure through the turn's own `Err` return (which
//!   `web_chat::ops` renders as `chat_error`), not through a progress event.
//!   Synthesising a `TurnCompleted` here would report a failed turn as a
//!   successful one to the ledger and the timeline, so the sink logs the
//!   failure and forwards nothing.
//! * **`ProgressEvent::Finished { usage }` is not a cost event.**
//!   `AgentProgress::TurnCostUpdated` requires a model id and a USD total that
//!   the crate event does not carry, and OpenHuman's authoritative cost figures
//!   come from the inference layer's charged amounts. Fabricating a
//!   `model: ""` / `total_usd: 0.0` update would silently under-report in the
//!   chat cost footer, so the usage is logged and only `TurnCompleted` is
//!   forwarded.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::Sender;

use tinyagents_harness::host::{ProgressEvent, ProgressSink};

use crate::openhuman::agent::progress::AgentProgress;

/// How long a **lifecycle** event may wait for room on a full channel.
///
/// Sized to ride out the transient window a burst of token deltas opens, while
/// staying far below anything a user or a parent turn would perceive as a
/// stall. Deltas never wait at all.
const LIFECYCLE_SEND_GRACE: std::time::Duration = std::time::Duration::from_millis(50);

/// Iteration bookkeeping for one run.
// No longer `Copy`: `open_calls` owns per-call state, so the struct is moved or
// borrowed rather than duplicated. It is only ever touched through the `runs()`
// mutex guard, which never needed a copy.
#[derive(Debug, Default, Clone)]
struct RunState {
    /// Model iterations observed so far, 0 before the first tool call.
    rounds: u32,
    /// Whether the events seen most recently were a run of `ToolCall`s.
    ///
    /// A model can request several tools in one response, and the runtime emits
    /// one event per tool — so consecutive calls are one iteration, not several.
    in_tool_batch: bool,
    /// Tool calls opened but not yet closed, keyed by call id.
    ///
    /// `ProgressEvent::ToolCallFinished` carries neither the tool name nor a
    /// duration, so both are captured when the call *opens* and recovered here.
    /// Keyed by call id rather than kept as a single slot because a model can
    /// request several tools in one response and they close in any order.
    open_calls: HashMap<String, OpenToolCall>,
}

/// What the opening `ToolCall` knew and the closing event does not.
#[derive(Debug, Clone)]
struct OpenToolCall {
    tool_name: String,
    started_at: Instant,
    /// Iteration the call was attributed to when it opened.
    ///
    /// Recorded rather than re-read at completion: model output between the
    /// call and its result advances the counter, so reading it late would
    /// report the tool under the wrong round.
    iteration: u32,
}

/// Forwards crate progress into an OpenHuman [`AgentProgress`] channel.
///
/// Construct one per turn with the same sender that would otherwise be handed
/// to `Agent::set_on_progress`, so the existing
/// `web_chat::progress_bridge` consumer sees an identical event stream
/// regardless of which runtime produced it.
///
/// Cheap to clone-by-`Arc`: the crate's blanket
/// `impl ProgressSink for Arc<T>` makes `Arc<OpenHumanProgressSink>` usable
/// wherever a sink is wanted, which is how a sink shared by concurrent
/// sub-runs is passed around.
pub struct OpenHumanProgressSink {
    /// The per-request progress channel the turn loop's consumer owns.
    ///
    /// Bounded by whoever created it. Backpressure is handled by dropping, not
    /// by awaiting — see the module docs.
    tx: Sender<AgentProgress>,

    /// Per-run progress state, keyed by [`RunId`].
    ///
    /// Not a single counter, because the type doc explicitly supports one
    /// `Arc<OpenHumanProgressSink>` shared by concurrent sub-runs. With shared
    /// state, a child's `Started` would reset the parent's iteration count and
    /// a child's tool calls would advance it — so the parent's own events would
    /// carry a number describing somebody else's work.
    runs: Mutex<HashMap<String, RunState>>,

    /// The first run id observed, treated as the request's root.
    ///
    /// Only the root's lifecycle is projected as *top-level* `TurnStarted` /
    /// `TurnCompleted`. Without this, the first sub-run to finish would tell the
    /// progress bridge the whole request was complete while other runs were
    /// still going — the bridge would close out the turn and everything after it
    /// would render against a finished timeline.
    root_run: Mutex<Option<String>>,

    /// How many events were dropped because the channel was full or closed.
    ///
    /// Exposed via [`Self::dropped`] so a diagnostic can distinguish "the UI
    /// showed nothing because nothing happened" from "the UI showed nothing
    /// because the bridge fell behind". Never surfaced to the turn.
    dropped: AtomicU64,
}

impl OpenHumanProgressSink {
    /// Wraps a per-request `AgentProgress` sender.
    pub fn new(tx: Sender<AgentProgress>) -> Self {
        Self {
            tx,
            runs: Mutex::new(HashMap::new()),
            root_run: Mutex::new(None),
            dropped: AtomicU64::new(0),
        }
    }

    /// Number of events dropped so far (full or closed channel).
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Locks the per-run state map, tolerating a poisoned mutex.
    fn runs(&self) -> std::sync::MutexGuard<'_, HashMap<String, RunState>> {
        self.runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether `run` is the request's root, claiming the slot if it is vacant.
    ///
    /// First run seen wins. The coarse stream carries no parent/child edge, so
    /// arrival order is the only signal available — and the root's `Started` is
    /// necessarily first, since a sub-run cannot begin before the turn that
    /// spawns it.
    fn is_root_run(&self, run: &str) -> bool {
        let mut root = self
            .root_run
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match root.as_deref() {
            Some(existing) => existing == run,
            None => {
                *root = Some(run.to_string());
                true
            }
        }
    }

    /// The current 1-based iteration index for `run`.
    ///
    /// Starts at `1` — a turn is in its first round before it has called any
    /// tool.
    fn iteration_for(&self, run: &str) -> u32 {
        self.runs()
            .get(run)
            .map(|state| state.rounds.saturating_add(1))
            .unwrap_or(1)
    }

    /// Advances `run`'s iteration for a tool call and returns the 1-based index
    /// the call belongs to.
    ///
    /// A *batch* of tool calls is one model iteration. The runtime emits one
    /// `ToolCall` per requested tool, so a model that asks for two tools in
    /// parallel produces two consecutive events that belong to the same
    /// iteration — counting each one would report three iterations for a
    /// two-parallel-calls-then-answer turn and mislabel the second call. Only
    /// the first call after non-tool activity opens a new iteration.
    ///
    /// Still a lower bound, not a fact: the coarse stream has no explicit model
    /// boundary, so a turn that emits no tokens between two sequential tool
    /// batches cannot be distinguished from one parallel batch.
    fn advance_for_tool_call(&self, run: &str) -> u32 {
        let mut runs = self.runs();
        let state = runs.entry(run.to_string()).or_default();
        if !state.in_tool_batch {
            state.rounds = state.rounds.saturating_add(1);
            state.in_tool_batch = true;
        }
        state.rounds
    }

    /// Records that non-tool activity was seen for `run`, closing any open tool
    /// batch so the next `ToolCall` starts a fresh iteration.
    fn note_model_activity(&self, run: &str) {
        self.runs()
            .entry(run.to_string())
            .or_default()
            .in_tool_batch = false;
    }

    /// Hands one **lifecycle** event to the channel, waiting briefly if the
    /// channel is momentarily full.
    ///
    /// Lifecycle events are not interchangeable with token deltas: the web
    /// progress bridge is a state machine, so a lost `ToolCallStarted` or
    /// `TurnCompleted` leaves a tool row stuck in `running` forever, whereas a
    /// lost `TextDelta` is one missed UI tick. `turn/tools.rs::emit_progress`
    /// awaits its lifecycle sends for exactly that reason.
    ///
    /// It waits with a **bound** rather than awaiting outright, because the
    /// opposite failure is also real and also documented: the sink is a bounded
    /// channel shared by the orchestrator, every inline sub-agent, and their
    /// delta forwarders, and an unbounded `send().await` here can park a
    /// sub-agent's loop and hang the parent turn that is awaiting it — the
    /// subagent-stall flake `tool_progress.rs::emit` was written to avoid.
    /// [`LIFECYCLE_SEND_GRACE`] is the compromise: long enough to ride out the
    /// transient full-channel window a burst of deltas creates, far too short
    /// to stall a turn.
    async fn forward_lifecycle(&self, event: AgentProgress) {
        match self.tx.try_send(event) {
            Ok(()) => return,
            Err(TrySendError::Closed(dropped)) => {
                // No listener; waiting cannot help.
                let total = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                log::trace!(
                    "[tinyagents][progress] dropped lifecycle event on closed channel kind={:?} dropped_total={}",
                    std::mem::discriminant(&dropped),
                    total,
                );
                return;
            }
            Err(TrySendError::Full(event)) => {
                if self
                    .tx
                    .send_timeout(event, LIFECYCLE_SEND_GRACE)
                    .await
                    .is_ok()
                {
                    return;
                }
            }
        }

        let total = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
        log::warn!(
            "[tinyagents][progress] dropped a lifecycle event after waiting {:?} — the progress \
             bridge may now be out of sync (dropped_total={})",
            LIFECYCLE_SEND_GRACE,
            total,
        );
    }

    /// Hands one high-frequency event to the channel, dropping it if the
    /// channel cannot take it right now.
    ///
    /// This is the whole of the sink's failure handling for token deltas, and it
    /// deliberately has no error path out: `ProgressSink::emit` returns unit
    /// precisely so a dead UI socket can never fail a turn.
    fn forward(&self, event: AgentProgress) {
        match self.tx.try_send(event) {
            Ok(()) => {}
            Err(TrySendError::Full(dropped)) => {
                let total = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                log::debug!(
                    "[tinyagents][progress] dropped event on full channel kind={:?} dropped_total={}",
                    std::mem::discriminant(&dropped),
                    total,
                );
            }
            Err(TrySendError::Closed(dropped)) => {
                let total = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                log::trace!(
                    "[tinyagents][progress] dropped event on closed channel kind={:?} dropped_total={}",
                    std::mem::discriminant(&dropped),
                    total,
                );
            }
        }
    }
}

#[async_trait]
impl ProgressSink for OpenHumanProgressSink {
    /// Projects one [`ProgressEvent`] onto zero or one [`AgentProgress`]
    /// events and forwards it.
    ///
    /// Zero, for the two variants OpenHuman models elsewhere — see the module
    /// docs for `Error` and for the cost half of `Finished`. Never awaits
    /// anything that can block; the body is a counter bump and a `try_send`.
    async fn emit(&self, ev: ProgressEvent) {
        match ev {
            ProgressEvent::Started { run, thread, agent } => {
                // `TurnStarted` is a unit variant: the consumer already knows
                // its own client/thread/request ids (that is exactly why
                // `AgentProgress` carries no routing info), so run/thread/agent
                // are logged for correlation rather than forwarded.
                log::debug!(
                    "[tinyagents][progress] turn started run={} thread={:?} agent={}",
                    run,
                    thread.as_ref().map(|t| t.as_str()),
                    agent,
                );
                let is_root = self.is_root_run(run.as_str());
                self.runs()
                    .insert(run.as_str().to_string(), RunState::default());
                if !is_root {
                    // A sub-run beginning is not the request beginning. Emitting
                    // `TurnStarted` here would restart the parent's timeline.
                    log::debug!(
                        "[tinyagents][progress] sub-run started run={run}; not emitting a \
                         top-level TurnStarted"
                    );
                    return;
                }
                self.forward_lifecycle(AgentProgress::TurnStarted).await;
            }

            ProgressEvent::ToolCall { run, call, tool } => {
                // A tool call closes the current round, so the counter advances
                // first and the event is attributed to the round it belongs to.
                let iteration = self.advance_for_tool_call(run.as_str());
                log::debug!(
                    "[tinyagents][progress] tool_call run={} call={} tool={} iteration={}",
                    run,
                    call,
                    tool,
                    iteration,
                );
                // Remember what the closing event will not carry: the tool name,
                // a start instant, and the round this call belongs to.
                if let Some(state) = self.runs().get_mut(run.as_str()) {
                    state.open_calls.insert(
                        call.as_str().to_string(),
                        OpenToolCall {
                            tool_name: tool.clone(),
                            started_at: Instant::now(),
                            iteration,
                        },
                    );
                }
                self.forward_lifecycle(AgentProgress::ToolCallStarted {
                    call_id: call.as_str().to_string(),
                    tool_name: tool,
                    // The crate deliberately omits tool arguments from the
                    // progress side channel (they can be large and can carry
                    // untrusted or sensitive text). `Null` is already the
                    // documented shape on the tinyagents path — see
                    // `AgentProgress::ToolCallCompleted::arguments`, which
                    // backfills the span input for exactly this reason.
                    arguments: serde_json::Value::Null,
                    iteration,
                    // Server-computed timeline copy comes from
                    // `Tool::display_label` / `display_detail` at the tool
                    // registry, which this sink has no handle to; `None` tells
                    // the client to use its own formatter.
                    // TODO(phase4): resolve labels from the tool registry
                    // (`crate::openhuman::tools::traits::Tool::display_label`)
                    // once the sink is constructed with a registry handle.
                    display_label: None,
                    display_detail: None,
                })
                .await;
            }

            ProgressEvent::ToolCallFinished {
                run,
                call,
                success,
                output,
            } => {
                // The tool name / duration / round come from the opening
                // `ToolCall`; the crate's event carries none of them.
                let opened = self
                    .runs()
                    .get_mut(run.as_str())
                    .and_then(|state| state.open_calls.remove(call.as_str()));
                let Some(opened) = opened else {
                    // No matching open call: the run was torn down, or a close
                    // arrived twice. Emitting anyway would invent a tool name
                    // and a duration, so this is dropped and logged instead —
                    // a missing row is recoverable, a fabricated one is not.
                    log::debug!(
                        "[tinyagents][progress] tool_call_finished with no open call \
                         run={run} call={call} success={success}; dropping"
                    );
                    return;
                };

                let elapsed_ms = opened.started_at.elapsed().as_millis() as u64;
                log::debug!(
                    "[tinyagents][progress] tool_call_finished run={} call={} tool={} \
                     success={} elapsed_ms={} output_chars={}",
                    run,
                    call,
                    opened.tool_name,
                    success,
                    elapsed_ms,
                    output.chars().count(),
                );

                // Classified only on failure, from the tool's own output — the
                // same text `tools::status::classify` sees on the legacy path,
                // so the timeline renders an identical cause/next-action.
                // `timed_out: false` because the coarse stream does not
                // distinguish a timeout from any other failure; claiming one
                // would put a specific, wrong cause in front of a user.
                let failure = if success {
                    None
                } else {
                    Some(crate::openhuman::tools::status::classify(&output, false))
                };

                self.forward_lifecycle(AgentProgress::ToolCallCompleted {
                    call_id: call.as_str().to_string(),
                    tool_name: opened.tool_name,
                    success,
                    output_chars: output.chars().count(),
                    output,
                    // The crate emits no arguments on either event, so there is
                    // nothing to backfill the span input with. `None` says
                    // "not captured", which is what actually happened.
                    arguments: None,
                    elapsed_ms,
                    iteration: opened.iteration,
                    failure,
                })
                .await;
            }

            ProgressEvent::Token { run, text } => {
                // Model output closes any open tool batch: the next `ToolCall`
                // belongs to a new iteration.
                self.note_model_activity(run.as_str());
                let iteration = self.iteration_for(run.as_str());
                log::trace!(
                    "[tinyagents][progress] token run={} chars={} iteration={}",
                    run,
                    text.len(),
                    iteration,
                );
                self.forward(AgentProgress::TextDelta {
                    delta: text,
                    iteration,
                });
            }

            ProgressEvent::Finished { run, usage } => {
                let iterations = self.iteration_for(run.as_str());
                log::debug!(
                    "[tinyagents][progress] turn finished run={} iterations={} usage={:?}",
                    run,
                    iterations,
                    usage,
                );
                // TODO(phase4): the usage block is dropped rather than mapped
                // to `AgentProgress::TurnCostUpdated`, which needs a model id
                // and a USD total the crate event does not carry. The
                // authoritative cost path is
                // `crate::openhuman::agent::cost::TurnCost`, fed from the
                // inference layer's charged amounts; wiring usage through would
                // mean threading the resolved model + a cost estimate into this
                // sink rather than inventing `model: ""` / `total_usd: 0.0`.
                let is_root = self.is_root_run(run.as_str());
                self.runs().remove(run.as_str());
                if !is_root {
                    // A sub-run finishing is not the request finishing. This is
                    // the corruption that matters most: the bridge would close
                    // the turn out while other runs were still producing events.
                    log::debug!(
                        "[tinyagents][progress] sub-run finished run={run}; not emitting a \
                         top-level TurnCompleted"
                    );
                    return;
                }
                self.forward_lifecycle(AgentProgress::TurnCompleted { iterations })
                    .await;
            }

            ProgressEvent::Error { run, message } => {
                // Not forwarded on purpose. `AgentProgress` has no turn-level
                // failure variant — a failed turn surfaces through the turn's
                // own `Err`, which `web_chat::ops` renders as `chat_error` —
                // and emitting `TurnCompleted` here would record a failure as a
                // success in both the run ledger and the chat timeline.
                log::warn!("[tinyagents][progress] turn failed run={run} err={message}");
                // TODO(phase4): if the runtime ever becomes the only producer
                // of turn outcomes, this needs a real host-side failure event
                // (a new `AgentProgress` variant plus a `chat_error` mapping in
                // `web_chat::progress_bridge`), not a repurposed one.
            }
        }
    }
}

#[cfg(test)]
#[path = "progress_sink_tests.rs"]
mod tests;
