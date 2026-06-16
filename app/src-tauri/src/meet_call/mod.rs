//! Tauri command surface for the "Join a Google Meet call" feature.
//!
//! The core (`src/openhuman/meet/`) validates the meet URL + display name
//! and mints a `request_id`. The frontend then invokes
//! [`meet_call_open_window`] to actually pop a top-level CEF webview that
//! navigates to the Meet URL with a fresh data directory so the join is
//! anonymous (no leaked cookies from any other Google session).
//!
//! ## Why a top-level window and not a child of the main webview?
//!
//! Meet calls are a discrete activity the user wants to see (and resize /
//! position) independently of the OpenHuman main window. The existing
//! `webview_accounts` machinery is account-bound and embeds child
//! webviews inside the main window — the wrong shape for an ad-hoc call.
//!
//! ## What about CDP automation (typing the name, clicking "Ask to
//! join")?
//!
//! Out of scope for this initial cut. The window opens at the Meet URL;
//! the user (or, in a follow-up, a `meet_scanner` module mirroring the
//! `whatsapp_scanner` pattern) handles the join page. No JS is injected
//! into this webview — per the project rule for embedded provider
//! webviews.
//!
//! ## Scanner teardown and the 60-second navigation block
//!
//! `meet_scanner::spawn` returns an `AbortHandle` that we store in
//! `MeetCallState`. When a close signal arrives — whether from the user
//! clicking our "Leave" button (`meet_call_close_window`) **or** from the
//! OS title bar — `WindowEvent::CloseRequested` fires and we abort the
//! scanner immediately. Without this abort the scanner's CDP polling loops
//! (NAME_INPUT_BUDGET + JOIN_BUTTON_BUDGET, up to 60 s) keep WebSocket
//! connections open to CEF's debugging endpoint. CEF waits for all active
//! CDP sessions to detach before completing renderer shutdown, so an
//! un-cancelled scanner delays `WindowEvent::Destroyed` — and therefore
//! the `meet-call:closed` frontend event — by up to 60 s, blocking
//! navigation. See issue #1378.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Deserialize;
use tauri::{webview::WebviewWindowBuilder, AppHandle, Emitter, Manager, Runtime, WebviewUrl};
use tokio::task::AbortHandle;
use url::Url;

use crate::meet_scanner;

/// Per-process registry of open Meet webview windows, keyed by
/// `request_id` so the frontend can ask us to close a specific call.
///
/// `scanner_aborts` stores the abort handle returned by
/// [`meet_scanner::spawn`] so `CloseRequested` can cancel the join
/// automation before CEF starts renderer shutdown. Aborting the scanner
/// drops its CDP connections, which unblocks the window destruction
/// sequence. See the module-level doc for details.
pub struct MeetCallState {
    /// request_id → window label
    inner: Mutex<HashMap<String, String>>,
    /// request_id → scanner task abort handle
    scanner_aborts: Mutex<HashMap<String, AbortHandle>>,
}

impl MeetCallState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            scanner_aborts: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for MeetCallState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
pub struct OpenWindowArgs {
    pub request_id: String,
    pub meet_url: String,
    /// Bot's Meet participant tile name — what the bot types into
    /// Meet's "Your name" input. Also passed to the core wake gate
    /// so the bot's own captioned TTS is filtered out as self-echo.
    pub display_name: String,
    /// Call owner's Meet participant name — the human who launched
    /// the bot. The core wake-word gate (privacy lock: only the
    /// owner can trigger tool calls) compares speaker captions
    /// against this value. Defaulted to empty so callers staged
    /// during the rollout window keep parsing; an empty owner
    /// fails closed in core (no wakes fire).
    #[serde(default)]
    pub owner_display_name: String,
}

