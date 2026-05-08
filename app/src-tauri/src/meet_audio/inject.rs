//! Install the OpenHuman audio bridge into the Meet webview via CDP.
//!
//! ## Why this can't live in the runtime
//!
//! The listen path uses CEF's public `cef_audio_handler_t` API and
//! needs no Chromium changes. The speak path is the opposite: there is
//! no public API for *writing* PCM into a renderer's audio input, and
//! the Chromium-internal `FileSource` that backs
//! `--use-file-for-fake-audio-capture` only reads a static WAV. Our
//! options are:
//!
//!   - Patch Chromium and rebuild from source (multi-day; we don't
//!     maintain a CEF source build pipeline yet).
//!   - Inject a tiny Web Audio bridge into the Meet page over CDP.
//!
//! This module implements the second path. It runs once per call,
//! after the meet-call window opens but before [`crate::meet_scanner`]
//! starts driving the join page:
//!
//! 1. Attach a CDP session to the Meet target (or about:blank — see
//!    note on initial URL below).
//! 2. `Page.addScriptToEvaluateOnNewDocument` with
//!    [`AUDIO_BRIDGE_JS`] so it runs at document-start of the *next*
//!    document load.
//! 3. `Page.reload` so even an already-navigated Meet page picks up
//!    the override before its first `getUserMedia` call.
//!
//! ## Why a reload (rather than starting at about:blank)
//!
//! `meet_call_open_window` builds the WebviewWindow with the Meet URL
//! directly. Refactoring it to navigate via CDP would change the
//! lifecycle for every other code path that watches the meet window,
//! including `meet_scanner`'s target-URL prefix matching. A one-time
//! reload is surgical: meet_scanner already polls for the meet target
//! and tolerates re-navigation.

use std::time::Duration;

use serde_json::{json, Value};

use crate::cdp::{self, CdpConn};

/// JS bundled at build time — the actual Web Audio bridge lives in the
/// sibling `audio_bridge.js`. `include_str!` bakes it into the binary
/// so there's nothing to copy at install.
pub const AUDIO_BRIDGE_JS: &str = include_str!("audio_bridge.js");

/// Captions bridge — DOM observer over Meet's live captions region
/// plus auto-enable for the CC button. Installed alongside the audio
/// bridge so a single `Page.reload` boots both.
pub const CAPTIONS_BRIDGE_JS: &str = include_str!("captions_bridge.js");

/// How long we wait for CDP to surface the meet target after the
/// window builds. Mirrors [`crate::meet_scanner::TARGET_DISCOVERY_BUDGET`]
/// so the two scanners share a budget shape.
const TARGET_DISCOVERY_BUDGET: Duration = Duration::from_secs(20);
const TARGET_DISCOVERY_INTERVAL: Duration = Duration::from_millis(500);

/// Run the inject + reload sequence. Returns the attached CDP
/// connection + session id so the caller (the speak pump) can keep
/// using it for `Runtime.evaluate` calls — opening one CDP session
/// per call rather than per pump tick saves ~5 ms per push.
pub async fn install_audio_bridge(meet_url: &str) -> Result<(CdpConn, String), String> {
    let (mut cdp, session) = wait_for_meet_target(meet_url).await?;
    log::info!(
        "[meet-audio] inject attached session={} meet_url_chars={}",
        session,
        meet_url.chars().count()
    );

    // Page.enable is required before some build's reload events fire
    // ordering callbacks; harmless on builds where it isn't.
    let _ = cdp.call("Page.enable", json!({}), Some(&session)).await;
    let _ = cdp.call("Runtime.enable", json!({}), Some(&session)).await;

    cdp.call(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": AUDIO_BRIDGE_JS }),
        Some(&session),
    )
    .await
    .map_err(|e| format!("addScriptToEvaluateOnNewDocument(audio): {e}"))?;

    cdp.call(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": CAPTIONS_BRIDGE_JS }),
        Some(&session),
    )
    .await
    .map_err(|e| format!("addScriptToEvaluateOnNewDocument(captions): {e}"))?;

    // Camera bridge: monkey-patches getUserMedia so the agent's outbound
    // video is a programmatic mascot canvas rather than the static Y4M
    // backed by --use-file-for-fake-video-capture. Installed on the same
    // CDP session as the audio + captions bridges so a single Page.reload
    // boots all three. See `crate::meet_video` for the rationale.
    if let Err(err) = crate::meet_video::inject::install_camera_bridge_on_session(
        &mut cdp,
        &session,
    )
    .await
    {
        // Non-fatal: a missing camera bridge falls back to the static
        // Y4M, so the agent still has *some* outbound video. Audio +
        // captions remain functional.
        log::warn!("[meet-audio] camera bridge install failed: {err} (falling back to static Y4M)");
    }

    // Reload so the script applies to the (already-loaded) meet page.
    // `ignoreCache: true` defeats the bfcache so we get a real
    // document-start hook for the bridge.
    cdp.call(
        "Page.reload",
        json!({ "ignoreCache": true }),
        Some(&session),
    )
    .await
    .map_err(|e| format!("Page.reload: {e}"))?;

    log::info!("[meet-audio] inject reload requested session={session}");

    // Confirm the bridge is live before we return — saves the speak
    // pump from sending its first chunk into a void if the script
    // failed to run for any reason. Best-effort: a missing bridge
    // logs and we still return Ok so the listen path keeps working.
    confirm_bridge_alive(&mut cdp, &session).await;
    crate::meet_video::inject::confirm_bridge_alive(&mut cdp, &session).await;

    Ok((cdp, session))
}

