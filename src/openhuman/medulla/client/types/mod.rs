//! JSON types mirroring the backend API responses.
//!
//! Field names use `serde` renames to match the backend's camelCase wire
//! format exactly. Unknown fields are tolerated so the client keeps working
//! against newer server versions.

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod event;
mod reward;
mod run;
mod session;

pub use event::{EventEnvelope, EventKind};
pub use reward::{
    HistoryRewardBreakdown, HistoryRewardClaim, HistoryRewardStatus, HistoryUploadResult,
};
pub use run::{
    LoopEvent, RunConfig, RunLimits, RunOptions, RunOrchestrationOptions, RunReply, RunResult,
    ToolCall, ToolDef, ToolResult, WorkspaceProfileInput,
};
pub use session::{
    AbortResult, Message, Role, SendResult, SessionArchived, SessionCreated, SessionDetail,
    SessionStatus, SessionSummary,
};

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// Audience hint accepted by the login-token consume endpoint.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Audience {
    App,
    Dashboard,
}

// ---------------------------------------------------------------------------
// Sessions (/medulla/v1)
// ---------------------------------------------------------------------------

/// Client for the Medulla backend HTTP + SSE API.
#[derive(Debug, Clone)]
pub struct MedullaClient {
    pub(super) base_url: String,
    pub(super) jwt: String,
    pub(super) http: reqwest::Client,
}
/// Builder for [`MedullaClient`].
#[derive(Debug, Default)]
pub struct MedullaClientBuilder {
    pub(super) base_url: Option<String>,
    pub(super) jwt: Option<String>,
    pub(super) http: Option<reqwest::Client>,
}
/// Raw response envelope shared by every endpoint.
#[derive(Debug, Deserialize)]
pub(super) struct RawEnvelope {
    #[serde(default)]
    pub(super) success: bool,
    #[serde(default)]
    pub(super) data: Option<Value>,
    #[serde(default)]
    pub(super) error: Option<String>,
    #[serde(rename = "errorCode", default)]
    pub(super) error_code: Option<String>,
    #[serde(default)]
    pub(super) details: Option<Value>,
}

/// Request body for creating a durable session.
///
/// `workspaceProfiles` carries the authored `MEDULLA.md` for each active
/// workspace root; the backend session-mint accepts and distils them. Omitted
/// entirely when no workspace has a profile, so a plain session mint is unchanged.
#[derive(serde::Serialize)]
pub(super) struct CreateSessionBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) title: Option<&'a str>,
    #[serde(
        rename = "workspaceProfiles",
        skip_serializing_if = "<[WorkspaceProfileInput]>::is_empty"
    )]
    pub(super) workspace_profiles: &'a [WorkspaceProfileInput],
}

/// Request body for adding a message to a session.
#[derive(serde::Serialize)]
pub(super) struct SendMessageBody<'a> {
    pub(super) body: &'a str,
}

/// Request body for starting an orchestration run.
#[derive(serde::Serialize)]
pub(super) struct RunBody<'a> {
    pub(super) input: &'a str,
    #[serde(flatten)]
    pub(super) options: &'a RunOptions,
}

/// Request body for continuing an orchestration run.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContinueRunBody<'a> {
    pub(super) cycle_id: &'a str,
    pub(super) tool_results: Vec<ToolResult>,
}