/// Open a dedicated top-level CEF webview window pointed at the Meet URL.
///
/// The window label is derived from `request_id` so concurrent calls
/// don't collide. A fresh `app_local_data_dir/meet_call/<request_id>`
/// directory keeps cookies isolated — Google Meet treats us as a brand
/// new anonymous user. The window emits `meet-call:closed` when the user
/// closes it so the frontend can clean up its in-flight call list.
#[tauri::command]
pub async fn meet_call_open_window<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, MeetCallState>,
    args: OpenWindowArgs,
) -> Result<String, String> {
    let request_id = sanitize_request_id(&args.request_id)?;
    let parsed = Url::parse(args.meet_url.trim())
        .map_err(|e| format!("[meet-call] invalid meet_url: {e}"))?;
    if parsed.scheme() != "https" || parsed.host_str() != Some("meet.google.com") {
        return Err("[meet-call] only https://meet.google.com URLs are accepted".into());
    }

    let label = window_label_for(&request_id);

    if let Some(existing) = app.get_webview_window(&label) {
        log::info!("[meet-call] reusing existing window label={label} request_id={request_id}");
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(label);
    }

    // Only one meet-call window can be live at a time — concurrent bot
    // sessions race the CEF audio handler registration (`listen_capture`)
    // and confuse the user with multiple "Meet — OpenHuman" windows in
    // their Dock. Close any stragglers from a prior Join before opening
    // a fresh one. The CloseRequested handler will tear down their
    // scanner + audio session via the per-window event listeners below.
    let stale_labels: Vec<String> = app
        .webview_windows()
        .keys()
        .filter(|l| l.starts_with("meet-call-"))
        .cloned()
        .collect();
    for stale in stale_labels {
        if let Some(window) = app.get_webview_window(&stale) {
            log::info!("[meet-call] closing stale window label={stale} before new join");
            let _ = window.close();
        }
    }

    let data_dir = data_directory_for(&app, &request_id)?;
    if let Err(err) = std::fs::create_dir_all(&data_dir) {
        log::warn!(
            "[meet-call] failed to create data dir {}: {}",
            data_dir.display(),
            err
        );
    }

    log::info!(
        "[meet-call] opening window label={label} request_id={request_id} url_host={} display_name_chars={}",
        parsed.host_str().unwrap_or(""),
        args.display_name.chars().count()
    );

    let title = format!("Meet — {}", truncate_for_title(&args.display_name));
    // Spawn the meet window **off-screen** so the user never sees it.
    //
    // Why off-screen and not `.visible(false)`: with CEF on macOS, a
    // window built hidden never gets a backing surface — the page
    // doesn't lay out or paint, which silently breaks the
    // `meet_scanner`'s automated join (the synthetic
    // `Input.dispatchMouseEvent` clicks land on un-rendered DOM).
    // Positioning off-screen keeps the window technically visible so
    // the renderer fully boots (WebRTC negotiates, getUserMedia fires,
    // CDP attaches, layout is real, clicks hit), but the user never
    // sees a meet window. The main OpenHuman UI is the only surface.
    //
    // The Y coordinate `-30000` is large enough to clear any sane
    // multi-monitor topology (macOS spaces, vertical stacks, etc.)
    // without overflowing i32 in the underlying Cocoa/Win32 APIs.
    let builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(parsed.clone()))
        .title(title)
        .inner_size(1100.0, 760.0)
        .resizable(true)
        .position(-30000.0, -30000.0)
        // Critical: do NOT take focus on creation. If this window
        // becomes the macOS key window, the main OpenHuman window is
        // demoted to "non-key" and Chromium throttles its renderer +
        // worker timers down to ~1Hz — which starves the
        // MascotFrameProducer to ~1fps and produces the visible
        // "stuck at one frame" symptom in Meet.
        .focused(false)
        .data_directory(data_dir.clone());

    let window = builder
        .build()
        .map_err(|e| format!("[meet-call] WebviewWindowBuilder.build failed: {e}"))?;

    // Install the in-process CDP transport for this Meet webview so the
    // audio bridge, video bridge, and join scanner can attach via the
    // shared channel rather than the legacy TCP DevTools port. The call
    // is idempotent; on a transient install failure (e.g. CEF observer
    // not yet ready) the downstream `conn_for_label` will retry.
    if let Err(err) = crate::cdp::install_for_label(&label) {
        log::warn!(
            "[meet-call] cdp install_for_label({label}) failed: {err} \
             — meet_audio/video/scanner will retry on first attach"
        );
    }

    // Push the window off-screen post-build. macOS Cocoa clamps NSWindow
    // frame origins to the union of all attached monitors' bounds, so
    // (-30000, -30000) lands at (0, 0) on a single-display setup or on
    // a secondary monitor's edge on multi-display setups. Not perfect,
    // but the post-join hide() in `meet_scanner::run` is the primary
    // hiding mechanism — this just keeps the brief pre-join window
    // out of the user's main display where possible.
    //
    // We can't hide() here: a window built hidden never gives its
    // renderer a backing surface, and `meet_scanner` drives the join
    // via CDP `Input.dispatchMouseEvent` which requires laid-out DOM.
    // Hide post-join instead.
    if let Err(err) = window.set_position(tauri::PhysicalPosition::new(-30000i32, -30000i32)) {
        log::warn!("[meet-call] post-build set_position failed: {err}");
    }
    if let Ok(pos) = window.outer_position() {
        log::info!(
            "[meet-call] post-build outer_position={{x:{},y:{}}} (target=-30000,-30000)",
            pos.x,
            pos.y
        );
    }

    state
        .inner
        .lock()
        .unwrap()
        .insert(request_id.clone(), label.clone());

    // Kick off the CDP-driven join automation: dismiss the device-check,
    // type the display name, and click "Ask to join". Store the returned
    // AbortHandle so we can cancel the task on close (see CloseRequested
    // handler below). Without cancellation the scanner's polling loops
    // keep CDP connections open and delay CEF renderer shutdown by up to
    // 60 s (issue #1378).
    let scanner_abort = meet_scanner::spawn(
        app.clone(),
        request_id.clone(),
        parsed.to_string(),
        args.display_name.clone(),
    );
    state
        .scanner_aborts
        .lock()
        .unwrap()
        .insert(request_id.clone(), scanner_abort);

    // Start the live meet-agent audio loop: registers a CEF audio
    // handler keyed by the meet URL, opens a core session, and spawns
    // the speak-pump poller. Fire-and-forget — failures here must not
    // prevent the user from at least seeing the join page, so we log
    // and continue. The teardown below mirrors this on window close.
    {
        let app_for_audio = app.clone();
        let request_id_for_audio = request_id.clone();
        let url_for_audio = parsed.to_string();
        let bot_for_audio = args.display_name.clone();
        let owner_for_audio = args.owner_display_name.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = crate::meet_audio::start(
                app_for_audio,
                request_id_for_audio.clone(),
                url_for_audio,
                owner_for_audio,
                bot_for_audio,
            )
            .await
            {
                log::warn!(
                    "[meet-call] meet_audio start failed request_id={request_id_for_audio} err={err}"
                );
            }
        });
    }

    // Register window lifecycle handlers.
    //
    // CloseRequested — fires for both programmatic window.close() calls
    // and OS title-bar close. We abort the scanner here so CEF does not
    // wait for in-flight CDP polling loops before completing renderer
    // shutdown. This is the primary fix for the 60-second navigation
    // block described in issue #1378.
    //
    // Destroyed — fires once the renderer is fully torn down. We emit
    // the frontend close event, stop the audio loop, and purge the
    // isolated CEF data directory.
    {
        let app_for_event = app.clone();
        let label_for_event = label.clone();
        let request_id_for_event = request_id.clone();
        let data_dir_for_event = data_dir.clone();
        window.on_window_event(move |event| {
            match event {
                tauri::WindowEvent::CloseRequested { .. } => {
                    // Abort the scanner task so its CDP connections are
                    // dropped before CEF starts tearing down the renderer.
                    // This unblocks the window destruction sequence and
                    // ensures `meet-call:closed` reaches the frontend
                    // promptly rather than after a 60-second stall.
                    //
                    // abort() is idempotent — safe to call if the scanner
                    // already finished naturally.
                    if let Some(state) = app_for_event.try_state::<MeetCallState>() {
                        if let Some(abort) = state
                            .scanner_aborts
                            .lock()
                            .unwrap()
                            .remove(&request_id_for_event)
                        {
                            abort.abort();
                            log::info!(
                                "[meet-call] scanner aborted on close request_id={request_id_for_event}"
                            );
                        }
                    }
                }

                tauri::WindowEvent::Destroyed => {
                    if let Some(state) = app_for_event.try_state::<MeetCallState>() {
                        state.inner.lock().unwrap().remove(&request_id_for_event);
                        // Defensive: if CloseRequested didn't fire (e.g. the
                        // window was destroyed by the OS without a prior close
                        // signal), abort the scanner here as a fallback.
                        if let Some(abort) = state
                            .scanner_aborts
                            .lock()
                            .unwrap()
                            .remove(&request_id_for_event)
                        {
                            abort.abort();
                            log::debug!(
                                "[meet-call] scanner aborted on destroy (fallback) request_id={request_id_for_event}"
                            );
                        }
                    }
                    if let Err(err) = app_for_event.emit(
                        "meet-call:closed",
                        serde_json::json!({
                            "request_id": request_id_for_event,
                            "label": label_for_event,
                        }),
                    ) {
                        log::debug!("[meet-call] emit closed failed: {err}");
                    }
                    log::info!(
                        "[meet-call] window destroyed label={label_for_event} request_id={request_id_for_event}"
                    );
                    // Tear down the meet-agent audio loop *before* the
                    // data dir wipe so the audio handler registration
                    // releases CEF cleanly while the browser is still
                    // shutting down.
                    {
                        let app_for_audio = app_for_event.clone();
                        let request_id_for_audio = request_id_for_event.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(err) =
                                crate::meet_audio::stop(app_for_audio, request_id_for_audio.clone())
                                    .await
                            {
                                log::debug!(
                                    "[meet-call] meet_audio stop err request_id={request_id_for_audio} err={err}"
                                );
                            }
                        });
                    }

                    // CEF may still be flushing the profile to disk on
                    // teardown; do the rmdir off the UI thread so any
                    // last-second writes don't race the delete.
                    let dir_to_purge = data_dir_for_event.clone();
                    let request_id_for_purge = request_id_for_event.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(err) = std::fs::remove_dir_all(&dir_to_purge) {
                            log::debug!(
                                "[meet-call] data-dir cleanup skipped request_id={request_id_for_purge} dir={} err={err}",
                                dir_to_purge.display()
                            );
                        }
                    });
                }

                _ => {}
            }
        });
    }

    Ok(label)
}

