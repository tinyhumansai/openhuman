use serde::{Deserialize, Serialize};

/// Socket connection status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Error,
}

impl Default for ConnectionStatus {
    fn default() -> Self {
        Self::Disconnected
    }
}

/// Socket connection state emitted to frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocketState {
    pub status: ConnectionStatus,
    pub socket_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Default for SocketState {
    fn default() -> Self {
        Self {
            status: ConnectionStatus::Disconnected,
            socket_id: None,
            error: None,
        }
    }
}

/// Generic socket message wrapper
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocketMessage {
    pub event: String,
    pub data: serde_json::Value,
}

/// MCP request structure (JSON-RPC 2.0)
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// MCP response structure (JSON-RPC 2.0)
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
}

/// MCP error structure
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}
