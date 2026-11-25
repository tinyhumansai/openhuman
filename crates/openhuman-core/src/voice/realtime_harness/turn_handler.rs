//! Handling one relayed voice turn end to end: dedup, stream deltas back to
//! the relay socket, race the turn against the spoken-ack deadline, and hand
//! a slow turn off to chat delivery in the background.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use log::{info, warn};
use serde_json::{json, Value};
use tokio::sync::Semaphore;

use tinyagents_harness::progress::AgentProgress;

use super::agent::run_voice_turn;
use super::chat_delivery::{
    deliver_voice_failure_to_chat, deliver_voice_result_to_chat, emit_error, emit_event,
};
use super::prompt::{
    extract_prompt, is_answerable_prompt, is_content_free, readback_payload, should_arm_speak_back,
    spoken_delta,
};

/// How long a voice turn may run before we stop making the caller wait and hand
/// the result off to chat. The cloud voice session cancels a turn with no spoken
/// token in ~11-12s, and a slow tool action (email/calendar summary can be
/// 20-30s of Composio round-trips) will never fit that window. Once this elapses
/// we close the voice turn cleanly — the caller has heard the relay's spoken
/// filler and is told the answer is still coming (see VOICE_HANDOFF_LINES) — and
/// let the orchestrator finish in the background,
/// delivering its answer into the user's in-app chat and, while the call is
/// still up, reading it aloud. Sits under the provider's cancel deadline so
/// `done` always beats the cut.
const VOICE_ACK_DEADLINE_SECS: u64 = 8;

/// Spoken when the ack deadline closes a turn that never produced text of its
/// own, so the caller is told the answer is still coming instead of being left on
/// a trail of filler.
///
/// The model is deliberately not asked to speak first; the relay owns the
/// filler because the model could not be relied on to do so anyway: building
/// the per-turn orchestrator (config
/// load, tool registry, integration catalogue) takes seconds before the first
/// model token is even requested, and the first round is usually a tool call
/// carrying no text. Measured against staging, the first streamable token landed
/// ~10.6s into a turn whose whole budget is 8s.
///
/// Neutral rather than a promise. "I'll have that for you in a moment" states an
/// outcome the turn cannot guarantee the shape of: the answer may follow a second
/// later, or thirty, and on a turn the caller expected to be trivial ("no, not
/// now") a promise of future delivery reads as the assistant having
/// misunderstood. A short acknowledgement says the same thing about the only fact
/// known at this point — work is still going — without committing to when.
///
/// Rotated per turn so a caller who hits the deadline twice in a call does not
/// hear the same words back. Each ends in a full stop deliberately: the provider
/// synthesises on sentence boundaries, so an unterminated line is buffered rather
/// than spoken (see `VOICE_FILLERS` in the backend relay).
///
/// The answer itself still arrives on both delivery paths — posted to chat and,
/// while the call is still up, read aloud.
pub(super) const VOICE_HANDOFF_LINES: [&str; 4] = [
    "Still on it. ",
    "Still going. ",
    "Still working on it. ",
    "Bear with me. ",
];
static VOICE_HANDOFF_CURSOR: AtomicUsize = AtomicUsize::new(0);

/// Next handoff line, rotating. Pure apart from the cursor; unit-tested.
pub(super) fn next_handoff_line() -> &'static str {
    VOICE_HANDOFF_LINES
        [VOICE_HANDOFF_CURSOR.fetch_add(1, Ordering::Relaxed) % VOICE_HANDOFF_LINES.len()]
}

/// Sent for a turn we have nothing to say to.
///
/// A turn that ends with no spoken content at all is not a valid answer to the
/// cloud session: it ends the whole call with `custom_llm_error: LLM Cascade
/// Error: Brain returned no response` (confirmed live — three recognition
/// artefacts answered with empty turns killed a working call). So even "nothing
/// to say" has to say something, and the least intrusive something is an ellipsis
/// pause, which the provider voices as a beat of silence rather than words.
const VOICE_SILENT_REPLY: &str = "… ";

