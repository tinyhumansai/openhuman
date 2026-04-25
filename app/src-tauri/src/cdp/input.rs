//! Thin helpers around `Input.dispatchMouseEvent` and
//! `Input.dispatchKeyEvent` so providers can drive web UIs without
//! touching the page's JavaScript.
//!
//! All coordinates are CSS pixels relative to the viewport — the same
//! frame `DOMSnapshot.captureSnapshot(includeDOMRects=true)` returns
//! bounding rects in. Callers typically pair these with
//! [`crate::cdp::Snapshot::rect`] to find the click target.
//!
//! Everything here is CEF-only — CDP requires a remote-debugging port,
//! which wry doesn't expose.
//!
//! # Cookbook
//!
//! ```ignore
//! let snap = Snapshot::capture_with_rects(&mut cdp, &session).await?;
//! let idx = snap.find_descendant(0, |s, i| s.attr(i, "aria-label") == Some("Search mail"))
//!     .ok_or("search box not found")?;
//! let rect = snap.rect(idx).ok_or("search box has no layout rect")?;
//! let (cx, cy) = rect.center();
//! input::click(&mut cdp, &session, cx, cy).await?;
//! input::type_text(&mut cdp, &session, "from:linkedin.com").await?;
//! input::press_key(&mut cdp, &session, Key::Enter).await?;
//! ```

use serde_json::{json, Value};

use super::CdpConn;

#[allow(dead_code)] // helper is used by currently gated input paths.
#[cfg(target_os = "macos")]
const SELECT_ALL_MODIFIER: u32 = 4;
#[allow(dead_code)] // helper is used by currently gated input paths.
#[cfg(not(target_os = "macos"))]
const SELECT_ALL_MODIFIER: u32 = 2;

/// Names recognised by `Input.dispatchKeyEvent`'s `key` field. We
/// hand-pick the ones Gmail's keyboard handlers care about so callers
/// can use a typed value rather than stringly-typed literals scattered
/// across providers.
#[allow(dead_code)] // variants reserved for upcoming providers / write ops.
#[derive(Debug, Clone, Copy)]
pub enum Key {
    Enter,
    Escape,
    Tab,
    Backspace,
    ArrowDown,
    ArrowUp,
}

impl Key {
    /// `(key, code, windowsVirtualKeyCode)` triple. Gmail's listeners
    /// branch on different fields depending on browser; we set all three
    /// to maximise compatibility.
    fn cdp_fields(self) -> (&'static str, &'static str, u32) {
        match self {
            Key::Enter => ("Enter", "Enter", 13),
            Key::Escape => ("Escape", "Escape", 27),
            Key::Tab => ("Tab", "Tab", 9),
            Key::Backspace => ("Backspace", "Backspace", 8),
            Key::ArrowDown => ("ArrowDown", "ArrowDown", 40),
            Key::ArrowUp => ("ArrowUp", "ArrowUp", 38),
        }
    }
}

/// Click at `(x, y)` — left button, no modifiers, single click.
/// Issues mouseMoved → mousePressed → mouseReleased so hover handlers
/// (Gmail's search-box has one) fire correctly before the click.
pub async fn click(cdp: &mut CdpConn, session: &str, x: f64, y: f64) -> Result<(), String> {
    log::debug!("[cdp::input] click session={session} x={x:.1} y={y:.1}");
    let _ = mouse_event(cdp, session, "mouseMoved", x, y, 0).await?;
    let _ = mouse_event(cdp, session, "mousePressed", x, y, 1).await?;
    let _ = mouse_event(cdp, session, "mouseReleased", x, y, 1).await?;
    log::debug!("[cdp::input] click complete session={session}");
    Ok(())
}

async fn mouse_event(
    cdp: &mut CdpConn,
    session: &str,
    kind: &str,
    x: f64,
    y: f64,
    click_count: u32,
) -> Result<Value, String> {
    log::debug!(
        "[cdp::input] mouse_event session={session} kind={kind} x={x:.1} y={y:.1} clicks={click_count}"
    );
    cdp.call(
        "Input.dispatchMouseEvent",
        json!({
            "type": kind,
            "x": x,
            "y": y,
            "button": "left",
            "buttons": if kind == "mousePressed" { 1 } else { 0 },
            "clickCount": click_count,
        }),
        Some(session),
    )
    .await
    .map_err(|e| format!("Input.dispatchMouseEvent {kind}: {e}"))
}

