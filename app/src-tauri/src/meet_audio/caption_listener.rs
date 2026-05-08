//! Listen path v2: drains Meet's built-in captions region via the
//! `captions_bridge.js` we install at session start, and forwards each
//! new line to core's `meet_agent_push_caption` RPC.
//!
//! Replaces the old [`super::listen_capture`] (CEF audio handler →
//! Whisper STT) which proved unreliable: CEF's `cef_audio_handler_t`
//! is queried lazily on first audio output, so a solo agent in a
//! lobby never engaged the pipeline. Captions handle that case for
//! free — Meet's STT is already running, speaker-attributed, and
//! pre-segmented.
//!
//! Lifecycle is owned by [`super::SpeakPump`]'s sibling: dropping the
//! returned [`CaptionListener`] shuts the polling task down.

use std::time::Duration;

use tokio::sync::oneshot;
use tokio::time::interval;

use crate::cdp::CdpConn;

use super::inject;

/// Polling cadence for `__openhumanDrainCaptions`. Captions arrive at
/// roughly word-by-word frequency; 500 ms is the sweet spot between
/// "responsive enough that wake-word detection feels live" and "not
/// hammering the CDP socket".
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Cap on consecutive drain failures before the listener gives up.
/// Same shape as the speak pump — usually means the page navigated
/// away (call ended) or the renderer crashed.
const MAX_CONSECUTIVE_ERRORS: u32 = 30;

/// RAII handle. Drop to stop the listener task.
pub struct CaptionListener {
    pub request_id: String,
    pub(crate) _shutdown_tx: Option<oneshot::Sender<()>>,
}

impl Drop for CaptionListener {
    fn drop(&mut self) {
        let _ = self._shutdown_tx.take();
    }
}

/// Spawn the caption polling loop for a session whose audio bridge
/// has already installed both `audio_bridge.js` and
/// `captions_bridge.js`. Owns its own clone of the CDP connection so
/// drains run concurrently with speak-pump feeds.
pub fn start(request_id: String, cdp: CdpConn, session_id: String) -> CaptionListener {
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
    let request_id_for_task = request_id.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = interval(POLL_INTERVAL);
        // Burn the first tick so the very first drain has something
        // to drain (the page-side observer needs ~250 ms to attach).
        tick.tick().await;
        let mut cdp = cdp;
        let mut errors: u32 = 0;
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    log::info!(
                        "[meet-audio] caption listener shutdown request_id={request_id_for_task}"
                    );
                    break;
                }
                _ = tick.tick() => {
                    match drain_and_forward(&request_id_for_task, &mut cdp, &session_id).await {
                        Ok(_) => errors = 0,
                        Err(err) => {
                            errors += 1;
                            log::debug!(
                                "[meet-audio] caption tick err request_id={request_id_for_task} consec_errors={errors} err={err}"
                            );
                            if errors >= MAX_CONSECUTIVE_ERRORS {
                                log::warn!(
                                    "[meet-audio] caption listener giving up after {errors} consecutive errors request_id={request_id_for_task}"
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }
    });

    CaptionListener {
        request_id,
        _shutdown_tx: Some(shutdown_tx),
    }
}

async fn drain_and_forward(
    request_id: &str,
    cdp: &mut CdpConn,
    session_id: &str,
) -> Result<(), String> {
    let captions = inject::drain_captions(cdp, session_id).await?;
    if captions.is_empty() {
        return Ok(());
    }
    log::info!(
        "[meet-audio] captions drained count={} request_id={request_id}",
        captions.len()
    );
    for (speaker, text, ts_ms) in captions {
        // Propagate the failure so MAX_CONSECUTIVE_ERRORS can trip if
        // core's session/RPC path is broken — without this the
        // listener would silently drop captions forever while the
        // page kept producing them.
        super::rpc_call(
            "openhuman.meet_agent_push_caption",
            serde_json::json!({
                "request_id": request_id,
                "speaker": speaker,
                "text": text,
                "ts_ms": ts_ms,
            }),
        )
        .await
        .map_err(|err| format!("push_caption (request_id={request_id}): {err}"))?;
    }
    Ok(())
}
