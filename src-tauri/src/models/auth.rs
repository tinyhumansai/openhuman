use serde::{Deserialize, Serialize};

/// User session information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// JWT session token
    pub token: String,
    /// User ID
    pub user_id: String,
    /// When the session was created (Unix timestamp)
    pub created_at: u64,
    /// When the session expires (Unix timestamp)
    pub expires_at: Option<u64>,
}

/// User profile information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    #[serde(rename = "firstName")]
    pub first_name: Option<String>,
    #[serde(rename = "lastName")]
    pub last_name: Option<String>,
    pub username: Option<String>,
    pub email: Option<String>,
    #[serde(rename = "telegramId")]
    pub telegram_id: Option<String>,
}

/// Token exchange request payload
#[derive(Debug, Serialize)]
pub struct TokenExchangeRequest {
    pub token: String,
}

/// Token exchange response from backend
#[derive(Debug, Deserialize)]
pub struct TokenExchangeResponse {
    #[serde(rename = "sessionToken")]
    pub session_token: String,
    pub user: User,
}

/// Auth error response from backend
#[derive(Debug, Deserialize)]
pub struct AuthErrorResponse {
    pub success: bool,
    pub error: Option<String>,
}

/// Auth state that can be emitted to frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthState {
    pub is_authenticated: bool,
    pub user: Option<User>,
}
