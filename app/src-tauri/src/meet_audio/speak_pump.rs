//! Speak path: poll synthesized PCM out of core and feed it into the
//! Meet page's audio bridge over CDP.
//!
//! Design lives in [`super::inject`]: the bridge is installed once at
//! session start by `install_audio_bridge`, which returns the open CDP
//! connection + session id. The pump owns those for the lifetime of
//! the call so each tick is a single `Runtime.evaluate` round-trip
//! rather than fresh attach + detach.

use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::oneshot;
use tokio::time::interval;

use crate::cdp::CdpConn;

use super::inject;

/// Polling cadence. Same as the listen path's flush boundary so the
/// loop stays in lockstep — every ~100 ms we push captured audio in
/// (listen) and pull synthesized audio out (speak).
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Cap on consecutive feed failures before we give up and stop
/// pumping. Hitting this usually means the page navigated away
/// (Meet's "you've been removed" / network drop) — the meet-call
/// window-destroyed handler will tear the rest of the session down
/// either way.
const MAX_CONSECUTIVE_FEED_ERRORS: u32 = 30;

/// How long the speaking-state event keeps reporting `speaking=true`
/// after the last non-empty PCM tick. Brain enqueues outbound in
/// chunks of ~50–200 ms and there's a gap of one or two pump ticks
/// (100 ms each) between chunks while the next batch is being
/// synthesised. Without a hangover, the mascot's mouth would flicker
/// shut every gap. 400 ms covers the typical inter-chunk silence
/// without bridging across legitimate end-of-utterance pauses.
const SPEAKING_HANGOVER: Duration = Duration::from_millis(400);

/// Tauri event channel for "the bot is/isn't speaking right now".
/// Consumed by `MascotFrameProducer` (frontend) to flip the mascot
/// SVG between idle and a mouth-open / talking pose so the Meet
/// participant sees a visual cue that matches the audio they hear.
const SPEAKING_STATE_EVENT: &str = "meet-video:speaking-state";

/// RAII handle. Drop to stop the pump task. The shutdown channel
/// causes the spawned loop to exit on the next select tick.
pub struct SpeakPump {
    pub request_id: String,
    _shutdown_tx: Option<oneshot::Sender<()>>,
}

impl Drop for SpeakPump {
    fn drop(&mut self) {
        let _ = self._shutdown_tx.take();
    }
}

/// Spawn the speak pump for a session that already has the audio
/// bridge installed. `cdp` and `session_id` come from
/// [`inject::install_audio_bridge`] and are owned by the pump task
/// from this point on. `app` is held so the pump can fire
/// `meet-video:speaking-state` events when the bot starts / stops
/// producing PCM (drives the in-Meet mascot's mouth animation).
pub fn start<R: Runtime>(
    app: AppHandle<R>,
    request_id: String,
    cdp: CdpConn,
    session_id: String,
) -> SpeakPump {
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
    let request_id_for_task = request_id.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = interval(POLL_INTERVAL);
        // Burn the first tick (`interval` fires immediately) so we
        // don't poll before the listen path has had a chance to push.
        tick.tick().await;
        let mut cdp = cdp;
        let mut feed_errors: u32 = 0;
        // Edge-detect state for the speaking-state event. We emit on
        // every flip and never on every tick — the frontend renderer
        // would otherwise see a flood of redundant state updates and
        // burn worker time on no-op rerenders.
        let mut speaking_state = SpeakingTracker::new();
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    log::info!(
                        "[meet-audio] speak pump shutdown request_id={request_id_for_task}"
                    );
                    // Make sure the mascot stops talking when the
                    // session ends — without this the last "speaking"
                    // edge would leave the mouth open for the next
                    // call's first frame.
                    speaking_state.force_off(&app, &request_id_for_task);
                    break;
                }
                _ = tick.tick() => {
                    let had_pcm = match poll_and_feed(&request_id_for_task, &mut cdp, &session_id).await {
                        Ok(had) => {
                            feed_errors = 0;
                            had
                        }
                        Err(err) => {
                            feed_errors += 1;
                            log::debug!(
                                "[meet-audio] speak pump tick err request_id={request_id_for_task} consec_errors={feed_errors} err={err}"
                            );
                            if feed_errors >= MAX_CONSECUTIVE_FEED_ERRORS {
                                log::warn!(
                                    "[meet-audio] speak pump giving up after {feed_errors} consecutive errors request_id={request_id_for_task}"
                                );
                                speaking_state.force_off(&app, &request_id_for_task);
                                break;
                            }
                            // A failed tick is *not* evidence the bot
                            // stopped speaking — leave the hangover to
                            // expire naturally so transient CDP errors
                            // don't flicker the mascot's mouth shut.
                            false
                        }
                    };
                    speaking_state.tick(had_pcm, &app, &request_id_for_task);
                }
            }
        }
    });

    SpeakPump {
        request_id,
        _shutdown_tx: Some(shutdown_tx),
    }
}

