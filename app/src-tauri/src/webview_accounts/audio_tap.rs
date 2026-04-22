//! Account-to-browser ID mapping for the audio capture pipeline.
//!
//! CEF assigns an integer browser ID only after the webview is created.
//! `webview_account_open` calls [`map_account_to_browser`] once the browser ID
//! is available, so downstream callers (e.g. `CallSessionManager`) can resolve
//! an account ID to a CEF browser ID via [`get_browser_for_account`].
//!
//! The actual audio tap state (ring buffer, PCM push, subscribe) lives in the
//! CEF runtime crate at `tauri_runtime_cef::audio_tap_registry` — this module
//! only handles the account ↔ browser ID mapping which is a Tauri-shell concern.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const LOG_PREFIX: &str = "[audio-tap]";

/// account_id → browser_id
static ACCOUNT_TO_BROWSER: OnceLock<Mutex<HashMap<String, i32>>> = OnceLock::new();

fn account_map() -> &'static Mutex<HashMap<String, i32>> {
    ACCOUNT_TO_BROWSER.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record the mapping from an account id (e.g. `"acct-abc123"`) to a CEF
/// browser id.  Called from `webview_account_open`'s `with_webview` callback
/// once the browser id is available.
pub fn map_account_to_browser(account_id: &str, browser_id: i32) {
    log::debug!("{LOG_PREFIX} mapping account_id={account_id} → browser_id={browser_id}");
    account_map()
        .lock()
        .unwrap()
        .insert(account_id.to_string(), browser_id);
}

/// Look up the CEF browser id for an account id.  Returns `None` if the
/// account hasn't been opened yet or its browser id is not yet known.
pub fn get_browser_for_account(account_id: &str) -> Option<i32> {
    account_map().lock().unwrap().get(account_id).copied()
}

/// Remove the account → browser mapping when the webview is closed or purged.
pub fn remove_account_mapping(account_id: &str) {
    let removed = account_map().lock().unwrap().remove(account_id);
    if let Some(bid) = removed {
        log::debug!("{LOG_PREFIX} removed mapping account_id={account_id} browser_id={bid}");
    }
}
