//! One-shot orchestration run types (/orchestration/v1).
//!
//! Split from the parent types module. Field names use serde renames to match
//! the backend camelCase wire format exactly, and unknown fields are tolerated
//! so the client keeps working against newer server versions.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A client-side tool definition offered to a run.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON-Schema object describing the tool parameters.
    pub parameters: Value,
}

/// A tool call requested by the orchestrator.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

/// A tool result fed back via `run/continue`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolResult {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Config knobs for a run (`options.config`).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_passes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
}

/// Resource limits for a run (`options.limits`).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLimits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tasks_per_delegate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
}

/// One workspace's authored `MEDULLA.md`, sent verbatim on a run request. The
/// medulla SDK owns the format, so the text is forwarded unparsed and the
/// backend distils it into the orchestrator's context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceProfileInput {
    /// The workspace/repo path this profile describes.
    pub workspace: String,
    /// Verbatim `MEDULLA.md` contents.
    pub medulla_md: String,
}

/// The `options` object of a run request.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOrchestrationOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_overrides: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<RunConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<RunLimits>,
    /// Authored workspace profiles for the directories this cycle works over.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_profiles: Option<Vec<WorkspaceProfileInput>>,
}

/// Optional inputs to [`crate::openhuman::medulla::client::MedullaClient::run`].
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<RunOrchestrationOptions>,
}

/// Final reply from a tool-less run.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReply {
    pub reply: String,
    #[serde(default)]
    pub pass_count: Option<u32>,
    #[serde(default)]
    pub compressed_history: Vec<Value>,
    #[serde(default)]
    pub escalations: Vec<Value>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub cycle_id: Option<String>,
}

/// A single step of the client tool-loop.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "stop", rename_all = "snake_case")]
pub enum LoopEvent {
    /// The orchestrator wants the client to run tools and continue.
    ToolUse {
        #[serde(rename = "cycleId")]
        cycle_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "toolCalls", default)]
        tool_calls: Vec<ToolCall>,
    },
    /// The run finished with a final reply.
    End {
        #[serde(rename = "cycleId")]
        cycle_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        reply: String,
        #[serde(rename = "passCount", default)]
        pass_count: Option<u32>,
        #[serde(rename = "compressedHistory", default)]
        compressed_history: Vec<Value>,
        #[serde(default)]
        escalations: Vec<Value>,
    },
    /// Long-poll returned without progress; poll `run/continue` again.
    Pending {
        #[serde(rename = "cycleId")]
        cycle_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    /// The run errored.
    Error {
        #[serde(rename = "cycleId")]
        cycle_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        error: Value,
    },
}

/// Outcome of [`crate::openhuman::medulla::client::MedullaClient::run`]: either a final reply (tool-less)
/// or a tool-loop event (when tools were supplied).
#[derive(Debug, Clone)]
pub enum RunResult {
    Reply(RunReply),
    Loop(LoopEvent),
}
