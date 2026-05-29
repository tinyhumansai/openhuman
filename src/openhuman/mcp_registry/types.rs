//! Types for the MCP clients domain.
//!
//! This module defines all data structures used for the Smithery.ai registry
//! integration, local server installation tracking, connection state, and
//! MCP stdio protocol framing.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── CommandKind ─────────────────────────────────────────────────────────────

/// How to launch the MCP server subprocess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandKind {
    /// Launched via `npx` (Node.js ecosystem).
    Node,
    /// Launched via `uvx` (Python ecosystem).
    Python,
    /// Direct binary execution.
    Binary,
}

impl CommandKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Python => "python",
            Self::Binary => "binary",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "python" => Self::Python,
            "binary" => Self::Binary,
            _ => Self::Node,
        }
    }
}

// ── Transport ────────────────────────────────────────────────────────────────

/// How a connected MCP server's transport is dialled.
///
/// Mirrors `mcp_client::registry::McpTransportClient` at the install-record
/// layer — same two backends (`McpStdioClient` / `McpHttpClient`), one extra
/// indirection because the install row has to be serialisable + persistable
/// across restarts. The `dispatch_kind` string is what we persist into the
/// `mcp_servers.transport` column (`"stdio"` | `"http_remote"`); existing
/// rows from before the column existed default to `"stdio"` per the
/// store-side migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Transport {
    /// Local subprocess JSON-RPC over stdin/stdout. Spawned via
    /// `command` + `args` (resolved from `command_kind`).
    Stdio,
    /// HTTPS endpoint hosted by the upstream registry (typically Smithery —
    /// `~99%` of their listings are HTTP-remote). Streamable HTTP + OAuth +
    /// SSE per the MCP spec, served by [`mcp_client::McpHttpClient`].
    HttpRemote { url: String },
}

impl Transport {
    /// Stable string identifier persisted in `mcp_servers.transport`.
    /// Kept narrow on purpose — schema migrations notice unknown values.
    pub fn dispatch_kind(&self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::HttpRemote { .. } => "http_remote",
        }
    }

    /// Inverse of `dispatch_kind`. Unknown / missing values fall back to
    /// `Stdio` so pre-migration rows (where the column didn't exist and
    /// every record was implicitly stdio) keep working with no behaviour
    /// change.
    pub fn parse(kind: &str, deployment_url: Option<&str>) -> Self {
        match kind {
            "http_remote" => Self::HttpRemote {
                url: deployment_url.unwrap_or("").to_string(),
            },
            _ => Self::Stdio,
        }
    }

    /// `Some(url)` for HTTP-remote, `None` for stdio. Convenience accessor
    /// for the store layer that needs to persist `deployment_url` as its
    /// own column.
    pub fn deployment_url(&self) -> Option<&str> {
        match self {
            Self::Stdio => None,
            Self::HttpRemote { url } => Some(url.as_str()),
        }
    }
}

// ── InstalledServer ─────────────────────────────────────────────────────────

/// A locally installed MCP server record.
///
/// Env values are intentionally NOT stored here — only the key names.
/// Values live in the `mcp_client_env` table and are never serialized
/// into list or status responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledServer {
    /// Stable UUID v4 generated at install time.
    pub server_id: String,
    /// Smithery registry qualified name, e.g. `@modelcontextprotocol/server-filesystem`.
    pub qualified_name: String,
    /// Human-readable display name from the registry.
    pub display_name: String,
    /// Short description from the registry.
    pub description: Option<String>,
    /// Icon URL from the registry.
    pub icon_url: Option<String>,
    /// How the server subprocess should be launched (stdio installs only).
    /// For HTTP-remote installs this is still set to a sensible default —
    /// callers route off [`Self::transport`] instead.
    pub command_kind: CommandKind,
    /// Resolved binary or launcher (`npx`, `uvx`, etc). Empty string for
    /// HTTP-remote installs.
    pub command: String,
    /// Arguments passed to `command`. Empty vec for HTTP-remote installs.
    pub args: Vec<String>,
    /// Names of required env vars (values are stored separately and never logged).
    pub env_keys: Vec<String>,
    /// Optional JSON configuration blob.
    pub config: Option<Value>,
    /// Unix epoch milliseconds when the server was installed.
    pub installed_at: i64,
    /// Unix epoch milliseconds when the server last connected successfully.
    pub last_connected_at: Option<i64>,
    /// Transport variant for this install — `Stdio` for legacy / local
    /// subprocess servers, `HttpRemote { url }` for Smithery-hosted ones.
    /// Defaults to `Stdio` for rows persisted before the column existed.
    #[serde(default = "default_transport")]
    pub transport: Transport,
}

