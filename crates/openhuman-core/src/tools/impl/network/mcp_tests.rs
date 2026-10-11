use super::*;
use crate::config::{Config, McpServerConfig};
use base64::Engine as _;
use serde_json::{json, Value};
use tinymcp::tools::{McpListServersTool, McpListToolsTool};
use tinytools::{Tool, ToolCallOptions, ToolResult};

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
        server: tinymcp_bus::McpServerConfig {
            name: "docs".into(),
            endpoint: endpoint.into(),
            command: String::new(),
            args: Vec::new(),
            env: Default::default(),
            cwd: None,
            description: Some("Docs MCP".into()),
            enabled: true,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            timeout_secs: 30,
            auth,
        },
        ..Default::default()
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

#[tokio::test]
async fn list_servers_strips_endpoint_userinfo() {
    let tool = McpListServersTool::new(registry_with(
        "https://svc-user:private12345@example.com/mcp?token=othersecret#fragment",
        crate::config::McpAuthConfig::None,
    ));
    let result = tool.execute(json!({})).await.expect("execute");
    let rendered = full_output(&result);
    assert!(rendered.contains("https://example.com/mcp"), "{rendered}");
    for secret in ["svc-user", "private12345", "othersecret", "fragment"] {
        assert!(!rendered.contains(secret), "leaked {secret}: {rendered}");
    }
}

struct EchoingServer {
    echo: String,
    fail: Option<&'static str>,
}

impl wiremock::Respond for EchoingServer {
    fn respond(&self, request: &wiremock::Request) -> wiremock::ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let method = body["method"].as_str().unwrap_or_default();
        if Some(method) == self.fail {
            return wiremock::ResponseTemplate::new(500)
                .set_body_string(format!("upstream rejected credential {}", self.echo));
        }
        let result = match method {
            "initialize" => json!({
                "protocolVersion": tinymcp_bus::LATEST_PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "echo", "version": "1.0.0" },
            }),
            "notifications/initialized" => return wiremock::ResponseTemplate::new(202),
            "tools/list" => json!({
                "tools": [{
                    "name": "whoami",
                    "description": format!("Reports the key {}", self.echo),
                    "inputSchema": { "type": "object", "properties": {} },
                }]
            }),
            "tools/call" => json!({
                "content": [
                    { "type": "text", "text": format!("you sent {}", self.echo) },
                ],
                "structuredContent": { "key": self.echo, "nested": [self.echo.clone()] },
                "isError": false,
            }),
            _ => json!({}),
        };
        wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": body["id"].clone(),
            "result": result,
        }))
    }
}

async fn echoing_server(echo: &str, fail: Option<&'static str>) -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(EchoingServer {
            echo: echo.to_string(),
            fail,
        })
        .mount(&server)
        .await;
    server
}

fn call_tool(registry: Arc<McpServerRegistry>) -> tinymcp::tools::McpCallTool {
    mcp_call_tool(registry, Arc::new(SecurityPolicy::default()))
}

fn call_args() -> Value {
    json!({ "server": "docs", "tool": "whoami", "arguments": {} })
}

fn echoed_secret(auth: &crate::config::McpAuthConfig) -> String {
    match auth {
        crate::config::McpAuthConfig::Basic { username, password } => {
            base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"))
        }
        _ => SECRET.to_string(),
    }
}

#[tokio::test]
async fn failing_calls_redact_configured_secrets_from_errors() {
    for (auth, kind) in every_auth_kind() {
        let echo = echoed_secret(&auth);
        let server = echoing_server(&echo, Some("tools/call")).await;
        let registry = registry_with(&format!("{}/mcp", server.uri()), auth);

        let result = call_tool(registry.clone())
            .execute(call_args())
            .await
            .expect("execute");
        let rendered = full_output(&result);
        assert!(result.is_error, "{kind}: {rendered}");
        assert!(
            rendered.contains("mcp_call_tool failed"),
            "{kind}: {rendered}"
        );
        assert!(rendered.contains("[redacted]"), "{kind}: {rendered}");
        assert!(!rendered.contains(SECRET), "{kind} leaked: {rendered}");
        assert!(!rendered.contains(&echo), "{kind} leaked: {rendered}");

        let server = echoing_server(&echo, Some("tools/list")).await;
        let registry = registry_with(&format!("{}/mcp", server.uri()), every_auth_kind_for(kind));
        let result = McpListToolsTool::new(registry)
            .execute(json!({ "server": "docs" }))
            .await
            .expect("execute");
        let rendered = full_output(&result);
        assert!(result.is_error, "{kind}: {rendered}");
        assert!(
            rendered.contains("mcp_list_tools failed"),
            "{kind}: {rendered}"
        );
        assert!(!rendered.contains(SECRET), "{kind} leaked: {rendered}");
        assert!(!rendered.contains(&echo), "{kind} leaked: {rendered}");
    }
}

fn every_auth_kind_for(kind: &str) -> crate::config::McpAuthConfig {
    every_auth_kind()
        .into_iter()
        .find(|(_, k)| *k == kind)
        .map(|(auth, _)| auth)
        .expect("known kind")
}

