//! Core type definitions for the QuickJS skill runtime.
//!
//! This module defines the essential data structures used throughout the skills system,
//! including lifecycle statuses, message types for internal communication,
//! tool definitions, and state snapshots.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::openhuman::webhooks::WebhookResponseData;

/// Status of a running skill instance.
///
/// Represents the current phase of a skill's lifecycle, from registration
/// to active execution or error states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SkillStatus {
    /// Skill is registered but its execution has not yet started.
    #[default]
    Pending,
    /// Skill is currently initializing (e.g., loading JS, running `onInit()`).
    Initializing,
    /// Skill is actively running and ready to handle tool calls or events.
    Running,
    /// Skill is in the process of shutting down gracefully.
    Stopping,
    /// Skill has been stopped and its execution loop has terminated.
    Stopped,
    /// Skill encountered a fatal error and cannot continue execution.
    Error,
}

/// Messages sent to a skill instance's message loop for processing.
///
/// This enum covers all possible interactions between the runtime/host
/// and the skill's JavaScript environment.
#[derive(Debug)]
pub enum SkillMessage {
    /// Request the skill to execute one of its exported tools.
    CallTool {
        /// Name of the tool to call.
        tool_name: String,
        /// JSON arguments to pass to the tool.
        arguments: serde_json::Value,
        /// Channel to send the tool execution result back to the caller.
        reply: tokio::sync::oneshot::Sender<Result<ToolResult, String>>,
    },
    /// Deliver a system-wide or custom server event to the skill.
    ServerEvent {
        /// Name of the event.
        event: String,
        /// JSON data associated with the event.
        data: serde_json::Value,
    },
    /// Trigger a pre-registered cron job within the skill.
    CronTrigger {
        /// Identifier of the schedule that triggered this message.
        schedule_id: String,
    },
    /// Request the skill to stop its execution gracefully.
    Stop {
        /// Channel to acknowledge when the skill has finished stopping.
        reply: tokio::sync::oneshot::Sender<()>,
    },
    /// Signal the start of the skill's setup/configuration flow.
    SetupStart {
        /// Channel to return the first setup step definition.
        reply: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>,
    },
    /// Submit user input for a specific setup step.
    SetupSubmit {
        /// Identifier of the setup step being submitted.
        step_id: String,
        /// JSON values provided by the user.
        values: serde_json::Value,
        /// Channel to return the next setup step or a completion signal.
        reply: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>,
    },
    /// Abort the current setup flow.
    SetupCancel {
        /// Channel to acknowledge the cancellation.
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// Request a list of all runtime-configurable options supported by the skill.
    ListOptions {
        /// Channel to return the list of options.
        reply: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>,
    },
    /// Update the value of a specific skill option.
    SetOption {
        /// Name of the option to set.
        name: String,
        /// New JSON value for the option.
        value: serde_json::Value,
        /// Channel to acknowledge the update.
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// Notify the skill that a new AI interaction session has started.
    SessionStart {
        /// Unique identifier for the session.
        session_id: String,
        /// Channel to acknowledge the notification.
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// Notify the skill that an AI interaction session has ended.
    SessionEnd {
        /// Unique identifier for the session.
        session_id: String,
        /// Channel to acknowledge the notification.
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// Trigger a periodic "tick" event for skills that need regular maintenance.
    Tick {
        /// Channel to acknowledge processing of the tick.
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// Deliver an error notification from an asynchronous background operation.
    Error {
        /// Category of the error.
        error_type: String,
        /// human-readable error message.
        message: String,
        /// Optional source identifier where the error originated.
        source: Option<String>,
        /// Whether the skill can attempt to recover from this error.
        recoverable: bool,
    },
    /// Route a generic JSON-RPC call to the skill's custom RPC handler.
    Rpc {
        /// Method name.
        method: String,
        /// JSON parameters for the method.
        params: serde_json::Value,
        /// Channel to return the RPC result.
        reply: tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>,
    },
    /// Pass parameters loaded from the frontend to the skill.
    ///
    /// Typically used to provide session-specific data like wallet addresses.
    LoadParams {
        /// JSON parameters to load.
        params: serde_json::Value,
    },
    /// Deliver a targeted webhook request to the skill.
    WebhookRequest {
        /// Unique ID for tracking this specific request/response cycle.
        correlation_id: String,
        /// HTTP method (GET, POST, etc.).
        method: String,
        /// Request path relative to the skill's endpoint.
        path: String,
        /// Map of HTTP headers.
        headers: HashMap<String, serde_json::Value>,
        /// Map of query string parameters.
        query: HashMap<String, String>,
        /// Raw request body as a string.
        body: String,
        /// Identifier of the tunnel through which the request arrived.
        tunnel_id: String,
        /// Human-readable name of the tunnel.
        tunnel_name: String,
        /// Channel to return the skill's HTTP response.
        reply: tokio::sync::oneshot::Sender<Result<WebhookResponseData, String>>,
    },
}

/// Defines the origin of a tool-call request for security policy enforcement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallOrigin {
    /// Request initiated from outside the skill runtime (e.g., UI, backend, or CLI).
    External,
    /// Request initiated from within a skill's own execution environment.
    SkillSelf {
        /// ID of the skill making the call.
        skill_id: String,
    },
}

/// Result of executing a tool, containing content blocks and error status.
///
/// This is the **unified** tool result type used by both built-in tools and
/// QuickJS skill tools. Follows the MCP (Model Context Protocol) specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// List of content blocks returned by the tool (follows MCP specification).
    pub content: Vec<ToolContent>,
    /// Indicates if the tool encountered an error during execution.
    #[serde(default)]
    pub is_error: bool,
}

impl ToolResult {
    /// Create a successful result with a single text block.
    pub fn success(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text { text: text.into() }],
            is_error: false,
        }
    }

    /// Create an error result with a single text block.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text {
                text: message.into(),
            }],
            is_error: true,
        }
    }

    /// Create a successful result with structured JSON data.
    pub fn json(data: serde_json::Value) -> Self {
        Self {
            content: vec![ToolContent::Json { data }],
            is_error: false,
        }
    }

    /// Extract all text content as a single joined string.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| match c {
                ToolContent::Text { text } => Some(text.as_str()),
                ToolContent::Json { data } => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Extract all content (text + JSON) as a single string.
    pub fn output(&self) -> String {
        self.content
            .iter()
            .map(|c| match c {
                ToolContent::Text { text } => text.clone(),
                ToolContent::Json { data } => {
                    serde_json::to_string_pretty(data).unwrap_or_default()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A single content block within a `ToolResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToolContent {
    /// plain-text output.
    Text { text: String },
    /// Structured JSON data output.
    Json { data: serde_json::Value },
}

/// Metadata defining a tool exported by a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Unique name of the tool within the scope of the skill.
    pub name: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// JSON Schema defining the expected input parameters for the tool.
    #[serde(default, rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// A comprehensive snapshot of a skill's current state.
///
/// This struct is serialized and sent to the frontend to update the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSnapshot {
    /// Unique identifier for the skill.
    pub skill_id: String,
    /// Human-readable display name.
    pub name: String,
    /// Current lifecycle status (e.g., Running, Error).
    pub status: SkillStatus,
    /// List of tools currently exported by the skill.
    pub tools: Vec<ToolDefinition>,
    /// Optional error message if the skill is in an `Error` state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Arbitrary key-value state published by the skill for UI consumption.
    #[serde(default)]
    pub state: HashMap<String, serde_json::Value>,
    /// Whether the skill has completed its initial setup/OAuth flow.
    #[serde(default)]
    pub setup_complete: bool,
    /// High-level connection health string derived from status and published state.
    #[serde(default)]
    pub connection_status: String,
}

/// Derive a unified connection status string for UI display.
///
/// This function translates internal statuses and skill-reported health
/// (e.g., auth status) into a set of standard strings consumed by the frontend.
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
    /// Unique identifier for the skill.
    pub skill_id: String,
    /// human-readable display name.
    pub name: String,
    /// Filename of the JavaScript entry point.
    pub entry_point: String,
    /// Maximum memory allowed for the JS runtime in bytes.
    #[serde(default = "default_memory_limit")]
    pub memory_limit: usize,
    /// Whether the skill should be started automatically.
    #[serde(default)]
    pub auto_start: bool,
}

fn default_memory_limit() -> usize {
    256 * 1024 * 1024 // 256 MB
}

/// Constants for events emitted from the runtime to the frontend via Tauri.
pub mod events {
    /// Emitted when a skill's internal state or status changes.
    pub const SKILL_STATE_CHANGED: &str = "runtime:skill-state-changed";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(entries: &[(&str, &str)]) -> HashMap<String, serde_json::Value> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string())))
            .collect()
    }

    // --- Status-based early returns ---

    #[test]
    fn pending_returns_offline() {
        assert_eq!(
            derive_connection_status(SkillStatus::Pending, true, &HashMap::new()),
            "offline"
        );
    }

    #[test]
    fn stopped_returns_offline() {
        assert_eq!(
            derive_connection_status(SkillStatus::Stopped, true, &HashMap::new()),
            "offline"
        );
    }

    #[test]
    fn stopping_returns_offline() {
        assert_eq!(
            derive_connection_status(SkillStatus::Stopping, true, &HashMap::new()),
            "offline"
        );
    }

    #[test]
    fn error_status_returns_error() {
        assert_eq!(
            derive_connection_status(SkillStatus::Error, true, &HashMap::new()),
            "error"
        );
    }

    #[test]
    fn initializing_returns_connecting() {
        assert_eq!(
            derive_connection_status(SkillStatus::Initializing, false, &HashMap::new()),
            "connecting"
        );
    }

    // --- No published state ---

    #[test]
    fn running_setup_complete_no_state_returns_connected() {
        assert_eq!(
            derive_connection_status(SkillStatus::Running, true, &HashMap::new()),
            "connected"
        );
    }

    #[test]
    fn running_no_setup_no_state_returns_setup_required() {
        assert_eq!(
            derive_connection_status(SkillStatus::Running, false, &HashMap::new()),
            "setup_required"
        );
    }

    // --- Published connection_status ---

    #[test]
    fn conn_error_returns_error() {
        let st = state_with(&[("connection_status", "error")]);
        assert_eq!(
            derive_connection_status(SkillStatus::Running, true, &st),
            "error"
        );
    }

    #[test]
    fn auth_error_returns_error() {
        let st = state_with(&[("auth_status", "error")]);
        assert_eq!(
            derive_connection_status(SkillStatus::Running, true, &st),
            "error"
        );
    }

    #[test]
    fn conn_connecting_returns_connecting() {
        let st = state_with(&[("connection_status", "connecting")]);
        assert_eq!(
            derive_connection_status(SkillStatus::Running, true, &st),
            "connecting"
        );
    }

    #[test]
    fn auth_authenticating_returns_connecting() {
        let st = state_with(&[("auth_status", "authenticating")]);
        assert_eq!(
            derive_connection_status(SkillStatus::Running, true, &st),
            "connecting"
        );
    }

    #[test]
    fn conn_connected_no_auth_returns_connected() {
        let st = state_with(&[("connection_status", "connected")]);
        assert_eq!(
            derive_connection_status(SkillStatus::Running, true, &st),
            "connected"
        );
    }

    #[test]
    fn conn_connected_auth_authenticated_returns_connected() {
        let st = state_with(&[
            ("connection_status", "connected"),
            ("auth_status", "authenticated"),
        ]);
        assert_eq!(
            derive_connection_status(SkillStatus::Running, true, &st),
            "connected"
        );
    }

    #[test]
    fn conn_connected_auth_not_authenticated() {
        let st = state_with(&[
            ("connection_status", "connected"),
            ("auth_status", "not_authenticated"),
        ]);
        assert_eq!(
            derive_connection_status(SkillStatus::Running, true, &st),
            "not_authenticated"
        );
    }

    #[test]
    fn conn_disconnected_setup_complete_returns_disconnected() {
        let st = state_with(&[("connection_status", "disconnected")]);
        assert_eq!(
            derive_connection_status(SkillStatus::Running, true, &st),
            "disconnected"
        );
    }

    #[test]
    fn conn_disconnected_no_setup_returns_setup_required() {
        let st = state_with(&[("connection_status", "disconnected")]);
        assert_eq!(
            derive_connection_status(SkillStatus::Running, false, &st),
            "setup_required"
        );
    }

    #[test]
    fn unknown_state_falls_through_to_connecting() {
        let st = state_with(&[("connection_status", "unknown_value")]);
        assert_eq!(
            derive_connection_status(SkillStatus::Running, true, &st),
            "connecting"
        );
    }
}
