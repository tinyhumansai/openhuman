//! Install the OpenHuman camera bridge into the Meet webview via CDP.
//!
//! ## Why post-reload `Runtime.evaluate`, not `addScriptToEvaluateOnNewDocument`
//!
//! The natural shape would be to mirror [`crate::meet_audio::inject`]:
//! register via `Page.addScriptToEvaluateOnNewDocument`, then ride the
//! audio bridge's `Page.reload` so all three scripts run at
//! document-start. We tried that. With CEF 146 + a 56 KB camera bridge
//! (the inlined mascot SVGs as data URIs are the bulk), registering a
//! third pre-document script consistently crashed the renderer during
//! the reload — `meet-scanner` would see
//! `cdp error: {"code":-32000,"message":"Target crashed"}` within ~1 s
//! of opening, the page was gone before either readiness probe could
//! answer, and the user saw a blank Meet window.
//!
//! The camera bridge only needs to be in place before Meet's first
//! `getUserMedia` call, which happens after the user (or
//! `meet_scanner`) clicks "Ask to join" — multiple seconds after the
//! navigation completes. Plenty of room to inject via
//! `Runtime.evaluate` once the post-reload page is up.
//!
//! Lifecycle:
//! 1. `meet_audio::inject::install_audio_bridge` registers + reloads
//!    (unchanged).
//! 2. After the audio bridge's readiness probe confirms the new doc is
//!    live, [`install_camera_bridge_post_reload`] evaluates the bridge
//!    JS directly. No second reload, no pre-document script.

use serde_json::{json, Value};
use std::time::Duration;

use crate::cdp::CdpConn;

/// Inject the camera bridge into the Meet page's main world via
/// `Runtime.evaluate`. Called *after* the audio bridge's Page.reload
/// has settled, so we land on the live, post-reload document.
///
/// Returns `Ok(())` if the evaluation didn't throw page-side. Errors
/// are non-fatal at the call site: the audio path keeps working and
/// Meet falls back to the static-Y4M outbound camera.
pub async fn install_camera_bridge_post_reload(
    cdp: &mut CdpConn,
    session: &str,
    frame_bus_port: u16,
) -> Result<(), String> {
    let js = super::build_camera_bridge_js(frame_bus_port);
    log::info!(
        "[meet-camera] inject session={session} bridge_chars={} frame_bus_port={frame_bus_port}",
        js.chars().count()
    );
    let res = cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": js,
                // returnByValue:false because the bridge IIFE returns
                // undefined; we only care about exceptionDetails.
                "awaitPromise": false,
            }),
            Some(session),
        )
        .await
        .map_err(|e| format!("Runtime.evaluate(camera bridge): {e}"))?;
    if let Some(exception) = res.get("exceptionDetails") {
        return Err(format!("page exception: {exception}"));
    }
    Ok(())
}

