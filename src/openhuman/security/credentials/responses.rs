//! Response DTOs shared by auth RPC and `core_server` (re-exported from [`crate::core_server::types`]).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStateResponse {
    pub is_authenticated: bool,
    pub user_id: Option<String>,
    pub user: Option<serde_json::Value>,
    pub profile_id: Option<String>,
}

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
