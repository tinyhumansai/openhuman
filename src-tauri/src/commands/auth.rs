use crate::models::auth::{AuthState, TokenExchangeResponse, User};
use crate::services::session_service::SessionService;
use crate::services::socket_service::SOCKET_SERVICE;
use once_cell::sync::Lazy;
use std::sync::Arc;

// Global session service instance
pub static SESSION_SERVICE: Lazy<Arc<SessionService>> =
    Lazy::new(|| Arc::new(SessionService::new()));

/// Exchange a login token for a session token
/// This is called after the user authenticates via deep link
#[tauri::command]
pub async fn exchange_token(
    backend_url: String,
    token: String,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/auth/desktop-exchange", backend_url);

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("ngrok-skip-browser-warning", "true")
        .json(&serde_json::json!({ "token": token }))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = response.status().as_u16();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if status != 200 {
        let error = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("Unknown error");
        return Err(format!("Exchange failed ({}): {}", status, error));
    }

    // Try to parse and store session
    if let Ok(exchange_response) = serde_json::from_value::<TokenExchangeResponse>(body.clone()) {
        // Store session securely
        let _ = SESSION_SERVICE.store_session(&exchange_response.session_token, &exchange_response.user);
    }

    Ok(body)
}

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
