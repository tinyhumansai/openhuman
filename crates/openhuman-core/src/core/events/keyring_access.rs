//! Payload for one exact keyring operation's human approval request.

/// Host-verified, display-safe metadata; never includes a secret value.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KeyringAccessApprovalRequest {
    pub request_id: String,
    pub caller_agent: String,
    pub origin: String,
    pub purpose: String,
    pub access: String,
    pub name: String,
    pub expires_at_ms: u64,
}
