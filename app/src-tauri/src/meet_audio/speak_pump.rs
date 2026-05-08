//! Speak path: drain synthesized PCM the brain enqueued and write it
//! into Chromium's fake-audio source for the embedded Meet webview.
//!
//! ## Status
//!
//! This file is the scaffolding for the speak path. The actual sink —
//! a Unix domain socket that Chromium's patched `FileSource` reads as
//! if it were a WAV file — lands in a follow-up slice along with the
//! C++ patch in the vendored CEF subtree (see project plan: "Pipe://
//! fake-audio source patch in vendored Chromium"). Until that ships
//! the pump simply polls `meet_agent_poll_speech` and discards the
//! audio while logging counters, which exercises the full RPC path
//! end-to-end so the brain knows somebody is draining its queue.

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use tokio::sync::oneshot;
use tokio::time::interval;

/// Polling cadence. Matches the listen path's flush boundary so the
/// loop stays in lockstep — every ~100 ms we push captured audio in
/// and pull synthesized audio out.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// RAII handle. Drop to stop polling. The pump task exits cleanly on
/// the next tick after the shutdown channel resolves.
pub struct SpeakPump {
    pub request_id: String,
    /// Held so Drop signals the pump task to exit.
    _shutdown_tx: Option<oneshot::Sender<()>>,
}

impl Drop for SpeakPump {
    fn drop(&mut self) {
        // Take + drop the sender so the receiver wakes with `Err(_)`.
        let _ = self._shutdown_tx.take();
    }
}

pub fn start(request_id: String) -> SpeakPump {
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
    let request_id_for_task = request_id.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = interval(POLL_INTERVAL);
        // Burn the first tick (`interval` fires immediately) so we
        // don't poll before the brain has had a chance to enqueue
        // anything from the corresponding push.
        tick.tick().await;
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    log::info!(
                        "[meet-audio] speak pump shutdown request_id={request_id_for_task}"
                    );
                    break;
                }
                _ = tick.tick() => {
                    if let Err(err) = poll_once(&request_id_for_task).await {
                        log::debug!(
                            "[meet-audio] poll_speech err request_id={request_id_for_task} err={err}"
                        );
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

/// One iteration: ask core for any pending PCM, decode it, "write" it
/// to the (future) Chromium pipe sink. Today the bytes are counted and
/// dropped — that placeholder lets us validate the timing/throughput
/// before the sink lands.
async fn poll_once(request_id: &str) -> Result<(), String> {
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
        match B64.decode(pcm_b64.as_bytes()) {
            Ok(bytes) => {
                // TODO(meet-audio-speak-pipe): write `bytes` into the
                // Chromium fake-audio UDS sink keyed by request_id.
                // For now, just account it so the logs reflect the
                // brain's progress.
                log::debug!(
                    "[meet-audio] speak pump drained request_id={request_id} bytes={} done={utterance_done}",
                    bytes.len()
                );
            }
            Err(err) => {
                log::warn!(
                    "[meet-audio] speak pump base64 decode failed request_id={request_id} err={err}"
                );
            }
        }
    } else if utterance_done {
        log::info!(
            "[meet-audio] speak pump utterance complete request_id={request_id}"
        );
    }
    Ok(())
}