/// Best-effort readiness probe — logs the bridge's self-reported state
/// once it's live. Mirrors the audio bridge's `confirm_bridge_alive`
/// shape so a failure here is observable in the same place.
pub async fn confirm_bridge_alive(cdp: &mut CdpConn, session: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let res = cdp
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": "(typeof window.__openhumanCameraBridgeInfo === 'function') \
                                   ? JSON.stringify(window.__openhumanCameraBridgeInfo()) \
                                   : null",
                    "returnByValue": true,
                }),
                Some(session),
            )
            .await;
        if let Ok(v) = res {
            let value = v
                .get("result")
                .and_then(|r| r.get("value"))
                .cloned()
                .unwrap_or(Value::Null);
            if let Some(s) = value.as_str() {
                log::info!("[meet-camera] bridge alive info={s}");
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    log::warn!("[meet-camera] bridge readiness probe timed out");
}

/// Spawn a background loop that polls `__openhumanCameraBridgeInfo()`
/// over a freshly-attached CDP session every `interval`, computing the
/// per-interval delta in `remoteFrameCount` (effective FPS) and
/// `droppedOutOfOrder` (race incidents). Logs every tick so a tail
/// gives a live timeline of producer/consumer health.
///
/// Lives only when `OPENHUMAN_DEV_MEET_CAMERA_DIAG=1`; otherwise no-op.
/// Self-terminates when the CDP connection closes (e.g. the meet
/// window was destroyed).
pub fn spawn_diagnostics_poller<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    request_id: String,
    meet_url: String,
) {
    let enabled = std::env::var("OPENHUMAN_DEV_MEET_CAMERA_DIAG")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !enabled {
        return;
    }
    log::info!(
        "[meet-camera-diag] poller starting meet_url_chars={}",
        meet_url.chars().count()
    );
    tauri::async_runtime::spawn(async move {
        // Allow the bridge time to install before the first poll.
        tokio::time::sleep(Duration::from_secs(3)).await;
        let label = crate::meet_call::window_label_for(&request_id);
        let meet_url_for_pred = meet_url.clone();
        let pred = move |t: &crate::cdp::target::CdpTarget| -> bool {
            t.url.starts_with(&meet_url_for_pred)
        };
        let (mut cdp, session) =
            match crate::cdp::target::connect_and_attach_matching_in_process_by_label::<R, _>(
                &app, &label, pred,
            )
            .await
            {
                Ok(pair) => pair,
                Err(err) => {
                    log::warn!("[meet-camera-diag] cdp attach failed: {err}");
                    return;
                }
            };
        let mut last_frames: u64 = 0;
        let mut last_dropped: u64 = 0;
        let mut tick = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            tick += 1;
            let res = cdp
                .call(
                    "Runtime.evaluate",
                    json!({
                        "expression": "(typeof window.__openhumanCameraBridgeInfo === 'function') \
                                       ? JSON.stringify(window.__openhumanCameraBridgeInfo()) \
                                       : null",
                        "returnByValue": true,
                    }),
                    Some(&session),
                )
                .await;
            let raw = match res {
                Ok(v) => v
                    .get("result")
                    .and_then(|r| r.get("value"))
                    .and_then(|x| x.as_str().map(|s| s.to_string())),
                Err(err) => {
                    // CDP closed (window gone) → exit cleanly.
                    log::info!("[meet-camera-diag] cdp poll err — exiting: {err}");
                    return;
                }
            };
            let Some(raw) = raw else {
                log::warn!("[meet-camera-diag] tick={tick} bridge info missing");
                continue;
            };
            let parsed: Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(_) => {
                    log::warn!("[meet-camera-diag] tick={tick} parse fail raw={raw}");
                    continue;
                }
            };
            let frames = parsed
                .get("remoteFrameCount")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let dropped = parsed
                .get("droppedOutOfOrder")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let ws_state = parsed
                .get("wsState")
                .and_then(|x| x.as_str())
                .unwrap_or("?");
            let frame = parsed.get("frame").and_then(|x| x.as_u64()).unwrap_or(0);
            let fresh_ms = parsed.get("remoteFreshMs").and_then(|x| x.as_u64());
            let mood = parsed
                .get("currentMood")
                .and_then(|x| x.as_str())
                .unwrap_or("?");
            let port = parsed
                .get("frameBusPort")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let source = parsed
                .get("lastDrawSource")
                .and_then(|x| x.as_str())
                .unwrap_or("?");
            let canvas_probe = parsed.get("canvasProbe").unwrap_or(&Value::Null);
            let canvas_luma = canvas_probe.get("avgLuma").and_then(|x| x.as_i64());
            let canvas_min = canvas_probe.get("minLuma").and_then(|x| x.as_i64());
            let canvas_max = canvas_probe.get("maxLuma").and_then(|x| x.as_i64());
            let remote_bitmap = parsed.get("lastRemoteBitmapInfo").unwrap_or(&Value::Null);
            let remote_w = remote_bitmap.get("width").and_then(|x| x.as_u64());
            let remote_h = remote_bitmap.get("height").and_then(|x| x.as_u64());
            let remote_bytes = remote_bitmap.get("bytes").and_then(|x| x.as_u64());
            let outbound = parsed.get("outboundVideoStats").unwrap_or(&Value::Null);
            let frames_encoded = outbound.get("framesEncoded").and_then(|x| x.as_u64());
            let bytes_sent = outbound.get("bytesSent").and_then(|x| x.as_u64());
            let encoded_w = outbound.get("frameWidth").and_then(|x| x.as_u64());
            let encoded_h = outbound.get("frameHeight").and_then(|x| x.as_u64());
            let quality_reason = outbound
                .get("qualityLimitationReason")
                .and_then(|x| x.as_str())
                .unwrap_or("?");
            let video_elements = parsed
                .get("videoElements")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
            let video_count = video_elements.len();
            let mascot_video = video_elements.iter().find(|video| {
                video
                    .get("tracks")
                    .and_then(|x| x.as_array())
                    .map(|tracks| {
                        tracks.iter().any(|track| {
                            track
                                .get("label")
                                .and_then(|x| x.as_str())
                                .map(|label| label.contains("OpenHuman Mascot"))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            });
            let mascot_video_summary = mascot_video
                .map(|video| {
                    let vw = video.get("videoWidth").and_then(|x| x.as_u64());
                    let vh = video.get("videoHeight").and_then(|x| x.as_u64());
                    let ready = video.get("readyState").and_then(|x| x.as_u64());
                    let luma = video
                        .get("luma")
                        .and_then(|x| x.get("avgLuma"))
                        .and_then(|x| x.as_i64());
                    format!("w={vw:?} h={vh:?} ready={ready:?} luma={luma:?}")
                })
                .unwrap_or_else(|| "none".to_string());
            let delta_frames = frames.saturating_sub(last_frames);
            let delta_dropped = dropped.saturating_sub(last_dropped);
            let fps = (delta_frames as f32) / 2.0;
            log::info!(
                "[meet-camera-diag] tick={tick} ws={ws_state} port={port} \
                 frames_total={frames} fps_2s={fps:.1} \
                 dropped_total={dropped} new_dropped={delta_dropped} \
                 fresh_ms={fresh_ms:?} bridge_frame={frame} mood={mood} source={source} \
                 remote={remote_w:?}x{remote_h:?}/{remote_bytes:?}B \
                 canvas_luma={canvas_luma:?}/{canvas_min:?}-{canvas_max:?} \
                 outbound_frames={frames_encoded:?} outbound_bytes={bytes_sent:?} \
                 outbound_size={encoded_w:?}x{encoded_h:?} quality={quality_reason} \
                 videos={video_count} mascot_video={mascot_video_summary}"
            );
            last_frames = frames;
            last_dropped = dropped;
        }
    });
}

/// Host-side mood control. Future hookup: the meet-agent state machine
/// (`src/openhuman/meet_agent/session.rs`) calls this on phase
/// transitions so the camera reflects what the agent is actually doing
/// instead of running on the JS-side 5s auto-toggle. Until that's
/// wired, the bridge's own `setInterval` provides the visible toggle.
#[allow(dead_code)]
pub async fn set_mood(cdp: &mut CdpConn, session: &str, mood: &str) -> Result<(), String> {
    // Mood is an internal enum — guard against accidental injection
    // even though the call site is internal.
    if !mood.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("invalid mood: {mood}"));
    }
    let expression = format!(
        "(typeof window.__openhumanSetMood === 'function') \
         ? window.__openhumanSetMood('{mood}') : false"
    );
    let res = cdp
        .call(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
            Some(session),
        )
        .await
        .map_err(|e| format!("Runtime.evaluate set_mood: {e}"))?;
    if let Some(exception) = res.get("exceptionDetails") {
        return Err(format!("page exception: {exception}"));
    }
    Ok(())
}
