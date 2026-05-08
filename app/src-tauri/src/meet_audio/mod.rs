//! Shell-side audio plumbing for the live meet-agent loop.
//!
//! ## Pieces
//!
//! - [`listen_capture`] — taps the embedded Meet webview's audio output
//!   via the per-browser `CefAudioHandler` exposed by our vendored
//!   `tauri-runtime-cef::audio` extension, downsamples to 16 kHz mono
//!   PCM16LE, batches into ~100 ms chunks, and posts them to core via
//!   `openhuman.meet_agent_push_listen_pcm`. Zero OS-level audio
//!   permission needed: we read frames straight out of the renderer.
//!
//! - [`speak_pump`] — drains synthesized PCM the brain enqueued (via
//!   `openhuman.meet_agent_poll_speech`) and writes it into the
//!   Chromium `pipe://openhuman/<request_id>` fake-audio source we
//!   patch in the vendored CEF subtree. PR1 ships the pump scaffolding;
//!   the Chromium-side patch lands in a follow-up slice.
//!
//! ## Lifecycle
//!
//! [`start`] is invoked once the meet-call window has been built (in
//! `meet_call::meet_call_open_window`). It opens the core session,
//! registers the audio handler keyed by the call's URL, and spawns the
//! poll-speech loop. [`stop`] runs from the window-destroyed handler:
//! it drops the audio handler registration (which silences capture
//! immediately), stops the speak pump, and tells core to close the
//! session and report counters.

pub mod caption_listener;
pub mod inject;
pub mod listen_capture;
pub mod speak_pump;

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};

/// Process-wide registry of active meet-agent sessions, keyed by
/// `request_id`. Mirrors the shape of `meet_call::MeetCallState` so
/// the two registries stay symmetric.
#[derive(Default)]
pub struct MeetAudioState {
    inner: Mutex<HashMap<String, MeetAudioSession>>,
}

impl MeetAudioState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Held while a session is live. Dropping it runs the listen + speak
/// teardown synchronously — no async drop needed because the caption
/// listener and speak pump both shut down on signal/drop.
///
/// The legacy CEF-audio `listen_capture::ListenSession` is kept as an
/// optional field so the pre-register flow still has somewhere to
/// hand the registration off if a future build re-enables it. In the
/// caption-driven path it stays `None`.
pub struct MeetAudioSession {
    pub request_id: String,
    _captions: caption_listener::CaptionListener,
    _legacy_listen: Option<listen_capture::ListenSession>,
    _speak: speak_pump::SpeakPump,
}

#[derive(Debug, Clone, Serialize)]
pub struct StopSummary {
    pub request_id: String,
    pub listened_seconds: f32,
    pub spoken_seconds: f32,
    pub turn_count: u32,
}

/// Open a meet-agent audio session.
///
/// Listen path goes via the captions bridge (`captions_bridge.js`) +
/// [`caption_listener`]. Speak path goes via the audio bridge
/// (`audio_bridge.js`) + [`speak_pump`]. Both are installed by
/// [`inject::install_audio_bridge`].
///
/// `meet_url` must be the *exact* URL the CEF window was built with —
/// the inject path uses it as the CDP target prefix so two concurrent
/// calls each attach to their own browser.
pub async fn start<R: Runtime>(
    app: AppHandle<R>,
    request_id: String,
    meet_url: String,
) -> Result<(), String> {
    log::info!(
        "[meet-audio] start request_id={request_id} url_prefix={}",
        truncate_for_log(&meet_url, 64)
    );

    if let Some(state) = app.try_state::<MeetAudioState>() {
        let mut guard = state.inner.lock().unwrap();
        if guard.contains_key(&request_id) {
            // Idempotent restart: drop the previous session before
            // overwriting so its registration is released.
            guard.remove(&request_id);
            log::info!("[meet-audio] replaced existing session request_id={request_id}");
        }
    }

    // Tell core to open its session first so the very first PCM push
    // doesn't race the start RPC.
    rpc_call(
        "openhuman.meet_agent_start_session",
        serde_json::json!({
            "request_id": request_id,
            "sample_rate_hz": 16_000,
        }),
    )
    .await?;

    // Install the page-side audio + captions bridges in one go. The
    // returned CDP session is shared by the speak pump and caption
    // listener — we open a second session for the listener so the
    // two run concurrently without serialising on a single CDP
    // mailbox.
    let (speak, captions) = match inject::install_audio_bridge(&meet_url).await {
        Ok((cdp, session)) => {
            // Spawn the caption listener on its own CDP attach so a
            // long Runtime.evaluate from one side never starves the
            // other. The second attach reuses the same CDP target.
            let captions = match crate::cdp::connect_and_attach_matching(|t| {
                t.url.starts_with(&meet_url)
            })
            .await
            {
                Ok((cdp_for_captions, session_for_captions)) => caption_listener::start(
                    request_id.clone(),
                    cdp_for_captions,
                    session_for_captions,
                ),
                Err(err) => {
                    log::warn!(
                        "[meet-audio] caption listener cdp attach failed request_id={request_id} err={err}"
                    );
                    caption_listener_disabled(request_id.clone())
                }
            };
            let speak = speak_pump::start(request_id.clone(), cdp, session);
            (speak, captions)
        }
        Err(err) => {
            log::warn!(
                "[meet-audio] audio bridge install failed request_id={request_id} err={err} — speak + caption paths disabled for this call"
            );
            (
                speak_pump::start_disabled(request_id.clone()),
                caption_listener_disabled(request_id.clone()),
            )
        }
    };

    if let Some(state) = app.try_state::<MeetAudioState>() {
        state.inner.lock().unwrap().insert(
            request_id.clone(),
            MeetAudioSession {
                request_id: request_id.clone(),
                _captions: captions,
                _legacy_listen: None,
                _speak: speak,
            },
        );
    } else {
        log::warn!(
            "[meet-audio] MeetAudioState missing from app — session will be ungoverned request_id={request_id}"
        );
    }

    Ok(())
}