/// Cap on concurrent local-agent turns driven by the relay. Each turn loads
/// config, builds a full orchestrator, and runs for up to `TURN_TIMEOUT_SECS`,
/// so an unbounded burst (or retry storm) would spawn unbounded heavy agent
/// sessions. Excess turns queue on the permit rather than piling up.
const MAX_CONCURRENT_VOICE_TURNS: usize = 3;

static VOICE_TURN_LIMITER: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_VOICE_TURNS)));

/// Correlation ids currently being processed, so a relay retry that re-delivers
/// the same `voice:harness` event is deduplicated instead of running the turn
/// (and charging/emitting) twice.
static IN_FLIGHT_CORRELATIONS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// RAII claim on a correlation id: inserts on `claim`, removes on drop. `None`
/// from `claim` means a turn for that id is already in flight.
struct InFlightGuard(String);

impl InFlightGuard {
    fn claim(correlation_id: &str) -> Option<Self> {
        let mut set = IN_FLIGHT_CORRELATIONS
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if set.insert(correlation_id.to_string()) {
            Some(Self(correlation_id.to_string()))
        } else {
            None
        }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        let mut set = IN_FLIGHT_CORRELATIONS
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        set.remove(&self.0);
    }
}

/// Handle one relayed voice turn end to end. The reply streams token-by-token
/// back up the socket as it is produced. A turn that finishes inside the voice
/// window is spoken in full; a slow tool action (email/calendar summary — tens of
/// seconds of Composio round-trips) is acknowledged aloud, then finishes in the
/// background and delivers its answer into the user's in-app chat. Never panics —
/// every failure path emits `voice:harness:error` (or a clean `done`) so the
/// backend relay ends the turn cleanly.
pub async fn handle_voice_harness_turn(
    ctx: crate::core::runtime::DispatchCtx,
    correlation_id: String,
    messages: Vec<Value>,
) {
    let prompt = extract_prompt(&messages);
    if prompt.trim().is_empty() {
        emit_error(&correlation_id, "no user message in the relayed turn").await;
        return;
    }

    // A read-back turn hands us the very text it wants spoken. Running an
    // orchestrator turn to echo it costs a full agent build, memory recall and a
    // model round-trip — far more than the turn budget allows — so the caller
    // hears filler instead of the answer they are waiting for. Current backends
    // answer these at the relay and never wake us; an older one still relays them,
    // so answer from the prompt here too.
    if let Some(payload) = readback_payload(&prompt) {
        info!(
            "[voice-harness] read-back answered from the prompt correlation={correlation_id} chars={}",
            payload.chars().count()
        );
        // Always speak something, even for an empty payload — see VOICE_SILENT_REPLY.
        emit_event(
            "voice:harness:delta",
            json!({ "correlationId": correlation_id,
                    "text": if payload.is_empty() { VOICE_SILENT_REPLY } else { payload } }),
        )
        .await;
        emit_event(
            "voice:harness:done",
            json!({ "correlationId": correlation_id }),
        )
        .await;
        return;
    }

    // Speech recognition turns a pause during a filler-heavy turn into a "..."
    // user message, and the provider relays it as a real turn. Building an
    // orchestrator for it burns a concurrency slot and a model round-trip to
    // answer nothing — and its empty reply surfaced in chat as a failure notice
    // the user never asked for.
    if is_content_free(&prompt) {
        info!("[voice-harness] ignoring content-free turn correlation={correlation_id} prompt={prompt:?}");
        emit_event(
            "voice:harness:delta",
            json!({ "correlationId": correlation_id, "text": VOICE_SILENT_REPLY }),
        )
        .await;
        emit_event(
            "voice:harness:done",
            json!({ "correlationId": correlation_id }),
        )
        .await;
        return;
    }

    // Deduplicate a re-delivered turn (relay retry) before doing any work. Owned
    // so it can move into the background continuation and track the real turn.
    let Some(in_flight) = InFlightGuard::claim(&correlation_id) else {
        warn!("[voice-harness] duplicate turn correlation={correlation_id} already in flight — dropping");
        return;
    };

    // Stream the reply token-by-token: the orchestrator emits `TextDelta` as it
    // generates, and `forward_reply_deltas` relays each as a `voice:harness:delta`
    // so the reply leaves the desktop as it is produced.
    let streamed = Arc::new(AtomicBool::new(false));
    let (progress_tx, progress_rx) = tokio::sync::mpsc::channel::<AgentProgress>(256);
    let forwarder = tokio::spawn(forward_reply_deltas(
        progress_rx,
        correlation_id.clone(),
        streamed.clone(),
    ));

    // Run the turn on a detached task so it can outlive the spoken ack. A slow
    // delegation keeps working after we close the voice turn and delivers its
    // result into the user's chat. The task owns the agent, the concurrency
    // permit, and the in-flight guard so that bookkeeping tracks the real turn.
    let (result_tx, result_rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    // Captured before the prompt moves into the task, so the foreground can still
    // tell whether the user is waiting on an answer if that task dies.
    let answerable = is_answerable_prompt(&prompt);
    let turn_cid = correlation_id.clone();
    let turn_messages = messages;
    let turn_prompt = prompt;
    tokio::spawn(async move {
        let _in_flight = in_flight;
        // Bound concurrent heavy agent turns; excess turns queue HERE, inside the
        // detached task, never in front of the ack deadline below. Acquiring in the
        // foreground meant a turn queued behind slower ones started no clock and
        // emitted no `done` at all, and the provider ended the whole session over
        // it. Held for the REAL turn lifetime (including any background tail), so
        // it still caps a retry storm.
        let _permit = match VOICE_TURN_LIMITER.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => {
                warn!("[voice-harness] turn limiter unavailable correlation={turn_cid}");
                if is_answerable_prompt(&turn_prompt) {
                    deliver_voice_failure_to_chat(&turn_cid);
                }
                return;
            }
        };
        let outcome =
            run_voice_turn(&ctx, &turn_cid, &turn_messages, &turn_prompt, progress_tx).await;
        // Hand the result to the foreground. If it already deferred (dropped the
        // receiver at the ack deadline), the send fails and we deliver the reply
        // into the user's chat instead.
        if let Err(unsent) = result_tx.send(outcome) {
            match unsent {
                Ok(reply) => {
                    // A read-back turn only re-reads an answer already delivered to
                    // chat; delivering it again would duplicate the chat message, and
                    // re-arming speak-back would loop. So deliver + arm speak-back ONLY
                    // for a genuine deferred answer, and skip the whole delivery for a
                    // read-back echo turn.
                    if should_arm_speak_back(&turn_prompt) {
                        deliver_voice_result_to_chat(&turn_cid, reply, true);
                    } else {
                        info!("[voice-harness] deferred read-back turn carries no new content; skipping chat delivery correlation={turn_cid}");
                    }
                }
                Err(err) => {
                    // The spoken turn already closed with `done` and the caller was
                    // told the answer was still coming, so a silent failure would leave
                    // the user waiting for a message that never arrives. Post a brief
                    // failure notice to the same thread — but only for a turn whose
                    // answer the user is actually waiting on. A read-back or a
                    // recognition artefact has nothing to deliver, and a notice for
                    // one reads as the assistant failing a request nobody made.
                    warn!("[voice-harness] deferred turn failed correlation={turn_cid}: {err}");
                    if is_answerable_prompt(&turn_prompt) {
                        deliver_voice_failure_to_chat(&turn_cid);
                    }
                }
            }
        }
    });

    // Race the turn against the spoken-ack deadline.
    match tokio::time::timeout(Duration::from_secs(VOICE_ACK_DEADLINE_SECS), result_rx).await {
        Ok(Ok(outcome)) => {
            // Finished inside the voice window — deltas already streamed. Join the
            // forwarder so every delta is out before `done`, and learn whether
            // anything streamed (fallback for a non-streaming reply).
            let streamed_any = forwarder.await.unwrap_or(false);
            match outcome {
                Ok(reply) => {
                    if !streamed_any {
                        // A turn that produced no text still has to say something, or
                        // the provider ends the call (see VOICE_SILENT_REPLY).
                        let spoken = reply.trim();
                        let spoken = if spoken.is_empty() {
                            VOICE_SILENT_REPLY
                        } else {
                            spoken
                        };
                        emit_event(
                            "voice:harness:delta",
                            json!({ "correlationId": correlation_id, "text": spoken }),
                        )
                        .await;
                    }
                    emit_event(
                        "voice:harness:done",
                        json!({ "correlationId": correlation_id }),
                    )
                    .await;
                }
                Err(err) => {
                    warn!("[voice-harness] turn failed correlation={correlation_id}: {err}");
                    emit_error(&correlation_id, &err).await;
                }
            }
        }
        Ok(Err(_recv)) => {
            // Sender dropped without a value (task aborted or panicked). A silent
            // turn with no spoken text and no chat delivery is hard to trace, so
            // log with the correlation id before ending cleanly.
            warn!(
                "[voice-harness] turn task ended without a result (panicked or aborted) correlation={correlation_id}"
            );
            // Unlike the ack-deadline path, nothing else will deliver here: the task
            // died before reaching its own chat-delivery branch. The turn may already
            // have promised a follow-up in chat, so post the notice from this side —
            // for a turn the user is actually waiting on (see is_answerable_prompt).
            if answerable {
                deliver_voice_failure_to_chat(&correlation_id);
            }
            // Closing on nothing at all would end the whole call (see
            // VOICE_SILENT_REPLY), so a lost turn still ends with a spoken beat.
            if !streamed.load(Ordering::SeqCst) {
                emit_event(
                    "voice:harness:delta",
                    json!({ "correlationId": correlation_id, "text": VOICE_SILENT_REPLY }),
                )
                .await;
            }
            emit_event(
                "voice:harness:done",
                json!({ "correlationId": correlation_id }),
            )
            .await;
        }
        Err(_deadline) => {
            // Still running (a slow tool action). Close the voice turn cleanly; the
            // detached task keeps going and delivers its answer into the user's
            // chat. `timeout` consumed `result_rx`, so the task's send fails and
            // takes the chat-delivery path. The forwarder is left running to keep
            // draining progress — any late deltas reach a settled relay turn and
            // are dropped harmlessly.
            info!("[voice-harness] ack deadline reached, handing off to chat correlation={correlation_id}");
            // If the turn never said anything of its own, the caller has heard only
            // the relay's filler. Ending there sounds like the assistant lost the
            // thread, so say the answer is still coming — it is, on both delivery
            // paths (chat, and read aloud while the call is up).
            if !streamed.load(Ordering::SeqCst) {
                emit_event(
                    "voice:harness:delta",
                    json!({ "correlationId": correlation_id, "text": next_handoff_line() }),
                )
                .await;
            }
            emit_event(
                "voice:harness:done",
                json!({ "correlationId": correlation_id }),
            )
            .await;
        }
    }
}

