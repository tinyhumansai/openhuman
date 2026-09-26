//! Serde shapes of the backend-brokered OAuth payloads (unchanged wire
//! contracts of the `auth_oauth_*` RPCs).

use serde::{Deserialize, Serialize};

/// A summary of an active integration, as returned by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationSummary {
    /// Unique identifier for the integration.
    pub id: String,
    /// The name of the integration provider (e.g., "google", "slack").
    pub provider: String,
    /// RFC3339 timestamp of when the integration was created.
    pub created_at: String,
}

/// Decrypted OAuth token payload for handing off tokens to a local service or skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationTokensHandoff {
    /// The OAuth access token.
    pub access_token: String,
    /// The optional OAuth refresh token.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// RFC3339 timestamp of when the access token expires.
    pub expires_at: String,
}
