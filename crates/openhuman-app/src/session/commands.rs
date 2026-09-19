//! Tauri commands the renderer uses to drive login, logout and the current
//! user. Each is a one-line delegate to the managed `SessionHost`; the
//! `Err(String)` carries `openhuman_tinyhumans::SessionError`'s stable
//! `PREFIX:` so the frontend can classify without parsing prose.

use openhuman_tinyhumans::{CachedUser, SessionState};

use super::SessionHost;

fn err(error: openhuman_tinyhumans::SessionError) -> String {
    error.to_string()
}

/// Exchange a one-time login token for a session JWT, validate it against
/// the backend, and install it in the core.
#[tauri::command]
pub(crate) async fn auth_login_with_token(
    host: tauri::State<'_, SessionHost>,
    token: String,
) -> Result<SessionState, String> {
    log::info!("[session][cmd] auth_login_with_token");
    host.manager.login_with_token(&token).await.map_err(err)
}

/// Install a session JWT (or the offline local token) the renderer already
/// holds. A JWT is validated against the backend first.
#[tauri::command]
pub(crate) async fn auth_store_session(
    host: tauri::State<'_, SessionHost>,
    token: String,
    user: Option<serde_json::Value>,
) -> Result<SessionState, String> {
    log::info!(
        "[session][cmd] auth_store_session has_user={}",
        user.is_some()
    );
    host.manager
        .store_session_token(&token, user)
        .await
        .map_err(err)
}

/// Sign out: clear the session credential in the core.
#[tauri::command]
pub(crate) async fn auth_logout(
    host: tauri::State<'_, SessionHost>,
) -> Result<SessionState, String> {
    log::info!("[session][cmd] auth_logout");
    host.manager.logout().await.map_err(err)
}

/// The core's credential state plus the cached current user.
#[tauri::command]
pub(crate) async fn auth_state(
    host: tauri::State<'_, SessionHost>,
) -> Result<SessionState, String> {
    host.manager.state().await.map_err(err)
}

/// The current user from `/auth/me`, served from the cache unless `force`.
#[tauri::command]
pub(crate) async fn auth_current_user(
    host: tauri::State<'_, SessionHost>,
    force: Option<bool>,
) -> Result<CachedUser, String> {
    host.manager
        .current_user(force.unwrap_or(false))
        .await
        .map_err(err)
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