/// Default for `InstalledServer::transport` when the field is missing from
/// a serialised payload (e.g. legacy persisted rows, callers that haven't
/// migrated their construction site yet).
fn default_transport() -> Transport {
    Transport::Stdio
}

// ── McpTool ─────────────────────────────────────────────────────────────────

/// A tool exposed by a connected MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

// ── ConnStatus ──────────────────────────────────────────────────────────────

/// Connection status summary for one installed server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServerStatus {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

impl ServerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Error => "error",
        }
    }
}

/// Per-server connection status summary returned by `openhuman.mcp_clients_status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnStatus {
    pub server_id: String,
    pub qualified_name: String,
    pub display_name: String,
    pub status: ServerStatus,
    pub tool_count: u32,
    pub last_error: Option<String>,
}

// ── Smithery registry DTOs ───────────────────────────────────────────────────

/// Summary record returned by `GET /servers`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmitheryServerSummary {
    pub qualified_name: String,
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub use_count: u64,
    #[serde(default)]
    pub is_deployed: bool,
    /// Upstream registry id (`"smithery"` | `"mcp_official"`). Always set
    /// by the dispatcher in `super::registries` so the frontend can attribute
    /// rows and the install path can route `registry_get` back to the
    /// originating upstream.
    #[serde(default)]
    pub source: String,
    /// Raw extra fields preserved for future use.
    #[serde(flatten, default)]
    pub extra: std::collections::HashMap<String, Value>,
}

/// Detail record returned by `GET /servers/{qualifiedName}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmitheryServerDetail {
    pub qualified_name: String,
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub connections: Vec<SmitheryConnection>,
    /// Upstream registry id (`"smithery"` | `"mcp_official"`).
    #[serde(default)]
    pub source: String,
    #[serde(flatten, default)]
    pub extra: std::collections::HashMap<String, Value>,
}

/// One connection type listed on a server detail.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmitheryConnection {
    /// `"stdio"` or `"http"`.
    pub r#type: String,
    #[serde(default)]
    pub deployment_url: Option<String>,
    #[serde(default)]
    pub config_schema: Option<Value>,
    #[serde(default)]
    pub example_config: Option<Value>,
    #[serde(default)]
    pub published: bool,
    #[serde(flatten, default)]
    pub extra: std::collections::HashMap<String, Value>,
}

/// Pagination wrapper from Smithery's `/servers` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SmitheryPagination {
    #[serde(default)]
    pub current_page: u32,
    #[serde(default)]
    pub page_size: u32,
    #[serde(default)]
    pub total_pages: u32,
    #[serde(default)]
    pub total_count: u64,
}

/// Full response body from `GET /servers`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmitheryListResponse {
    pub servers: Vec<SmitheryServerSummary>,
    #[serde(default)]
    pub pagination: SmitheryPagination,
}

