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

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Deserialize;
use tauri::{webview::WebviewWindowBuilder, AppHandle, Emitter, Manager, Runtime, WebviewUrl};
use url::Url;

use crate::meet_scanner;

/// Per-process registry of open Meet webview windows, keyed by
/// `request_id` so the frontend can ask us to close a specific call.
#[derive(Default)]
pub struct MeetCallState {
    inner: Mutex<std::collections::HashMap<String, String>>, // request_id -> window label
}

impl MeetCallState {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Deserialize)]
pub struct OpenWindowArgs {
    pub request_id: String,
    pub meet_url: String,
    pub display_name: String,
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
    let builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(parsed.clone()))
        .title(title)
        .inner_size(1100.0, 760.0)
        .resizable(true)
        .data_directory(data_dir.clone());

    let window = builder
        .build()
        .map_err(|e| format!("[meet-call] WebviewWindowBuilder.build failed: {e}"))?;

    state
        .inner
        .lock()
        .unwrap()
        .insert(request_id.clone(), label.clone());

    // Kick off the CDP-driven join automation: dismiss the device-check,
    // type the display name, and click "Ask to join". Fire-and-forget —
    // the user can finish manually if any step times out. Pass the
    // normalised URL so the scanner can attach to the right CEF target
    // when more than one Meet window is open.
    meet_scanner::spawn(
        app.clone(),
        request_id.clone(),
        parsed.to_string(),
        args.display_name.clone(),
    );

    // Emit a `closed` event when the user dismisses the window AND clean
    // up the per-call data directory. The data dir holds an isolated CEF
    // profile (cookies, cache) we explicitly want to throw away after
    // each call so the next anonymous join doesn't reuse stale state and
    // disk doesn't grow unboundedly across many calls.
    {
        let app_for_event = app.clone();
        let label_for_event = label.clone();
        let request_id_for_event = request_id.clone();
        let data_dir_for_event = data_dir.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(state) = app_for_event.try_state::<MeetCallState>() {
                    state.inner.lock().unwrap().remove(&request_id_for_event);
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
        });
    }

    Ok(label)
}

#[tauri::command]
pub async fn meet_call_close_window<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, MeetCallState>,
    request_id: String,
) -> Result<bool, String> {
    let request_id = sanitize_request_id(&request_id)?;
    let label = match state.inner.lock().unwrap().get(&request_id).cloned() {
        Some(label) => label,
        None => return Ok(false),
    };
    if let Some(window) = app.get_webview_window(&label) {
        window
            .close()
            .map_err(|e| format!("[meet-call] window.close failed: {e}"))?;
        return Ok(true);
    }
    Ok(false)
}

fn window_label_for(request_id: &str) -> String {
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
}
