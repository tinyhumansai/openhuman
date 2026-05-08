//! Install the OpenHuman camera bridge into the Meet webview via CDP.
//!
//! Call shape mirrors [`crate::meet_audio::inject`]: one
//! `Page.addScriptToEvaluateOnNewDocument` per bridge, then a single
//! `Page.reload` to make sure the script runs on the live document.
//! Since the audio inject path already does the attach + reload, the
//! camera bridge piggybacks on the same CDP session via
//! [`install_camera_bridge_on_session`] — no second attach needed.

use serde_json::{json, Value};
use std::time::Duration;

use crate::cdp::CdpConn;

/// Install the camera bridge using an already-attached CDP session.
/// Called by `meet_audio::inject::install_audio_bridge` after the audio
/// + captions bridges are registered, so the single `Page.reload` that
/// follows boots all three.
pub async fn install_camera_bridge_on_session(
    cdp: &mut CdpConn,
    session: &str,
) -> Result<(), String> {
    let js = super::build_camera_bridge_js();
    log::info!(
        "[meet-camera] inject session={session} bridge_chars={}",
        js.chars().count()
    );
    cdp.call(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": js }),
        Some(session),
    )
    .await
    .map_err(|e| format!("addScriptToEvaluateOnNewDocument(camera): {e}"))?;
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
