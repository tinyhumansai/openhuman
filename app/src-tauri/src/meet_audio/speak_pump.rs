//! Speak path: poll synthesized PCM out of core and feed it into the
//! Meet page's audio bridge over CDP.
//!
//! Design lives in [`super::inject`]: the bridge is installed once at
//! session start by `install_audio_bridge`, which returns the open CDP
//! connection + session id. The pump owns those for the lifetime of
//! the call so each tick is a single `Runtime.evaluate` round-trip
//! rather than fresh attach + detach.

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
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
/// from this point on.
pub fn start(request_id: String, cdp: CdpConn, session_id: String) -> SpeakPump {
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
    let request_id_for_task = request_id.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = interval(POLL_INTERVAL);
        // Burn the first tick (`interval` fires immediately) so we
        // don't poll before the listen path has had a chance to push.
        tick.tick().await;
        let mut cdp = cdp;
        let mut feed_errors: u32 = 0;
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    log::info!(
                        "[meet-audio] speak pump shutdown request_id={request_id_for_task}"
                    );
                    break;
                }
                _ = tick.tick() => {
                    match poll_and_feed(&request_id_for_task, &mut cdp, &session_id).await {
                        Ok(_) => feed_errors = 0,
                        Err(err) => {
                            feed_errors += 1;
                            log::debug!(
                                "[meet-audio] speak pump tick err request_id={request_id_for_task} consec_errors={feed_errors} err={err}"
                            );
                            if feed_errors >= MAX_CONSECUTIVE_FEED_ERRORS {
                                log::warn!(
                                    "[meet-audio] speak pump giving up after {feed_errors} consecutive errors request_id={request_id_for_task}"
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }
    });

    SpeakPump {
        request_id,
        _shutdown_tx: Some(shutdown_tx),
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

async fn poll_and_feed(
    request_id: &str,
    cdp: &mut CdpConn,
    session_id: &str,
) -> Result<(), String> {
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
    } else if utterance_done {
        log::info!("[meet-audio] speak pump utterance complete request_id={request_id}");
    }
    Ok(())
}
