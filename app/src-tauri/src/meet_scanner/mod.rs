//! CDP-driven Meet join automation.
//!
//! Runs once per call, after `meet_call::meet_call_open_window` has
//! successfully built the dedicated CEF webview. Connects to CEF's
//! browser-level WebSocket, attaches to the new Meet target, and walks
//! through the join page in three phases:
//!
//!  1. Dismiss the device-check ("Continue without microphone and camera").
//!  2. Type the supplied guest display name into the "Your name" input.
//!  3. Click "Ask to join".
//!
//! All steps go through CDP from this scanner side — there is **no**
//! init-script JS injected into the webview. `Runtime.evaluate` is used
//! to find candidate elements by visible text / aria-label, and
//! `Input.insertText` to inject the display name as a synthetic IME
//! event so Meet's React-controlled `<input>` actually picks it up.
//!
//! The whole sequence is best-effort: if any phase times out we log and
//! bail without crashing the window — the user can finish joining
//! manually. Future work: emit lifecycle events back to the frontend so
//! the UI can show "asking host…" / "joined" status.

use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Runtime};

use crate::cdp::{self, CdpConn};

/// Wait at most this long for CEF to surface the new Meet page target
/// after `WebviewWindowBuilder::build()` returns. CEF lazy-creates the
/// renderer-side target a few hundred ms after the host-side window is
/// ready.
const TARGET_DISCOVERY_BUDGET: Duration = Duration::from_secs(20);
const TARGET_DISCOVERY_INTERVAL: Duration = Duration::from_millis(500);

/// Per-phase polling budgets. With the mascot fake-camera flag set
/// process-wide in `lib.rs`, Meet sees a "real" webcam and does NOT
/// show the "Continue without microphone and camera" screen at all,
/// so the device-check phase becomes a quick best-effort probe rather
/// than a meaningful wait. We still keep the phase in case a future
/// build runs without the fake-camera flag (or the Y4M failed to
/// rasterize), but cap it tight so the join flow doesn't stall.
const DEVICE_CHECK_BUDGET: Duration = Duration::from_secs(6);
const NAME_INPUT_BUDGET: Duration = Duration::from_secs(30);
const JOIN_BUTTON_BUDGET: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Spawn the CDP-driven join automation. Fire-and-forget — the caller
/// (the Tauri command that just opened the window) doesn't wait for it.
pub fn spawn<R: Runtime>(_app: AppHandle<R>, request_id: String, display_name: String) {
    tauri::async_runtime::spawn(async move {
        match run(&request_id, &display_name).await {
            Ok(()) => log::info!("[meet-scanner] join sequence completed request_id={request_id}"),
            Err(err) => {
                log::warn!("[meet-scanner] join sequence aborted request_id={request_id} err={err}")
            }
        }
    });
}

async fn run(request_id: &str, display_name: &str) -> Result<(), String> {
    let (mut cdp, session) = wait_for_meet_target().await?;
    log::info!("[meet-scanner] attached to meet target request_id={request_id} session={session}");

    // `Runtime.enable` is required before `Runtime.evaluate` returns
    // structured results in some CEF builds. `Page.enable` is harmless
    // and gives us frame-lifecycle events for free if a future PR wants
    // them. Both are best-effort — if they fail we still try to evaluate.
    let _ = cdp.call("Page.enable", json!({}), Some(&session)).await;
    let _ = cdp.call("Runtime.enable", json!({}), Some(&session)).await;

    // Phase 1 — dismiss the device-check screen.
    //
    // Meet's exact copy varies by region/A-B test; we try the canonical
    // English variants. The button is usually `[role="button"]` not
    // `<button>`, so `wait_and_click_text` looks at both.
    if let Err(err) = wait_and_click_text(
        &mut cdp,
        &session,
        &[
            "Continue without microphone and camera",
            "Continue without microphone",
            "Continue without camera",
        ],
        DEVICE_CHECK_BUDGET,
    )
    .await
    {
        log::info!("[meet-scanner] device-check dismissal not needed or unavailable: {err}");
    }

    // Phase 2 — type the display name.
    type_into_named_input(&mut cdp, &session, "Your name", display_name).await?;

    // Phase 3 — request to join.
    wait_and_click_text(
        &mut cdp,
        &session,
        &["Ask to join", "Join now"],
        JOIN_BUTTON_BUDGET,
    )
    .await?;

    Ok(())
}

