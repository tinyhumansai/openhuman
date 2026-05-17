use super::client::{
    McpAuthorizationContext, McpHttpClient, McpInitializeResult, McpRemoteTool, McpServerToolResult,
};
use super::stdio::McpStdioClient;
use crate::openhuman::config::{Config, McpAuthConfig, McpClientIdentityConfig, McpServerConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpRegistrySource {
    Config,
    LegacyGitbooks,
}

#[derive(Debug, Clone)]
pub struct McpServerDefinition {
    pub name: String,
    pub endpoint: String,
    pub command: Option<String>,
    pub description: Option<String>,
    pub timeout_secs: u64,
    pub auth: McpAuthConfig,
    pub source: McpRegistrySource,
    client: Arc<McpTransportClient>,
}

#[derive(Debug)]
pub enum McpTransportClient {
    Http(McpHttpClient),
    Stdio(McpStdioClient),
}

#[derive(Debug, Default, Clone)]
pub struct McpServerRegistry {
    by_name: HashMap<String, McpServerDefinition>,
    order: Vec<String>,
}

impl McpServerRegistry {
    pub fn from_config(config: &Config) -> Self {
        let mut registry = Self::default();
        if !config.mcp_client.enabled {
            return registry;
        }

        for server in &config.mcp_client.servers {
            registry.register_config_server(
                server,
                &config.mcp_client.client_identity,
                McpRegistrySource::Config,
            );
        }

        if config.gitbooks.enabled && registry.get("gitbooks").is_none() {
            registry.insert(McpServerDefinition {
                name: "gitbooks".into(),
                endpoint: config.gitbooks.endpoint.clone(),
                command: None,
                description: Some("OpenHuman GitBook documentation MCP server.".into()),
                timeout_secs: config.gitbooks.timeout_secs,
                auth: McpAuthConfig::None,
                source: McpRegistrySource::LegacyGitbooks,
                client: Arc::new(McpTransportClient::Http(McpHttpClient::with_options(
                    config.gitbooks.endpoint.clone(),
                    config.gitbooks.timeout_secs,
                    McpAuthConfig::None,
                    config.mcp_client.client_identity.clone(),
                ))),
            });
        }

        registry
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn list(&self) -> Vec<&McpServerDefinition> {
        self.order
            .iter()
            .filter_map(|name| self.by_name.get(name))
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<&McpServerDefinition> {
        self.by_name.get(name)
    }

    pub async fn list_tools(&self, server: &str) -> anyhow::Result<Vec<McpRemoteTool>> {
        let server = self
            .get(server)
            .ok_or_else(|| anyhow::anyhow!("unknown MCP server `{server}`"))?;
        server.client.list_tools().await
    }

    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> anyhow::Result<McpServerToolResult> {
        let server = self
            .get(server)
            .ok_or_else(|| anyhow::anyhow!("unknown MCP server `{server}`"))?;
        server.client.call_tool(tool, arguments).await
    }

    pub async fn initialize(&self, server: &str) -> anyhow::Result<McpInitializeResult> {
        let server = self
            .get(server)
            .ok_or_else(|| anyhow::anyhow!("unknown MCP server `{server}`"))?;
        server.client.initialize().await
    }

    pub async fn discover_authorization(
        &self,
        server: &str,
    ) -> anyhow::Result<Option<McpAuthorizationContext>> {
        let server = self
            .get(server)
            .ok_or_else(|| anyhow::anyhow!("unknown MCP server `{server}`"))?;
        server.client.discover_authorization().await
    }

    fn register_config_server(
        &mut self,
        server: &McpServerConfig,
        identity: &McpClientIdentityConfig,
        source: McpRegistrySource,
    ) {
        if !server.enabled {
            return;
        }
        let name = server.name.trim();
        let endpoint = server.endpoint.trim();
        let command = server.command.trim();
        if name.is_empty() || (endpoint.is_empty() && command.is_empty()) {
            tracing::warn!(
                name = server.name,
                endpoint = server.endpoint,
                command = server.command,
                "[mcp_client] skipping malformed MCP server config entry"
            );
            return;
        }
        self.insert(McpServerDefinition {
            name: name.to_string(),
            endpoint: endpoint.to_string(),
            command: transport_command(server),
            description: server.description.clone(),
            timeout_secs: server.timeout_secs,
            auth: server.auth.clone(),
            source,
            client: Arc::new(build_transport_client(server, identity)),
        });
    }

    fn insert(&mut self, def: McpServerDefinition) {
        let name = def.name.clone();
        if self.by_name.insert(name.clone(), def).is_none() {
            self.order.push(name);
        }
    }
}

impl McpTransportClient {
    pub async fn initialize(&self) -> anyhow::Result<McpInitializeResult> {
        match self {
            Self::Http(client) => client.initialize().await,
            Self::Stdio(client) => client.initialize().await,
        }
    }

    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpRemoteTool>> {
        match self {
            Self::Http(client) => client.list_tools().await,
            Self::Stdio(client) => client.list_tools().await,
        }
    }

    pub async fn call_tool(
        &self,
        tool: &str,
        arguments: Value,
    ) -> anyhow::Result<McpServerToolResult> {
        match self {
            Self::Http(client) => client.call_tool(tool, arguments).await,
            Self::Stdio(client) => client.call_tool(tool, arguments).await,
        }
    }

    pub async fn discover_authorization(&self) -> anyhow::Result<Option<McpAuthorizationContext>> {
        match self {
            Self::Http(client) => client.discover_authorization().await,
            Self::Stdio(_) => Ok(None),
        }
    }
}

fn build_transport_client(
    server: &McpServerConfig,
    identity: &McpClientIdentityConfig,
) -> McpTransportClient {
    if !server.command.trim().is_empty() {
        let env = server
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>();
        McpTransportClient::Stdio(McpStdioClient::new(
            server.command.trim().to_string(),
            server.args.clone(),
            env,
            server.cwd.as_ref().map(PathBuf::from),
            identity.clone(),
        ))
    } else {
        McpTransportClient::Http(McpHttpClient::with_options(
            server.endpoint.trim().to_string(),
            server.timeout_secs,
            server.auth.clone(),
            identity.clone(),
        ))
    }
}

fn transport_command(server: &McpServerConfig) -> Option<String> {
    let command = server.command.trim();
    if command.is_empty() {
        None
    } else {
        Some(command.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_config_seeds_legacy_gitbooks_when_enabled() {
        let config = Config::default();
        let registry = McpServerRegistry::from_config(&config);
        let gitbooks = registry.get("gitbooks").expect("gitbooks");
        assert_eq!(gitbooks.source, McpRegistrySource::LegacyGitbooks);
    }

    #[test]
    fn explicit_server_overrides_legacy_name() {
        let mut config = Config::default();
        config.mcp_client.servers.push(McpServerConfig {
            name: "gitbooks".into(),
            endpoint: "https://example.com/mcp".into(),
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            description: Some("Custom docs".into()),
            enabled: true,
            timeout_secs: 9,
            auth: crate::openhuman::config::McpAuthConfig::None,
        });
        let registry = McpServerRegistry::from_config(&config);
        let gitbooks = registry.get("gitbooks").expect("gitbooks");
        assert_eq!(gitbooks.source, McpRegistrySource::Config);
        assert_eq!(gitbooks.endpoint, "https://example.com/mcp");
    }

    #[test]
    fn disabled_config_short_circuits_registry() {
        let mut config = Config::default();
        config.mcp_client.enabled = false;
        let registry = McpServerRegistry::from_config(&config);
        assert!(registry.is_empty());
    }
}