/// Close the Meet webview for the given `request_id`.
///
/// Aborts the scanner task before signalling `window.close()` so that
/// CEF does not wait for in-flight CDP polling to complete. This keeps
/// the window destruction fast regardless of which phase the scanner is
/// currently in (issue #1378).
#[tauri::command]
pub async fn meet_call_close_window<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, MeetCallState>,
    request_id: String,
) -> Result<bool, String> {
    let request_id = sanitize_request_id(&request_id)?;

    // Abort the scanner before closing so its CDP connections are
    // dropped immediately. The CloseRequested handler will also try to
    // abort, but doing it here first means the scanner is gone before
    // CEF even receives the close signal.
    if let Some(abort) = state.scanner_aborts.lock().unwrap().remove(&request_id) {
        abort.abort();
        log::info!("[meet-call] scanner aborted before window close request_id={request_id}");
    }

    let label = match state.inner.lock().unwrap().get(&request_id).cloned() {
        Some(label) => label,
        None => {
            log::debug!("[meet-call] close: no window for request_id={request_id}");
            return Ok(false);
        }
    };
    if let Some(window) = app.get_webview_window(&label) {
        log::info!("[meet-call] closing window label={label} request_id={request_id}");
        window
            .close()
            .map_err(|e| format!("[meet-call] window.close failed: {e}"))?;
        return Ok(true);
    }
    // Window was in state but not found in Tauri — clean up stale entry.
    state.inner.lock().unwrap().remove(&request_id);
    log::debug!("[meet-call] cleaned up stale entry for request_id={request_id}");
    Ok(false)
}