/// Edge-detector + hangover for the speaking-state event stream.
/// State machine has two reachable values (`speaking` / `idle`) and
/// flips between them only when the underlying signal sustains long
/// enough to clear the hangover, so the frontend never sees a flap
/// during the natural gap between two PCM chunks.
struct SpeakingTracker {
    /// Currently-reported state. Defaults to `false` so the mascot
    /// boots into the idle pose; the first `speaking=true` tick is a
    /// real edge.
    reported: bool,
    /// Wall-clock the hangover expires. Set to `now + SPEAKING_HANGOVER`
    /// every tick that carries PCM; the state flips back to `false`
    /// only once `now > hangover_until` AND a tick with no PCM lands.
    hangover_until: Option<Instant>,
}

impl SpeakingTracker {
    fn new() -> Self {
        Self {
            reported: false,
            hangover_until: None,
        }
    }

    /// Drive the state machine from a single pump tick. `had_pcm`
    /// is whether `poll_and_feed` saw a non-empty `pcm_base64` for
    /// this tick. Emits the Tauri event only when the reported
    /// state actually flips.
    fn tick<R: Runtime>(&mut self, had_pcm: bool, app: &AppHandle<R>, request_id: &str) {
        if had_pcm {
            // Extend the hangover. If we were idle, flip up to
            // speaking — the user hears audio starting now.
            self.hangover_until = Some(Instant::now() + SPEAKING_HANGOVER);
            self.set_reported(true, app, request_id);
            return;
        }
        // No PCM this tick. If the hangover hasn't expired, stay in
        // whatever state we were already in (typically `speaking=true`
        // during the gap between two consecutive chunks).
        if let Some(until) = self.hangover_until {
            if Instant::now() < until {
                return;
            }
            // Hangover elapsed; clear so we don't re-evaluate on
            // every future idle tick.
            self.hangover_until = None;
        }
        // Hangover expired or never armed → bot is genuinely idle.
        self.set_reported(false, app, request_id);
    }

    /// Force the reported state to `false` and emit an event if that's
    /// a flip. Used on shutdown / fatal error paths so the mascot
    /// can't get stuck mid-talk.
    fn force_off<R: Runtime>(&mut self, app: &AppHandle<R>, request_id: &str) {
        self.hangover_until = None;
        self.set_reported(false, app, request_id);
    }

    fn set_reported<R: Runtime>(&mut self, next: bool, app: &AppHandle<R>, request_id: &str) {
        if self.reported == next {
            return;
        }
        self.reported = next;
        let payload = serde_json::json!({
            "requestId": request_id,
            "speaking": next,
        });
        if let Err(err) = app.emit(SPEAKING_STATE_EVENT, payload) {
            // Best-effort: a missing renderer (closed window mid-tick)
            // is the common case and not worth raising the log level.
            log::debug!(
                "[meet-audio] speaking-state emit failed request_id={request_id} speaking={next} err={err}"
            );
        } else {
            log::debug!("[meet-audio] speaking-state -> {next} request_id={request_id}");
        }
    }
}

/// No-op pump used when bridge install failed at session start. Keeps
/// the rest of the session lifecycle uniform — `MeetAudioSession` can
/// still hold a `SpeakPump` regardless of speak-path readiness.
pub fn start_disabled(request_id: String) -> SpeakPump {
    SpeakPump {
        request_id,
        _shutdown_tx: None,
    }
}

/// Run a single pump tick. Returns `true` when the tick actually
/// carried synthesized PCM (used by the caller to drive the
/// speaking-state edge detector).
async fn poll_and_feed(
    request_id: &str,
    cdp: &mut CdpConn,
    session_id: &str,
) -> Result<bool, String> {
    let v = super::rpc_call(
        "openhuman.meet_agent_poll_speech",
        serde_json::json!({ "request_id": request_id }),
    )
    .await?;
    let pcm_b64 = v
        .get("pcm_base64")
        .and_then(|x| x.as_str())
        .unwrap_or_default();
    let utterance_done = v
        .get("utterance_done")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let flush_pending = v
        .get("flush_pending")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);

    // Barge-in: brain set flush_pending when it cancelled the previous
    // outbound. Stop in-flight playback inside the JS bridge BEFORE we
    // feed the next chunk so the user hears the new reply instead of
    // the tail of the old one. Best-effort — if the page is gone the
    // flush errors and we drop through to the feed, which will fail
    // the same way and trigger the same recovery path.
    if flush_pending {
        match inject::flush_audio_bridge(cdp, session_id).await {
            Ok(stopped) => log::info!(
                "[meet-audio] barge-in flush request_id={request_id} sources_stopped={stopped}"
            ),
            Err(e) => {
                log::warn!("[meet-audio] barge-in flush failed request_id={request_id} err={e}")
            }
        }
    }

    if !pcm_b64.is_empty() {
        // Validate decode locally before pushing — saves a round-trip
        // when the brain enqueues a malformed batch.
        let bytes = B64
            .decode(pcm_b64.as_bytes())
            .map_err(|e| format!("base64: {e}"))?;
        log::debug!(
            "[meet-audio] speak pump feeding request_id={request_id} bytes={} done={utterance_done}",
            bytes.len()
        );
        inject::feed_pcm_chunk(cdp, session_id, pcm_b64).await?;
        return Ok(true);
    }
    if utterance_done {
        log::info!("[meet-audio] speak pump utterance complete request_id={request_id}");
    }
    Ok(false)
}
