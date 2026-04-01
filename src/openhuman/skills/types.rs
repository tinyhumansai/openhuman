//! Core type definitions for the QuickJS skill runtime.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::openhuman::webhooks::WebhookResponseData;

/// Status of a running skill instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SkillStatus {
    /// Skill is registered but not yet started.
    #[default]
    Pending,
    /// Skill is currently initializing (loading JS, running init()).
    Initializing,
    /// Skill is running and ready to handle tool calls.
    Running,
    /// Skill is in the process of stopping.
    Stopping,
    /// Skill has been stopped gracefully.
    Stopped,
    /// Skill encountered a fatal error.
    Error,
}

/// Messages sent to a skill instance's message loop.
#[derive(Debug)]
pub enum SkillMessage {
    /// Call a tool exported by this skill.
    CallTool {
        tool_name: String,
        arguments: serde_json::Value,
        reply: tokio::sync::oneshot::Sender<Result<ToolResult, String>>,
    },
    /// Deliver a server event to the skill.
    ServerEvent {
        event: String,
        data: serde_json::Value,
    },
    /// Trigger a cron job by name.
    #[allow(dead_code)]
    CronTrigger { schedule_id: String },
    /// Request the skill to stop.
    Stop {
        reply: tokio::sync::oneshot::Sender<()>,
    },
    /// Start the setup flow — returns the first SetupStep.
    SetupStart {
        reply: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>,
    },
    /// Submit a setup step with user-provided values.
    SetupSubmit {
        step_id: String,
        values: serde_json::Value,
        reply: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>,
    },
    /// Cancel the setup flow.
    SetupCancel {
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// List the skill's runtime-configurable options.
    ListOptions {
        reply: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>,
    },
    /// Set a single option value.
    SetOption {
        name: String,
        value: serde_json::Value,
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// Notify skill of an AI session start.
    SessionStart {
        session_id: String,
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// Notify skill of an AI session end.
    SessionEnd {
        session_id: String,
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// Trigger periodic tick.
    Tick {
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// Notify the skill of an error from an async operation.
    #[allow(dead_code)]
    Error {
        error_type: String,
        message: String,
        source: Option<String>,
        recoverable: bool,
    },
    /// Generic JSON-RPC call (for methods not covered by specific variants).
    Rpc {
        method: String,
        params: serde_json::Value,
        reply: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>,
    },
    /// Load params from frontend (e.g. wallet address for wallet skill).
    /// Delivered after skill/load RPC; skill may export onLoad(params) to receive them.
    LoadParams { params: serde_json::Value },
    /// Deliver an incoming webhook request to the skill (targeted, not broadcast).
    /// The skill must respond via the oneshot reply with status/headers/body.
    WebhookRequest {
        correlation_id: String,
        method: String,
        path: String,
        headers: HashMap<String, serde_json::Value>,
        query: HashMap<String, String>,
        body: String,
        tunnel_id: String,
        tunnel_name: String,
        reply: tokio::sync::oneshot::Sender<Result<WebhookResponseData, String>>,
    },
}

/// Origin of a tool-call request entering the skill runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallOrigin {
    /// Calls initiated by trusted host surfaces (RPC/UI/socket MCP).
    External,
    /// Calls initiated from inside a running skill.
    SkillSelf { skill_id: String },
}

/// Result of executing a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Tool output content (array of content blocks per MCP spec).
    pub content: Vec<ToolContent>,
    /// Whether the tool execution resulted in an error.
    #[serde(default)]
    pub is_error: bool,
}

/// A single content block in a tool result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToolContent {
    Text { text: String },
    Json { data: serde_json::Value },
}

/// A tool definition exported by a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Unique tool name (within the skill).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    #[serde(default, rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// Snapshot of a skill's current state, suitable for sending to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSnapshot {
    pub skill_id: String,
    pub name: String,
    pub status: SkillStatus,
    pub tools: Vec<ToolDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Arbitrary state the skill has published for the frontend.
    #[serde(default)]
    pub state: HashMap<String, serde_json::Value>,
    /// Whether the skill's setup/OAuth flow has been completed (persisted).
    #[serde(default)]
    pub setup_complete: bool,
    /// Derived connection status for the frontend UI.
    #[serde(default)]
    pub connection_status: String,
}

/// Derive a unified connection status string from skill state.
/// Mirrors the logic in `app/src/lib/skills/hooks.ts:deriveConnectionStatus`.
pub fn derive_connection_status(
    status: SkillStatus,
    setup_complete: bool,
    published_state: &HashMap<String, serde_json::Value>,
) -> String {
    match status {
        SkillStatus::Pending | SkillStatus::Stopped | SkillStatus::Stopping => {
            return "offline".to_string()
        }
        SkillStatus::Error => return "error".to_string(),
        SkillStatus::Initializing => return "connecting".to_string(),
        _ => {}
    }

    let conn = published_state
        .get("connection_status")
        .and_then(|v| v.as_str());
    let auth = published_state.get("auth_status").and_then(|v| v.as_str());

    // No self-reported state — derive from lifecycle + setup
    if conn.is_none() && auth.is_none() {
        if setup_complete && matches!(status, SkillStatus::Running) {
            return "connected".to_string();
        }
        if !setup_complete {
            return "setup_required".to_string();
        }
        return "connecting".to_string();
    }

    if conn == Some("error") || auth == Some("error") {
        return "error".to_string();
    }
    if conn == Some("connecting") || auth == Some("authenticating") {
        return "connecting".to_string();
    }
    if conn == Some("connected") {
        if auth.is_none() || auth == Some("authenticated") {
            return "connected".to_string();
        }
        if auth == Some("not_authenticated") {
            return "not_authenticated".to_string();
        }
    }
    if conn == Some("disconnected") {
        if setup_complete {
            return "disconnected".to_string();
        }
        return "setup_required".to_string();
    }

    "connecting".to_string()
}

/// Configuration for a skill instance, derived from its manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillConfig {
    pub skill_id: String,
    pub name: String,
    pub entry_point: String,
    /// Memory limit in bytes. Default: 256 MB.
    /// Skills can override this in their manifest.json.
    #[serde(default = "default_memory_limit")]
    pub memory_limit: usize,
    /// Whether this skill should auto-start on app launch.
    #[serde(default)]
    pub auto_start: bool,
}

fn default_memory_limit() -> usize {
    256 * 1024 * 1024 // 256 MB
}

/// Events emitted from the runtime to the frontend via Tauri.
#[allow(dead_code)]
pub mod events {
    pub const SKILL_STATUS_CHANGED: &str = "runtime:skill-status-changed";
    pub const SKILL_STATE_CHANGED: &str = "runtime:skill-state-changed";
    pub const SKILL_TOOLS_CHANGED: &str = "runtime:skill-tools-changed";
    pub const SKILL_LOG: &str = "runtime:skill-log";
}
