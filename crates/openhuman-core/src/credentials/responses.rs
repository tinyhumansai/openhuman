//! Response DTOs shared by auth RPC and `core_server` (re-exported from [`crate::core_server::types`]).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStateResponse {
    pub is_authenticated: bool,
    pub user_id: Option<String>,
    pub user: Option<serde_json::Value>,
    pub profile_id: Option<String>,
    /// Which credential backs `is_authenticated`: `"session"` for an app
    /// session JWT, `"api-key"` for a TinyHumans API key (library runtimes,
    /// no user identity), `"local"` for the offline local session, absent
    /// when signed out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    /// RFC3339 expiry recorded from the session JWT's `exp` at store time.
    /// Absent for API keys, local sessions and `exp`-less tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Backend that issued this session. Legacy sessions have no association.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuing_backend: Option<String>,
}

/// `AuthStateResponse::credential` value for an app-session JWT.
pub const CREDENTIAL_SESSION: &str = "session";
/// `AuthStateResponse::credential` value for a TinyHumans API key.
pub const CREDENTIAL_API_KEY: &str = "api-key";
/// `AuthStateResponse::credential` value for the offline local session.
pub const CREDENTIAL_LOCAL: &str = "local";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProfileSummary {
    pub id: String,
    pub provider: String,
    pub profile_name: String,
    pub kind: String,
    pub account_id: Option<String>,
    pub workspace_id: Option<String>,
    pub metadata_keys: Vec<String>,
    pub updated_at: String,
    pub has_token: bool,
    pub has_token_set: bool,
}