// ── Chat history entry (for config_assist) ───────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurn {
    pub role: String,
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn command_kind_roundtrip() {
        assert_eq!(CommandKind::parse("node").as_str(), "node");
        assert_eq!(CommandKind::parse("python").as_str(), "python");
        assert_eq!(CommandKind::parse("binary").as_str(), "binary");
        assert_eq!(CommandKind::parse("unknown").as_str(), "node");
    }

    #[test]
    fn server_status_as_str() {
        assert_eq!(ServerStatus::Connected.as_str(), "connected");
        assert_eq!(ServerStatus::Disconnected.as_str(), "disconnected");
        assert_eq!(ServerStatus::Connecting.as_str(), "connecting");
        assert_eq!(ServerStatus::Error.as_str(), "error");
    }

    #[test]
    fn smithery_server_summary_tolerates_missing_optional_fields() {
        let raw = json!({
            "qualifiedName": "@test/server",
            "displayName": "Test Server"
        });
        let s: SmitheryServerSummary = serde_json::from_value(raw).unwrap();
        assert_eq!(s.qualified_name, "@test/server");
        assert!(s.description.is_none());
        assert_eq!(s.use_count, 0);
        assert!(!s.is_deployed);
    }

    #[test]
    fn smithery_list_response_parses_pagination() {
        let raw = json!({
            "servers": [],
            "pagination": {
                "currentPage": 1,
                "pageSize": 20,
                "totalPages": 3,
                "totalCount": 55
            }
        });
        let resp: SmitheryListResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(resp.pagination.current_page, 1);
        assert_eq!(resp.pagination.total_pages, 3);
        assert_eq!(resp.pagination.total_count, 55);
    }

    #[test]
    fn installed_server_serializes_without_env_values() {
        let server = InstalledServer {
            server_id: "uuid-1".to_string(),
            qualified_name: "@test/server".to_string(),
            display_name: "Test".to_string(),
            description: None,
            icon_url: None,
            command_kind: CommandKind::Node,
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "@test/server".to_string()],
            env_keys: vec!["API_KEY".to_string()],
            config: None,
            installed_at: 1_700_000_000_000,
            last_connected_at: None,
            transport: Transport::Stdio,
        };
        let v = serde_json::to_value(&server).unwrap();
        // env_keys present, but no raw values
        assert!(v.get("env_keys").is_some());
        assert!(v.get("env_values").is_none());
    }

    /// `Transport::dispatch_kind` is the column value persisted into
    /// `mcp_servers.transport`. Pinning both stdio and http-remote so a
    /// schema-side change can't silently rename one without surfacing here.
    #[test]
    fn transport_dispatch_kind_strings_are_stable() {
        assert_eq!(Transport::Stdio.dispatch_kind(), "stdio");
        assert_eq!(
            Transport::HttpRemote {
                url: "https://example.com/mcp".to_string()
            }
            .dispatch_kind(),
            "http_remote"
        );
    }

    /// `Transport::parse` is what the store layer calls when re-hydrating
    /// a row. The Stdio fallback for unknown / missing values is the
    /// migration-safety hatch — rows persisted before the `transport`
    /// column existed must keep working as stdio installs.
    #[test]
    fn transport_parse_falls_back_to_stdio_for_unknown_kinds() {
        // Stdio: explicit + with-no-url
        assert_eq!(Transport::parse("stdio", None), Transport::Stdio);
        assert_eq!(Transport::parse("stdio", Some("ignored")), Transport::Stdio);

        // Pre-migration empty value → stdio (backwards-compat).
        assert_eq!(Transport::parse("", None), Transport::Stdio);
        // Unknown kind from a future row → stdio (defensive default; we'd
        // rather a misconfigured row stall on connect than misroute).
        assert_eq!(Transport::parse("garbage", None), Transport::Stdio);

        // HTTP remote round-trip carries the URL through.
        assert_eq!(
            Transport::parse("http_remote", Some("https://x.io/mcp")),
            Transport::HttpRemote {
                url: "https://x.io/mcp".to_string()
            }
        );
    }

    /// `deployment_url` accessor is what the store uses to populate the
    /// adjacent `mcp_servers.deployment_url` column. Stdio → `None`,
    /// HTTP remote → `Some(url)`. Confirms the two never get crossed.
    #[test]
    fn transport_deployment_url_accessor() {
        assert_eq!(Transport::Stdio.deployment_url(), None);
        let http = Transport::HttpRemote {
            url: "https://smithery.ai/server/x".to_string(),
        };
        assert_eq!(http.deployment_url(), Some("https://smithery.ai/server/x"));
    }

    /// `InstalledServer::transport` is `#[serde(default)]`-backed so that
    /// pre-migration JSON payloads (without the field at all) deserialise
    /// as stdio installs. Without this, every persisted row from before
    /// this change would fail to load after upgrade.
    #[test]
    fn installed_server_defaults_transport_to_stdio_on_missing_field() {
        let legacy = json!({
            "server_id": "uuid-1",
            "qualified_name": "@old/server",
            "display_name": "Old",
            "description": null,
            "icon_url": null,
            "command_kind": "node",
            "command": "npx",
            "args": ["-y", "@old/server"],
            "env_keys": [],
            "config": null,
            "installed_at": 1_700_000_000_000i64,
            "last_connected_at": null
            // ← deliberately no `transport` key
        });
        let s: InstalledServer = serde_json::from_value(legacy).unwrap();
        assert_eq!(s.transport, Transport::Stdio);
    }

    #[test]
    fn conn_status_status_field_serializes_lowercase() {
        let s = ConnStatus {
            server_id: "s1".to_string(),
            qualified_name: "@test/s".to_string(),
            display_name: "S".to_string(),
            status: ServerStatus::Connected,
            tool_count: 3,
            last_error: None,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["status"], json!("connected"));
    }
}
