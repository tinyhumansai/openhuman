use serde::{Deserialize, Serialize};
use serde_json::Value;

// --- ActionRequest lifecycle (AOS-S1.M1.2.3) ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionRequestLifecycleEnvelope {
    pub action_request: Value,
    pub row_version: i64,
    pub id: String,
    pub tenant_id: String,
    pub approval_state: String,
    pub execution_state: String,
    pub policy_outcome: String,
    pub correlation_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionRequestListResponse {
    pub items: Vec<ActionRequestLifecycleEnvelope>,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ListActionRequestsRpcParams {
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub approval_state: Option<String>,
    #[serde(default)]
    pub execution_state: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GetActionRequestRpcParams {
    pub action_request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActionRequestDecisionRpcParams {
    pub action_request_id: String,
    pub reason: String,
    pub expected_row_version: i64,
    /// Required stable per-intent key. Blank/missing is rejected so retries cannot silently mint a new UUID.
    pub idempotency_key: String,
}