pub fn window_label_for(request_id: &str) -> String {
    format!("meet-call-{request_id}")
}

fn data_directory_for<R: Runtime>(app: &AppHandle<R>, request_id: &str) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("[meet-call] app_local_data_dir: {e}"))?;
    Ok(base.join("meet_call").join(request_id))
}

fn sanitize_request_id(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("[meet-call] request_id must not be empty".into());
    }
    if trimmed.len() > 64 {
        return Err("[meet-call] request_id exceeds 64 characters".into());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("[meet-call] request_id contains forbidden characters".into());
    }
    Ok(trimmed.to_string())
}

fn truncate_for_title(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.chars().count() <= 32 {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(32).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_request_id_rejects_path_traversal() {
        assert!(sanitize_request_id("..").is_err());
        assert!(sanitize_request_id("a/b").is_err());
        assert!(sanitize_request_id("a b").is_err());
        assert!(sanitize_request_id("").is_err());
    }

    #[test]
    fn sanitize_request_id_accepts_uuids_and_simple_ids() {
        sanitize_request_id("550e8400-e29b-41d4-a716-446655440000").unwrap();
        sanitize_request_id("abc_123").unwrap();
    }

    #[test]
    fn window_label_has_predictable_prefix() {
        let label = window_label_for("abc-123");
        assert!(label.starts_with("meet-call-"));
        assert!(label.contains("abc-123"));
    }

    #[test]
    fn truncate_for_title_caps_long_names() {
        let long = "a".repeat(40);
        let truncated = truncate_for_title(&long);
        assert!(truncated.chars().count() <= 33); // 32 + ellipsis
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn truncate_for_title_passes_short_names_through() {
        assert_eq!(truncate_for_title("Alice"), "Alice");
    }

    #[tokio::test]
    async fn meet_call_state_scanner_aborts_insert_and_remove() {
        // Verify the scanner_aborts map works as a round-trip store:
        // inserting then removing returns Some, and a second remove returns
        // None (abort is idempotent so the consume-once pattern is safe).
        let state = MeetCallState::new();

        // Spawn a pending task so we have a valid AbortHandle.
        let h = tokio::spawn(std::future::pending::<()>());
        let abort_handle = h.abort_handle();
        h.abort(); // Clean up the task immediately.

        state
            .scanner_aborts
            .lock()
            .unwrap()
            .insert("req-1".into(), abort_handle);

        assert!(
            state
                .scanner_aborts
                .lock()
                .unwrap()
                .remove("req-1")
                .is_some(),
            "first remove must return the stored handle"
        );
        assert!(
            state
                .scanner_aborts
                .lock()
                .unwrap()
                .remove("req-1")
                .is_none(),
            "second remove must return None — handle already consumed"
        );
    }

    #[test]
    fn meet_call_state_default_is_empty() {
        let state = MeetCallState::default();
        assert!(state.inner.lock().unwrap().is_empty());
        assert!(state.scanner_aborts.lock().unwrap().is_empty());
    }
}