#[tokio::test]
async fn successful_results_redact_echoed_secrets() {
    for (auth, kind) in every_auth_kind() {
        let echo = echoed_secret(&auth);
        let server = echoing_server(&echo, None).await;
        let registry = registry_with(&format!("{}/mcp", server.uri()), auth);

        let result = call_tool(registry.clone())
            .execute_with_options(
                call_args(),
                ToolCallOptions {
                    prefer_markdown: true,
                },
            )
            .await
            .expect("execute");
        let rendered = full_output(&result);
        assert!(!result.is_error, "{kind}: {rendered}");
        assert!(
            rendered.contains("you sent [redacted]"),
            "{kind}: {rendered}"
        );
        assert!(!rendered.contains(SECRET), "{kind} leaked: {rendered}");
        assert!(!rendered.contains(&echo), "{kind} leaked: {rendered}");

        let result = McpListToolsTool::new(registry)
            .execute(json!({ "server": "docs" }))
            .await
            .expect("execute");
        let rendered = full_output(&result);
        assert!(!result.is_error, "{kind}: {rendered}");
        assert!(rendered.contains("whoami"), "{kind}: {rendered}");
        assert!(!rendered.contains(SECRET), "{kind} leaked: {rendered}");
        assert!(!rendered.contains(&echo), "{kind} leaked: {rendered}");
    }
}

#[tokio::test]
async fn basic_auth_username_is_redacted_from_output() {
    let username = "svc-user";
    let auth = crate::config::McpAuthConfig::Basic {
        username: username.into(),
        password: SECRET.into(),
    };

    let server = echoing_server(username, None).await;
    let registry = registry_with(&format!("{}/mcp", server.uri()), auth.clone());

    let result = call_tool(registry.clone())
        .execute(call_args())
        .await
        .expect("execute");
    let rendered = full_output(&result);
    assert!(!result.is_error, "{rendered}");
    assert!(
        !rendered.contains(username),
        "call leaked username: {rendered}"
    );

    let result = McpListToolsTool::new(registry)
        .execute(json!({ "server": "docs" }))
        .await
        .expect("execute");
    let rendered = full_output(&result);
    assert!(
        !rendered.contains(username),
        "list leaked username: {rendered}"
    );

    let server = echoing_server(username, Some("tools/call")).await;
    let registry = registry_with(&format!("{}/mcp", server.uri()), auth);
    let result = call_tool(registry)
        .execute(call_args())
        .await
        .expect("execute");
    let rendered = full_output(&result);
    assert!(result.is_error, "{rendered}");
    assert!(
        !rendered.contains(username),
        "error leaked username: {rendered}"
    );
}

#[tokio::test]
async fn endpoint_query_secrets_are_redacted_from_errors() {
    let server = echoing_server(SECRET, Some("tools/call")).await;
    let registry = registry_with(
        &format!("{}/mcp?token={SECRET}", server.uri()),
        crate::config::McpAuthConfig::None,
    );
    let result = call_tool(registry)
        .execute(call_args())
        .await
        .expect("execute");
    let rendered = full_output(&result);
    assert!(result.is_error, "{rendered}");
    assert!(!rendered.contains(SECRET), "{rendered}");
}

#[tokio::test]
async fn endpoint_userinfo_secrets_are_redacted_from_tool_results() {
    let username = "mcp-user";
    let password = "url-password-42";
    let server = echoing_server(password, None).await;
    let endpoint = server
        .uri()
        .replace("http://", &format!("http://{username}:{password}@"));
    let registry = registry_with(
        &format!("{endpoint}/mcp"),
        crate::config::McpAuthConfig::None,
    );
    let result = call_tool(registry)
        .execute(call_args())
        .await
        .expect("execute");
    let rendered = full_output(&result);
    assert!(!result.is_error, "{rendered}");
    assert!(rendered.contains("[redacted]"), "{rendered}");
    assert!(!rendered.contains(username), "username leaked: {rendered}");
    assert!(!rendered.contains(password), "password leaked: {rendered}");
}

#[tokio::test]
async fn encoded_query_secrets_are_redacted_from_successful_results() {
    let echo = "a+b%2Fc";
    let server = echoing_server(echo, None).await;
    let registry = registry_with(
        &format!("{}/mcp?api_key={echo}", server.uri()),
        crate::config::McpAuthConfig::None,
    );
    for result in [
        call_tool(registry.clone())
            .execute_with_options(
                call_args(),
                ToolCallOptions {
                    prefer_markdown: true,
                },
            )
            .await
            .expect("call"),
        McpListToolsTool::new(registry)
            .execute(json!({ "server": "docs" }))
            .await
            .expect("list"),
    ] {
        let rendered = full_output(&result);
        assert!(!result.is_error, "{rendered}");
        assert!(rendered.contains("[redacted]"), "{rendered}");
        assert!(!rendered.contains(echo), "{rendered}");
    }
}

#[tokio::test]
async fn call_tool_decodes_json_encoded_arguments() {
    // A model that JSON-encodes `arguments` still reaches the remote tool,
    // and the server receives an object.
    let server = echoing_server("plain", None).await;
    let registry = registry_with(
        &format!("{}/mcp", server.uri()),
        crate::config::McpAuthConfig::None,
    );
    let result = call_tool(registry)
        .execute(json!({ "server": "docs", "tool": "whoami", "arguments": "{\"q\":1}" }))
        .await
        .expect("execute");
    assert!(!result.is_error, "{}", full_output(&result));

    let sent: Vec<Value> = server
        .received_requests()
        .await
        .expect("recording")
        .iter()
        .filter_map(|request| serde_json::from_slice::<Value>(&request.body).ok())
        .filter(|body| body["method"] == "tools/call")
        .map(|body| body["params"]["arguments"].clone())
        .collect();
    assert_eq!(sent, [json!({ "q": 1 })]);
}

#[tokio::test]
async fn call_tool_refuses_arguments_that_are_not_an_object_naming_the_type() {
    let result = call_tool(test_registry())
        .execute(json!({ "server": "docs", "tool": "whoami", "arguments": 7 }))
        .await
        .expect("execute");
    assert!(result.is_error);
    assert!(result.output().contains("a number"), "{}", result.output());
}