/// Type a literal string by dispatching one `keyDown`/`char`/`keyUp`
/// triple per character. CDP's `dispatchKeyEvent type=char` is what
/// actually inserts text into focused editable fields — `keyDown`
/// alone leaves the input empty for most letters. The `keyDown`
/// + `keyUp` pair is still needed so listeners (autocomplete,
/// keystroke counters) see a normal keystroke.
pub async fn type_text(cdp: &mut CdpConn, session: &str, text: &str) -> Result<(), String> {
    log::debug!(
        "[cdp::input] type_text session={session} chars={}",
        text.chars().count()
    );
    for ch in text.chars() {
        let s = ch.to_string();
        // keyDown — Gmail's command/keyboard router observes these.
        cdp.call(
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyDown",
                "text": s,
                "unmodifiedText": s,
                "key": s,
            }),
            Some(session),
        )
        .await
        .map_err(|e| format!("Input.dispatchKeyEvent keyDown {ch:?}: {e}"))?;
        // char — actual text insertion into the focused editable.
        cdp.call(
            "Input.dispatchKeyEvent",
            json!({
                "type": "char",
                "text": s,
                "unmodifiedText": s,
                "key": s,
            }),
            Some(session),
        )
        .await
        .map_err(|e| format!("Input.dispatchKeyEvent char {ch:?}: {e}"))?;
        cdp.call(
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyUp",
                "text": s,
                "unmodifiedText": s,
                "key": s,
            }),
            Some(session),
        )
        .await
        .map_err(|e| format!("Input.dispatchKeyEvent keyUp {ch:?}: {e}"))?;
    }
    log::debug!("[cdp::input] type_text complete session={session}");
    Ok(())
}

/// Press a non-character key (Enter, Esc, …). Sends `rawKeyDown` →
/// `keyUp`; no `char` because non-printables don't insert text.
pub async fn press_key(cdp: &mut CdpConn, session: &str, key: Key) -> Result<(), String> {
    let (key_name, code, vk) = key.cdp_fields();
    log::debug!("[cdp::input] press_key session={session} key={key_name}");
    cdp.call(
        "Input.dispatchKeyEvent",
        json!({
            "type": "rawKeyDown",
            "key": key_name,
            "code": code,
            "windowsVirtualKeyCode": vk,
            "nativeVirtualKeyCode": vk,
        }),
        Some(session),
    )
    .await
    .map_err(|e| format!("Input.dispatchKeyEvent rawKeyDown {key_name}: {e}"))?;
    cdp.call(
        "Input.dispatchKeyEvent",
        json!({
            "type": "keyUp",
            "key": key_name,
            "code": code,
            "windowsVirtualKeyCode": vk,
            "nativeVirtualKeyCode": vk,
        }),
        Some(session),
    )
    .await
    .map_err(|e| format!("Input.dispatchKeyEvent keyUp {key_name}: {e}"))?;
    log::debug!("[cdp::input] press_key complete session={session} key={key_name}");
    Ok(())
}

/// Dispatch Cmd/Ctrl+A to select-all in the focused contenteditable / input.
/// Useful when the search box already has a previous query in it that
/// we need to overwrite — Gmail keeps the last query rendered in the
/// search input so a fresh visit sees stale text.
pub async fn select_all_in_focused(cdp: &mut CdpConn, session: &str) -> Result<(), String> {
    log::debug!(
        "[cdp::input] select_all_in_focused session={session} modifier={SELECT_ALL_MODIFIER}"
    );
    cdp.call(
        "Input.dispatchKeyEvent",
        json!({
            "type": "rawKeyDown",
            "key": "a",
            "code": "KeyA",
            "windowsVirtualKeyCode": 65,
            "nativeVirtualKeyCode": 65,
            "modifiers": SELECT_ALL_MODIFIER,
        }),
        Some(session),
    )
    .await
    .map_err(|e| format!("Input.dispatchKeyEvent select-all keyDown: {e}"))?;
    cdp.call(
        "Input.dispatchKeyEvent",
        json!({
            "type": "keyUp",
            "key": "a",
            "code": "KeyA",
            "windowsVirtualKeyCode": 65,
            "nativeVirtualKeyCode": 65,
            "modifiers": SELECT_ALL_MODIFIER,
        }),
        Some(session),
    )
    .await
    .map_err(|e| format!("Input.dispatchKeyEvent select-all keyUp: {e}"))?;
    log::debug!("[cdp::input] select_all_in_focused complete session={session}");
    Ok(())
}
