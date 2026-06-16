//! MCP client, server, auth, and GitBooks config types.

use super::super::defaults;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct GitbooksConfig {
    /// When `true`, register `gitbooks_search` and `gitbooks_get_page`.
    #[serde(default = "defaults::default_true")]
    pub enabled: bool,
    /// MCP endpoint URL for the OpenHuman GitBook docs.
    #[serde(default = "default_gitbooks_endpoint")]
    pub endpoint: String,
    /// Per-request timeout in seconds.
    #[serde(default = "default_gitbooks_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_gitbooks_endpoint() -> String {
    "https://tinyhumans.gitbook.io/openhuman/~gitbook/mcp".into()
}

fn default_gitbooks_timeout_secs() -> u64 {
    30
}

impl Default for GitbooksConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_true(),
            endpoint: default_gitbooks_endpoint(),
            timeout_secs: default_gitbooks_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct McpServerConfig {
    /// Stable server slug used by the agent-facing bridge tools.
    #[serde(default)]
    pub name: String,
    /// MCP endpoint URL. Current implementation supports stateless
    /// Streamable HTTP / JSON responses.
    #[serde(default)]
    pub endpoint: String,
    /// Optional stdio command for local MCP servers. When set, the
    /// client launches this command as a subprocess and speaks newline-
    /// delimited JSON-RPC over stdin/stdout per the MCP stdio transport.
    #[serde(default)]
    pub command: String,
    /// Command-line arguments for stdio MCP servers.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables for stdio MCP servers. MCP stdio auth
    /// is typically passed this way.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Optional working directory for stdio MCP servers.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Optional human-readable description shown in bridge tool output.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether this server should be exposed to the MCP bridge tools.
    #[serde(default = "defaults::default_true")]
    pub enabled: bool,
    /// Exact remote tool names this server may expose through the generic
    /// MCP bridge. Empty means all remote tools are allowed unless they
    /// appear in `disallowed_tools`.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Exact remote tool names that should always be hidden and blocked.
    /// This denylist takes precedence over `allowed_tools`.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    /// Per-request timeout in seconds.
    #[serde(default = "default_mcp_timeout_secs")]
    pub timeout_secs: u64,
    /// Optional auth strategy applied to outbound requests for this
    /// server. Useful for API-key and pre-provisioned bearer-token
    /// flows; interactive OAuth discovery is handled by the client
    /// transport separately when a server returns an auth challenge.
    #[serde(default)]
    pub auth: McpAuthConfig,
}

fn default_mcp_timeout_secs() -> u64 {
    30
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            endpoint: String::new(),
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            description: None,
            enabled: defaults::default_true(),
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            timeout_secs: default_mcp_timeout_secs(),
            auth: McpAuthConfig::None,
        }
    }
}

/// One HTTP header (name + value) for the multi-header [`McpAuthConfig::Headers`]
/// variant — e.g. a remote that requires both `X-Client-Key` and
/// `X-Client-Secret`, or an API key plus an org id.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpAuthConfig {
    None,
    BearerToken {
        token: String,
    },
    Basic {
        username: String,
        password: String,
    },
    Header {
        name: String,
        value: String,
    },
    /// Multiple request headers, all applied — for remotes that authenticate
    /// with more than one header (single-header servers use [`Self::Header`]).
    Headers {
        headers: Vec<HttpHeader>,
    },
    QueryParam {
        name: String,
        value: String,
    },
}

impl Default for McpAuthConfig {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct McpClientIdentityConfig {
    /// Client name sent during `initialize.clientInfo.name`.
    #[serde(default = "default_mcp_client_name")]
    pub name: String,
    /// Client title sent during `initialize.clientInfo.title`.
    #[serde(default = "default_mcp_client_title")]
    pub title: String,
    /// Client version sent during `initialize.clientInfo.version`.
    #[serde(default = "default_mcp_client_version")]
    pub version: String,
}

fn default_mcp_client_name() -> String {
    "openhuman-core".into()
}

fn default_mcp_client_title() -> String {
    "OpenHuman Core MCP Client".into()
}

fn default_mcp_client_version() -> String {
    env!("CARGO_PKG_VERSION").into()
}

impl Default for McpClientIdentityConfig {
    fn default() -> Self {
        Self {
            name: default_mcp_client_name(),
            title: default_mcp_client_title(),
            version: default_mcp_client_version(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct McpClientConfig {
    /// When `true`, register the generic MCP bridge tools and expose
    /// configured remote MCP servers to the agent runtime.
    #[serde(default = "defaults::default_true")]
    pub enabled: bool,
    /// Named remote MCP servers accessible via `mcp_list_*` /
    /// `mcp_call_tool`.
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
    /// Identity block sent during initialize.
    #[serde(default)]
    pub client_identity: McpClientIdentityConfig,
    /// Optional auth/overrides for the MCP *registry* browse APIs (Smithery +
    /// the official modelcontextprotocol/registry). Each value falls back to
    /// the corresponding env var when unset (issue #3039 gap A6).
    #[serde(default)]
    pub registry_auth: McpRegistryAuthConfig,
}

impl Default for McpClientConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_true(),
            servers: Vec::new(),
            client_identity: McpClientIdentityConfig::default(),
            registry_auth: McpRegistryAuthConfig::default(),
        }
    }
}

/// Registry-browse auth + endpoint overrides. Lets a user who hits Smithery
/// rate limits (or needs an authenticated official-registry endpoint) supply
/// credentials from the desktop app instead of editing env vars. Each field is
/// config-first with an env-var fallback so existing CI/Docker deployments that
/// only set env vars keep working unchanged.
///
/// Secrets are write-only over RPC: the getter reports whether each secret is
/// *set* (a boolean) and never echoes the value back.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct McpRegistryAuthConfig {
    /// Smithery API key. Falls back to `SMITHERY_API_KEY`.
    #[serde(default)]
    pub smithery_api_key: Option<String>,
    /// Base URL override for the official registry. Falls back to
    /// `MCP_OFFICIAL_REGISTRY_BASE` (non-secret).
    #[serde(default)]
    pub mcp_official_base: Option<String>,
    /// Bearer token for the official registry. Falls back to
    /// `MCP_OFFICIAL_REGISTRY_TOKEN`.
    #[serde(default)]
    pub mcp_official_token: Option<String>,
}