/// Forward the orchestrator's streamed assistant text to the relay socket, one
/// `voice:harness:delta` per top-level `AgentProgress::TextDelta`, until the
/// turn's progress channel closes (the agent drops its sender when the turn
/// ends). Returns whether any non-empty delta was streamed, so the caller can
/// fall back to emitting the whole reply for a turn that produced text off the
/// streaming path. Only top-level assistant text is voiced — sub-agent narration,
/// thinking, tool-call args, and lifecycle events are deliberately not spoken.
async fn forward_reply_deltas(
    mut progress_rx: tokio::sync::mpsc::Receiver<AgentProgress>,
    correlation_id: String,
    streamed: Arc<AtomicBool>,
) -> bool {
    let mut streamed_any = false;
    while let Some(progress) = progress_rx.recv().await {
        let Some(text) = spoken_delta(&progress) else {
            continue;
        };
        // Skip only truly empty deltas — whitespace carries word boundaries and
        // must be forwarded so the concatenated speech isn't run together.
        if text.is_empty() {
            continue;
        }
        streamed_any = true;
        // Published for the ack + handoff decisions, which need to know whether the
        // orchestrator is talking *while* the turn is still open.
        streamed.store(true, Ordering::SeqCst);
        emit_event(
            "voice:harness:delta",
            json!({ "correlationId": correlation_id, "text": text }),
        )
        .await;
    }
    streamed_any
}