/// Stop a meet-agent audio session. Best-effort: errors from individual
/// shutdown steps are logged but never propagated, because window
/// destruction must finish even if e.g. core is unreachable.
pub async fn stop<R: Runtime>(
    app: AppHandle<R>,
    request_id: String,
) -> Result<Option<StopSummary>, String> {
    let session = if let Some(state) = app.try_state::<MeetAudioState>() {
        state.inner.lock().unwrap().remove(&request_id)
    } else {
        None
    };

    let Some(session) = session else {
        log::debug!("[meet-audio] stop: no session for request_id={request_id}");
        return Ok(None);
    };

    // Dropping `session` first releases the audio handler registration
    // (so CEF stops feeding us frames) and signals the pump to exit.
    drop(session);

    match rpc_call(
        "openhuman.meet_agent_stop_session",
        serde_json::json!({ "request_id": request_id }),
    )
    .await
    {
        Ok(v) => {
            let listened = v
                .get("listened_seconds")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0) as f32;
            let spoken = v
                .get("spoken_seconds")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0) as f32;
            let turns = v.get("turn_count").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            log::info!(
                "[meet-audio] stop ok request_id={request_id} listened={listened:.2}s spoken={spoken:.2}s turns={turns}"
            );
            Ok(Some(StopSummary {
                request_id,
                listened_seconds: listened,
                spoken_seconds: spoken,
                turn_count: turns,
            }))
        }
        Err(err) => {
            log::warn!("[meet-audio] stop_session rpc failed request_id={request_id} err={err}");
            Ok(None)
        }
    }
}

/// Minimal JSON-RPC helper used by both this module and the speak pump
/// loop. Mirrors the call shape used by other shell scanners (see
/// `telegram_scanner::mod.rs`).
pub(crate) async fn rpc_call(
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let url = crate::core_rpc::core_rpc_url_value();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let req = crate::core_rpc::apply_auth(client.post(&url))
        .map_err(|e| format!("prepare {url}: {e}"))?;
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    let status = resp.status();
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("decode {status}: {e}"))?;
    if !status.is_success() {
        return Err(format!("{status}: {v}"));
    }
    if let Some(err) = v.get("error") {
        return Err(format!("rpc error: {err}"));
    }
    Ok(v.get("result").cloned().unwrap_or(serde_json::Value::Null))
}

/// No-op caption listener used when CDP attach failed at session
/// start. Lets the rest of the lifecycle hold a uniform value.
fn caption_listener_disabled(request_id: String) -> caption_listener::CaptionListener {
    caption_listener::CaptionListener {
        request_id,
        _shutdown_tx: None,
    }
}

/// Trim a string for logging without panicking on multi-byte chars.
fn truncate_for_log(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_handles_short_strings() {
        assert_eq!(truncate_for_log("hi", 10), "hi");
    }

    #[test]
    fn truncate_caps_long_strings() {
        let long = "a".repeat(100);
        let trimmed = truncate_for_log(&long, 10);
        assert!(trimmed.ends_with('…'));
        assert_eq!(trimmed.chars().count(), 11);
    }
}
