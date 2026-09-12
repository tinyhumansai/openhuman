use super::*;
use crate::openhuman::config::{Config, McpServerConfig};

fn test_registry() -> Arc<McpServerRegistry> {
    let mut config = Config::default();
    config.gitbooks.enabled = false;
    config.mcp_client.servers.push(McpServerConfig {
        name: "docs".into(),
        endpoint: "https://example.com/mcp".into(),
        command: String::new(),
        args: Vec::new(),
        env: std::collections::HashMap::new(),
        cwd: None,
        description: Some("Docs MCP".into()),
        enabled: true,
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        timeout_secs: 30,
        auth: crate::openhuman::config::McpAuthConfig::None,
    });
    // Through the host conversion, so the test builds the registry the
    // same way the application does.
    Arc::new(crate::openhuman::mcp::host::static_registry(&config))
}

#[tokio::test]
async fn list_servers_renders_registry_entries() {
    let tool = McpListServersTool::new(test_registry());
    let result = tool.execute(json!({})).await.expect("execute");
    assert!(result.output().contains("docs"));
    assert!(result.markdown_formatted.is_some());
}

#[tokio::test]
async fn list_tools_requires_server() {
    let tool = McpListToolsTool::new(test_registry());
    let result = tool.execute(json!({})).await;
    assert!(result.is_err());
}