/// Poll CEF's target list until a page whose URL is on `meet.google.com`
/// shows up, then attach a CDP session to it. We can't filter on the
/// per-call `request_id` because Meet uses opaque meeting codes in the
/// URL, but in practice only the just-opened webview hits this host
/// inside the per-call data directory, so a host match is enough.
async fn wait_for_meet_target() -> Result<(CdpConn, String), String> {
    let deadline = tokio::time::Instant::now() + TARGET_DISCOVERY_BUDGET;
    let mut last_err = String::new();
    while tokio::time::Instant::now() < deadline {
        match cdp::connect_and_attach_matching(|t| t.url.starts_with("https://meet.google.com/"))
            .await
        {
            Ok(pair) => return Ok(pair),
            Err(err) => {
                last_err = err;
                tokio::time::sleep(TARGET_DISCOVERY_INTERVAL).await;
            }
        }
    }
    Err(format!(
        "timeout waiting for meet.google.com target: {last_err}"
    ))
}

/// Repeatedly evaluate a click-by-text helper in the page until either
/// a click lands or `budget` elapses.
async fn wait_and_click_text(
    cdp: &mut CdpConn,
    session: &str,
    labels: &[&str],
    budget: Duration,
) -> Result<(), String> {
    let labels_js = serde_json::to_string(labels).map_err(|e| format!("labels json: {e}"))?;
    let expression = format!(
        r#"
        (() => {{
          const labels = {labels_js};
          const want = labels.map(l => l.toLowerCase());
          const candidates = document.querySelectorAll(
            'button, [role="button"], a[role="button"]'
          );
          for (const el of candidates) {{
            if (el.disabled || el.getAttribute('aria-disabled') === 'true') continue;
            const text = ((el.innerText || el.textContent) || '').trim().toLowerCase();
            if (!text) continue;
            if (!want.some(w => text.includes(w))) continue;
            const rect = el.getBoundingClientRect();
            if (rect.width === 0 || rect.height === 0) continue;
            el.scrollIntoView({{ block: 'center', inline: 'center' }});
            el.click();
            return text;
          }}
          return null;
        }})()
        "#
    );

    let deadline = tokio::time::Instant::now() + budget;
    let mut last_value = Value::Null;
    while tokio::time::Instant::now() < deadline {
        let res = cdp
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": false,
                }),
                Some(session),
            )
            .await?;
        let value = res
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null);
        if value.is_string() {
            log::info!(
                "[meet-scanner] clicked element matching {labels:?} text={}",
                value.as_str().unwrap_or("")
            );
            return Ok(());
        }
        last_value = value;
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    Err(format!(
        "timeout waiting for clickable element matching {labels:?} (last={last_value})"
    ))
}

/// Focus an `<input>` whose `aria-label` or `placeholder` contains
/// `hint`, then dispatch the supplied text via `Input.insertText` so
/// Meet's React-controlled input picks it up as a real keystroke.
async fn type_into_named_input(
    cdp: &mut CdpConn,
    session: &str,
    hint: &str,
    text: &str,
) -> Result<(), String> {
    let hint_js = serde_json::to_string(hint).map_err(|e| format!("hint json: {e}"))?;
    let focus_expr = format!(
        r#"
        (() => {{
          const hint = {hint_js}.toLowerCase();
          const inputs = document.querySelectorAll('input');
          for (const inp of inputs) {{
            const t = (inp.getAttribute('type') || 'text').toLowerCase();
            if (t !== 'text' && t !== 'search') continue;
            const aria = (inp.getAttribute('aria-label') || '').toLowerCase();
            const ph = (inp.placeholder || '').toLowerCase();
            if (!aria.includes(hint) && !ph.includes(hint)) continue;
            inp.focus();
            inp.click();
            // Clear any value already there so we don't append to a
            // half-typed name from a previous attempt.
            try {{ inp.select(); }} catch (_) {{}}
            return true;
          }}
          return false;
        }})()
        "#
    );

    let deadline = tokio::time::Instant::now() + NAME_INPUT_BUDGET;
    while tokio::time::Instant::now() < deadline {
        let res = cdp
            .call(
                "Runtime.evaluate",
                json!({ "expression": focus_expr, "returnByValue": true }),
                Some(session),
            )
            .await?;
        let focused = res
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if focused {
            cdp.call("Input.insertText", json!({ "text": text }), Some(session))
                .await?;
            log::info!(
                "[meet-scanner] inserted display name (hint={hint} chars={})",
                text.chars().count()
            );
            return Ok(());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    Err(format!("timeout waiting for input matching hint={hint}"))
}
