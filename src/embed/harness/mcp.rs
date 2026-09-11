//! MCP servers, declared programmatically.
//!
//! # Why this is a config write rather than a registry call
//!
//! The static MCP registry has exactly one constructor —
//! `McpServerRegistry::from_config` — and no public `register`. That is not an
//! oversight to route around: `McpServerDefinition` holds a private
//! `McpTransportClient`, so a definition cannot be built from outside the module
//! at all, and the transport itself is a closed enum. **The config is the
//! registration API.** So [`McpServer`] compiles to the same
//! `McpServerConfig` a `[[mcp_client.servers]]` block would parse into, and the
//! harness pushes it onto `config.mcp_client.servers` before the core boots.
//!
//! The practical consequence: servers are fixed at build time. Adding one to a
//! running harness is not possible here, and would need a registry seam
//! upstream rather than a change in this module.

use std::collections::HashMap;

use crate::openhuman::config::schema::McpServerConfig;
pub use crate::openhuman::config::schema::{HttpHeader, McpAuthConfig};

/// One MCP server the agent may call tools on.
///
/// Build with [`stdio`](Self::stdio) for a local subprocess or
/// [`http`](Self::http) for a remote endpoint; the two transports are mutually
/// exclusive and the constructors keep them that way.
#[derive(Clone)]
pub struct McpServer(McpServerConfig);

impl std::fmt::Debug for McpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `McpServerConfig` and `McpAuthConfig` both derive Debug without
        // redaction, so the derived formatter would print bearer tokens,
        // passwords, header values and stdio environment values. A harness
        // may be logged or surfaced in an error; keep the identifying fields
        // (name, endpoint, command, enabled tools) and redact everything
        // credential-shaped.
        let cfg = &self.0;
        f.debug_struct("McpServer")
            .field("name", &cfg.name)
            // The endpoint can itself embed credentials (userinfo/query); show
            // a sanitized form so it cannot leak into host logs.
            .field(
                "endpoint",
                &crate::embed::agent::sanitize_url_for_display(&cfg.endpoint),
            )
            .field("command", &cfg.command)
            .field("args", &cfg.args)
            .field("env_redacted", &(!cfg.env.is_empty()))
            .field("cwd", &cfg.cwd)
            .field("description", &cfg.description)
            .field("enabled", &cfg.enabled)
            .field("allowed_tools", &cfg.allowed_tools)
            .field("disallowed_tools", &cfg.disallowed_tools)
            .field("timeout_secs", &cfg.timeout_secs)
            .field("auth", &McpAuthDebug(&cfg.auth))
            .finish()
    }
}

/// Redacting view of an [`McpAuthConfig`]: reveals only the auth kind, never the
/// credential value.
struct McpAuthDebug<'a>(&'a McpAuthConfig);

impl std::fmt::Debug for McpAuthDebug<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            McpAuthConfig::None => f.write_str("None"),
            McpAuthConfig::BearerToken { .. } => f.write_str("BearerToken(<redacted>)"),
            McpAuthConfig::Basic { .. } => f.write_str("Basic(<redacted>)"),
            McpAuthConfig::Header { name, .. } => f
                .debug_tuple("Header")
                .field(name)
                .field(&"<redacted>")
                .finish(),
            McpAuthConfig::Headers { headers } => f
                .debug_tuple("Headers")
                .field(&headers.len())
                .field(&"<redacted>")
                .finish(),
            McpAuthConfig::QueryParam { name, .. } => f
                .debug_tuple("QueryParam")
                .field(name)
                .field(&"<redacted>")
                .finish(),
        }
    }
}

impl McpServer {
    /// A local server launched as a subprocess, speaking newline-delimited
    /// JSON-RPC over stdin/stdout.
    ///
    /// `name` is the slug the agent sees in the bridge tools, so make it stable
    /// — it appears in prompts and in tool-call arguments.
    pub fn stdio<I, S>(name: impl Into<String>, command: impl Into<String>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(McpServerConfig {
            name: name.into(),
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
            ..Default::default()
        })
    }

    /// A remote server over Streamable HTTP.
    pub fn http(name: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self(McpServerConfig {
            name: name.into(),
            endpoint: endpoint.into(),
            ..Default::default()
        })
    }

    /// Environment variables for a stdio server. MCP stdio auth is normally
    /// passed this way; ignored by an HTTP server, which authenticates with
    /// [`auth`](Self::auth).
    pub fn env<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let vars: HashMap<String, String> = vars
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        self.0.env.extend(vars);
        self
    }

    /// Working directory for a stdio server's subprocess.
    pub fn cwd(mut self, dir: impl Into<String>) -> Self {
        self.0.cwd = Some(dir.into());
        self
    }

    /// Outbound auth for an HTTP server.
    pub fn auth(mut self, auth: McpAuthConfig) -> Self {
        self.0.auth = auth;
        self
    }

    /// Expose only these remote tools to the agent.
    ///
    /// Empty (the default) means all of them. Worth setting for a large server:
    /// every exposed tool costs prompt budget on every turn.
    pub fn allow_tools<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.0.allowed_tools = tools.into_iter().map(Into::into).collect();
        self
    }

    /// Always hide and block these remote tools. Takes precedence over
    /// [`allow_tools`](Self::allow_tools).
    pub fn deny_tools<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.0.disallowed_tools = tools.into_iter().map(Into::into).collect();
        self
    }

    /// Per-request timeout in seconds (default 30).
    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.0.timeout_secs = secs;
        self
    }

    /// Human-readable description, shown in the bridge tool output.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.0.description = Some(description.into());
        self
    }

    /// The server slug.
    pub fn name(&self) -> &str {
        &self.0.name
    }

    /// The underlying config entry.
    pub(super) fn into_config(self) -> McpServerConfig {
        self.0
    }
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
