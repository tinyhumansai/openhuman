use crate::models::auth::{AuthState, User};
use crate::services::session_service::SessionService;
use crate::services::socket_service::SOCKET_SERVICE;
use once_cell::sync::Lazy;
use std::sync::Arc;

// Global session service instance
pub static SESSION_SERVICE: Lazy<Arc<SessionService>> =
    Lazy::new(|| Arc::new(SessionService::new()));


/// Get the current authentication state
#[tauri::command]
pub fn get_auth_state() -> AuthState {
    AuthState {
        is_authenticated: SESSION_SERVICE.is_authenticated(),
        user: SESSION_SERVICE.get_user(),
    }
}

/// Get the current session token
#[tauri::command]
pub fn get_session_token() -> Option<String> {
    SESSION_SERVICE.get_token()
}

/// Get the current user
#[tauri::command]
pub fn get_current_user() -> Option<User> {
    SESSION_SERVICE.get_user()
}

/// Check if the user is authenticated
#[tauri::command]
pub fn is_authenticated() -> bool {
    SESSION_SERVICE.is_authenticated()
}

/// Logout and clear session
#[tauri::command]
pub fn logout() -> Result<(), String> {
    // Request socket to disconnect
    let _ = SOCKET_SERVICE.request_disconnect();

    // Clear session
    SESSION_SERVICE.clear_session()?;

    // Clear socket credentials
    SOCKET_SERVICE.clear_credentials();

    Ok(())
}

/// Store session manually (used by frontend if needed)
#[tauri::command]
pub fn store_session(token: String, user: User) -> Result<(), String> {
    SESSION_SERVICE.store_session(&token, &user)
}
