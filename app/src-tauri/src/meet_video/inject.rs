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
) -> Result<(), String> {
    let js = super::build_camera_bridge_js();
    log::info!(
        "[meet-camera] inject session={session} bridge_chars={}",
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
