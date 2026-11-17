//! The desktop session owner: login-token exchange, `/auth/me` and the
//! current-user cache live in `openhuman-tinyhumans`; this module is the glue
//! that gives it a link to the embedded (or gateway) core, exposes it to the
//! renderer as Tauri commands, and forwards its change events as Tauri events.
//!
//! The core only *holds* the credential (`auth.set_credential`). Everything
//! that talks to the backend's auth endpoints happens here, in the shell
//! process, with the same `x-sdk-name` / version attribution headers the core
//! sends on its own backend traffic.

pub(crate) mod commands;
mod link;

use std::sync::Arc;

use openhuman_rpc::tinyhumans::{ClientHeaders, SessionEvent, SessionManager};
use tauri::{AppHandle, Emitter, Manager};

use crate::core_process::CoreProcessHandle;
use crate::AppRuntime;

pub(crate) use link::HttpCoreLink;

/// Tauri event emitted whenever the credential or the current user changes.
/// Payload: `openhuman_rpc::tinyhumans::SessionState`.
pub const AUTH_CHANGED_EVENT: &str = "auth://changed";
/// Tauri event emitted when the backend rejected the stored credential and it
/// has been cleared. Payload: `{ source }`.
pub const AUTH_EXPIRED_EVENT: &str = "auth://expired";

/// Managed Tauri state: the one session manager for this shell process.
pub struct SessionHost {
    pub manager: Arc<SessionManager<HttpCoreLink>>,
}

impl SessionHost {
    pub fn new(desktop: CoreProcessHandle) -> Self {
        // The shell and the core ship as one release, so one version answers
        // for both `x-core-version` and `x-tauri-version`.
        let headers = ClientHeaders::new(openhuman_rpc::tinyhumans::product_identity().as_str())
            .with_core_version(env!("CARGO_PKG_VERSION"))
            .with_tauri_version(env!("CARGO_PKG_VERSION"));
        let link = Arc::new(HttpCoreLink::new(desktop));
        Self {
            manager: SessionManager::new(link, headers),
        }
    }
}

/// Register the session host on the app and start forwarding its events to
/// the renderer.
pub fn install(app: &AppHandle<AppRuntime>, desktop: CoreProcessHandle) {
    let host = SessionHost::new(desktop);
    let mut events = host.manager.subscribe();
    app.manage(host);

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(SessionEvent::Changed(state)) => {
                    log::debug!(
                        "[session] auth changed authenticated={} credential={}",
                        state.core.is_authenticated,
                        state.core.credential.as_deref().unwrap_or("none")
                    );
                    if let Err(e) = handle.emit(AUTH_CHANGED_EVENT, state) {
                        log::debug!("[session] failed to emit {AUTH_CHANGED_EVENT}: {e}");
                    }
                }
                Ok(SessionEvent::Expired { source }) => {
                    log::warn!(
                        "[session] backend rejected the stored credential (source={source})"
                    );
                    if let Err(e) =
                        handle.emit(AUTH_EXPIRED_EVENT, serde_json::json!({ "source": source }))
                    {
                        log::debug!("[session] failed to emit {AUTH_EXPIRED_EVENT}: {e}");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    log::debug!("[session] event forwarder lagged by {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// The signed-in user id, for synchronous callers (Sentry `before_send`).
pub fn peek_user_id() -> Option<String> {
    openhuman_rpc::tinyhumans::identity::peek_user_id()
}