async fn wait_for_meet_target(meet_url: &str) -> Result<(CdpConn, String), String> {
    let deadline = tokio::time::Instant::now() + TARGET_DISCOVERY_BUDGET;
    let mut last_err = String::new();
    while tokio::time::Instant::now() < deadline {
        match cdp::connect_and_attach_matching(|t| t.url.starts_with(meet_url)).await {
            Ok(pair) => return Ok(pair),
            Err(err) => {
                last_err = err;
                tokio::time::sleep(TARGET_DISCOVERY_INTERVAL).await;
            }
        }
    }
    Err(format!(
        "[meet-audio] timeout waiting for meet target: {last_err}"
    ))
}

/// Poll `window.__openhumanAudioBridgeInfo()` for up to ~5 s. Logs the
/// outcome but never returns an error — the speak pump will rediscover
/// the bridge on the next push if it shows up late.
async fn confirm_bridge_alive(cdp: &mut CdpConn, session: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let res = cdp
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": "(typeof window.__openhumanAudioBridgeInfo === 'function') \
                                   ? JSON.stringify(window.__openhumanAudioBridgeInfo()) \
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
                log::info!("[meet-audio] bridge alive info={s}");
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    log::warn!("[meet-audio] bridge readiness probe timed out — speak pump will retry");
}

/// Drain the page-side caption queue. Returns 0 or more `(speaker,
/// text, ts_ms)` triples accumulated since the last drain. The caller
/// (the caption listener loop) calls this every ~500 ms.
pub async fn drain_captions(
    cdp: &mut CdpConn,
    session: &str,
) -> Result<Vec<(String, String, u64)>, String> {
    let res = cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": "(typeof window.__openhumanDrainCaptions === 'function') \
                               ? JSON.stringify(window.__openhumanDrainCaptions()) \
                               : '[]'",
                "returnByValue": true,
            }),
            Some(session),
        )
        .await
        .map_err(|e| format!("Runtime.evaluate drain_captions: {e}"))?;
    let json_str = res
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("[]")
        .to_string();
    let parsed: Vec<Value> =
        serde_json::from_str(&json_str).map_err(|e| format!("parse captions json: {e}"))?;
    let mut out = Vec::with_capacity(parsed.len());
    for entry in parsed {
        let speaker = entry
            .get("speaker")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let text = entry
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let ts_ms = entry.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
        if text.is_empty() {
            continue;
        }
        out.push((speaker, text, ts_ms));
    }
    Ok(out)
}

/// Dispatch one PCM chunk into the page's bridge. Called on every
/// poll-speech tick by [`crate::meet_audio::speak_pump`].
///
/// Errors are returned (rather than logged inline) so the pump can
/// decide whether to back off — repeated failures usually mean the
/// page navigated away (e.g. "you've been removed from the call"),
/// which the meet-call lifecycle handles by tearing the whole session
/// down anyway.
pub async fn feed_pcm_chunk(cdp: &mut CdpConn, session: &str, pcm_b64: &str) -> Result<(), String> {
    if pcm_b64.is_empty() {
        return Ok(());
    }
    // Build the call as a string literal so a long base64 payload
    // travels as a JS source argument (CDP's Runtime.callFunctionOn
    // would be cleaner but requires the bridge function's objectId,
    // and Runtime.evaluate keeps the wire shape one round-trip).
    //
    // The b64 alphabet has no quote / backslash characters so a plain
    // single-quoted literal is safe — but defensively escape just in
    // case some future encoder produces padding-edge weirdness.
    let escaped = pcm_b64.replace('\\', "\\\\").replace('\'', "\\'");
    let expression = format!(
        "(typeof window.__openhumanFeedPcm === 'function') ? window.__openhumanFeedPcm('{escaped}') : -1"
    );
    let res = cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
            }),
            Some(session),
        )
        .await
        .map_err(|e| format!("Runtime.evaluate feed: {e}"))?;
    if let Some(exception) = res.get("exceptionDetails") {
        return Err(format!("page exception: {exception}"));
    }
    Ok(())
}
