//! Forwards a running turn's `AgentProgress` stream into `WebChannelEvent`
//! socket events and mirrors it into the `TurnStates` port. Also emits the
//! `inference_heartbeat` liveness beat (see [`INFERENCE_HEARTBEAT_SECS`])
//! so a long silent prefill can't trip the frontend's ~120s silence timeout.

use serde_json::json;

use crate::threads::turn_state::mirror::ObserveProgress;
use crate::web_chat::{SubagentProgressDetail, WebChannelEvent};
use tinyagents_session::{port::TurnStates, turn_state::TurnStateMirror};

use super::types::ChatRequestMetadata;

#[path = "progress_bridge_narration.rs"]
mod narration;
#[cfg(test)]
use narration::interim_narration_text;
use narration::{flush_interim_narration, publish_seq_stamped, MIN_INTERIM_NARRATION_CHARS};
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
use wire_caps::{cap_wire_args, cap_wire_output};

#[path = "progress_bridge_ledger.rs"]
mod ledger;
pub(super) use ledger::{ledger_append_event, ledger_upsert_agent_run, ledger_upsert_telemetry};

include!("progress_bridge_attribution.rs");

#[path = "progress_bridge_spawn.rs"]
mod spawn;
pub(crate) use spawn::spawn_progress_bridge;

#[cfg(test)]
#[path = "progress_bridge_tests.rs"]
mod tests;
