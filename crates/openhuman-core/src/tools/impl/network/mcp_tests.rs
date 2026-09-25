use super::*;
use crate::config::{Config, McpServerConfig};

fn test_registry() -> Arc<McpServerRegistry> {
    registry_with(
        "https://example.com/mcp",
        crate::config::McpAuthConfig::None,
    )
}

fn registry_with(endpoint: &str, auth: crate::config::McpAuthConfig) -> Arc<McpServerRegistry> {
    let mut config = Config::default();
    config.gitbooks.enabled = false;
    config.mcp_client.servers.push(McpServerConfig {
        name: "docs".into(),
        endpoint: endpoint.into(),
        command: String::new(),
        args: Vec::new(),
        env: std::collections::HashMap::new(),
        cwd: None,
        description: Some("Docs MCP".into()),
        enabled: true,
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        timeout_secs: 30,
        auth,
    });
    // Through the host conversion, so the test builds the registry the
    // same way the application does.
    Arc::new(crate::mcp::host::static_registry(&config))
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

const SECRET: &str = "s3cr3t-Value_91";

fn every_auth_kind() -> Vec<(crate::config::McpAuthConfig, &'static str)> {
    use crate::config::{HttpHeader, McpAuthConfig};
    vec![
        (
            McpAuthConfig::BearerToken {
                token: SECRET.into(),
            },
            "bearer_token",
        ),
        (
            McpAuthConfig::Basic {
                username: "svc-user".into(),
                password: SECRET.into(),
            },
            "basic",
        ),
        (
            McpAuthConfig::Header {
                name: "X-Api-Key".into(),
                value: SECRET.into(),
            },
            "header",
        ),
        (
            McpAuthConfig::Headers {
                headers: vec![
                    HttpHeader {
                        name: "X-Org".into(),
                        value: "org-7".into(),
                    },
                    HttpHeader {
                        name: "X-Api-Key".into(),
                        value: SECRET.into(),
                    },
                ],
            },
            "headers",
        ),
        (
            McpAuthConfig::QueryParam {
                name: "api_key".into(),
                value: SECRET.into(),
            },
            "query_param",
        ),
    ]
}

fn full_output(result: &ToolResult) -> String {
    format!(
        "{}\n{}\n{}",
        result.output(),
        result.markdown_formatted.clone().unwrap_or_default(),
        serde_json::to_string(&result.content).unwrap_or_default()
    )
}

#[tokio::test]
async fn list_servers_never_emits_auth_secrets() {
    for (auth, kind) in every_auth_kind() {
        let tool = McpListServersTool::new(registry_with("https://example.com/mcp", auth));
        let result = tool.execute(json!({})).await.expect("execute");
        let rendered = full_output(&result);
        assert!(!rendered.contains(SECRET), "{kind} leaked: {rendered}");
        assert!(
            rendered.contains("\"auth_configured\":true"),
            "{kind}: {rendered}"
        );
        assert!(rendered.contains(kind), "{kind}: {rendered}");
    }
}

#[tokio::test]
async fn list_servers_reports_no_auth_as_unconfigured() {
    let tool = McpListServersTool::new(test_registry());
    let result = tool.execute(json!({})).await.expect("execute");
    let rendered = full_output(&result);
    assert!(rendered.contains("\"auth_configured\":false"), "{rendered}");
    assert!(!rendered.contains("\"auth\":"), "{rendered}");
}

#[tokio::test]
async fn list_servers_strips_endpoint_query_string() {
    let tool = McpListServersTool::new(registry_with(
        &format!("https://example.com/mcp?token={SECRET}&v=2#frag"),
        crate::config::McpAuthConfig::None,
    ));
    let result = tool.execute(json!({})).await.expect("execute");
    let rendered = full_output(&result);
    assert!(!rendered.contains(SECRET), "{rendered}");
    assert!(!rendered.contains("v=2"), "{rendered}");
    assert!(rendered.contains("https://example.com/mcp"), "{rendered}");
}
