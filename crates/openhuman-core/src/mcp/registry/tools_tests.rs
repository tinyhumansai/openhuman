use super::*;
use async_trait::async_trait;
use serde_json::json;
use std::sync::{Arc as SharedArc, Mutex};
use tinytools::ToolScope;

fn cfg() -> Arc<Config> {
    Arc::new(Config::default())
}

#[test]
fn names_and_levels() {
    assert_eq!(
        McpRegistrySearchTool::new(cfg()).name(),
        "mcp_registry_search"
    );
    assert_eq!(
        McpRegistrySearchTool::new(cfg()).permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(
        McpRegistryConnectTool::new(cfg()).permission_level(),
        PermissionLevel::Execute
    );
    assert_eq!(
        McpRegistryToolCallTool::new(cfg()).permission_level(),
        PermissionLevel::Execute
    );
    // Discovery tool: read-only, names match.
    assert_eq!(
        McpRegistryListToolsTool::new(cfg()).name(),
        "mcp_registry_list_tools"
    );
    assert_eq!(
        McpRegistryListToolsTool::new(cfg()).permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(
        McpRegistryUninstallTool::new(cfg()).permission_level(),
        PermissionLevel::Write
    );
    assert_eq!(McpRegistrySearchTool::new(cfg()).scope(), ToolScope::All);
}

#[tokio::test]
async fn get_requires_qualified_name() {
    let err = McpRegistryGetTool::new(cfg())
        .execute(json!({}))
        .await
        .expect_err("missing qualified_name");
    assert!(err.to_string().contains("qualified_name"));
}

#[tokio::test]
async fn list_tools_requires_server_id() {
    let err = McpRegistryListToolsTool::new(cfg())
        .execute(json!({}))
        .await
        .expect_err("missing server_id");
    assert!(err.to_string().contains("server_id"));
}

#[tokio::test]
async fn list_tools_errors_for_unconnected_server() {
    // A server_id that is not installed errors rather than succeeding empty,
    // and says so rather than claiming it is merely not connected (#6313).
    let dir = tempfile::tempdir().expect("temp workspace");
    let config = workspace_config(dir.path());
    crate::mcp::module_client::McpConfiguredClient::install_test_port(
        &config,
        Box::new(ToolCallPort::default()),
    )
    .expect("test module port");
    let err = McpRegistryListToolsTool::new(Arc::clone(&config))
        .execute(json!({ "server_id": "definitely-not-connected-uuid" }))
        .await
        .expect_err("unconnected server must error");
    let err = err.to_string();
    assert!(err.contains("no installed MCP server"), "{err}");
    assert!(err.contains("mcp_registry_status"), "{err}");
}

/// The identity a registry tool presents to the model, captured from the
/// hand-written tools before their specs moved to `tinymcp_bus::agent_tools`.
///
/// Name, description and schema are prompt-cache and transcript identity: a
/// byte of drift invalidates every cached prefix and changes what a resumed
/// session replays. The schema is compared as serialized text, not as a
/// `Value`, so a key-order change is caught too.
struct Golden {
    tool: Box<dyn Tool>,
    name: &'static str,
    description: &'static str,
    schema: &'static str,
    permission: PermissionLevel,
    exposure: tinytools::ToolExposure,
    concurrency_safe: bool,
}

fn goldens() -> Vec<Golden> {
    vec![
        Golden {
            tool: Box::new(McpRegistrySearchTool::new(cfg())),
            name: r#"mcp_registry_search"#,
            description: r#"Search the MCP server registry catalog by `query`, optionally filtered by `transport` ("stdio" | "hosted" | "all"), paginated by `page` / `page_size`. Use to discover installable MCP servers."#,
            schema: r#"{"properties":{"page":{"minimum":1,"type":"integer"},"page_size":{"minimum":1,"type":"integer"},"query":{"type":"string"},"transport":{"enum":["stdio","hosted","all"],"type":"string"}},"type":"object"}"#,
            permission: PermissionLevel::ReadOnly,
            exposure: tinytools::ToolExposure::Deferred,
            concurrency_safe: true,
        },
        Golden {
            tool: Box::new(McpRegistryGetTool::new(cfg())),
            name: r#"mcp_registry_get"#,
            description: r#"Get one MCP registry server's detail by `qualified_name`."#,
            schema: r#"{"properties":{"qualified_name":{"type":"string"}},"required":["qualified_name"],"type":"object"}"#,
            permission: PermissionLevel::ReadOnly,
            exposure: tinytools::ToolExposure::Deferred,
            concurrency_safe: true,
        },
        Golden {
            tool: Box::new(McpRegistryInstalledListTool::new(cfg())),
            name: r#"mcp_registry_installed_list"#,
            description: r#"List the MCP servers currently installed for this user."#,
            schema: r#"{"properties":{},"type":"object"}"#,
            permission: PermissionLevel::ReadOnly,
            exposure: tinytools::ToolExposure::Deferred,
            concurrency_safe: true,
        },
        Golden {
            tool: Box::new(McpRegistryStatusTool::new(cfg())),
            name: r#"mcp_registry_status"#,
            description: r#"Report the connection status of installed MCP servers."#,
            schema: r#"{"properties":{},"type":"object"}"#,
            permission: PermissionLevel::ReadOnly,
            exposure: tinytools::ToolExposure::Direct,
            concurrency_safe: true,
        },
        Golden {
            tool: Box::new(McpRegistryListToolsTool::new(cfg())),
            name: r#"mcp_registry_list_tools"#,
            description: r#"List the tools (name, description, input schema) exposed by a connected MCP server, given its `server_id`. Use this to discover what a connected server can do before calling `mcp_registry_tool_call`. The server must already be connected (see `mcp_registry_status` / `mcp_registry_connect`)."#,
            schema: r#"{"properties":{"server_id":{"type":"string"}},"required":["server_id"],"type":"object"}"#,
            permission: PermissionLevel::ReadOnly,
            exposure: tinytools::ToolExposure::Direct,
            concurrency_safe: true,
        },
        Golden {
            tool: Box::new(McpRegistryConnectTool::new(cfg())),
            name: r#"mcp_registry_connect"#,
            description: r#"Connect (spawn + handshake) an installed MCP server by `server_id`, returning its tools."#,
            schema: r#"{"properties":{"server_id":{"type":"string"}},"required":["server_id"],"type":"object"}"#,
            permission: PermissionLevel::Execute,
            exposure: tinytools::ToolExposure::Direct,
            concurrency_safe: false,
        },
        Golden {
            tool: Box::new(McpRegistryDisconnectTool::new(cfg())),
            name: r#"mcp_registry_disconnect"#,
            description: r#"Disconnect (stop) a connected MCP server by `server_id`."#,
            schema: r#"{"properties":{"server_id":{"type":"string"}},"required":["server_id"],"type":"object"}"#,
            permission: PermissionLevel::Execute,
            exposure: tinytools::ToolExposure::Direct,
            concurrency_safe: false,
        },
        Golden {
            tool: Box::new(McpRegistryToolCallTool::new(cfg())),
            name: r#"mcp_registry_tool_call"#,
            description: r#"Invoke a tool on a connected MCP server: `server_id` + `tool_name` + `arguments` object."#,
            schema: r#"{"properties":{"arguments":{"type":"object"},"server_id":{"type":"string"},"tool_name":{"type":"string"}},"required":["server_id","tool_name"],"type":"object"}"#,
            permission: PermissionLevel::Execute,
            exposure: tinytools::ToolExposure::Direct,
            concurrency_safe: false,
        },
        Golden {
            tool: Box::new(McpRegistryUninstallTool::new(cfg())),
            name: r#"mcp_registry_uninstall"#,
            description: r#"Uninstall an installed MCP server by `server_id`. Default-OFF (opt-in)."#,
            schema: r#"{"properties":{"server_id":{"type":"string"}},"required":["server_id"],"type":"object"}"#,
            permission: PermissionLevel::Write,
            exposure: tinytools::ToolExposure::Direct,
            concurrency_safe: false,
        },
    ]
}

#[test]
fn registry_tools_present_the_exact_identity_they_always_had() {
    for golden in goldens() {
        let tool = &golden.tool;
        assert_eq!(tool.name(), golden.name);
        assert_eq!(tool.description(), golden.description, "{}", golden.name);
        assert_eq!(
            serde_json::to_string(&tool.parameters_schema()).unwrap(),
            golden.schema,
            "{}",
            golden.name
        );
        assert_eq!(
            tool.permission_level(),
            golden.permission,
            "{}",
            golden.name
        );
        assert_eq!(tool.exposure(), golden.exposure, "{}", golden.name);
        assert_eq!(
            tool.is_concurrency_safe(&json!({})),
            golden.concurrency_safe,
            "{}",
            golden.name
        );
        assert!(!tool.external_effect(), "{}", golden.name);
        assert_eq!(tool.family(), None, "{}", golden.name);
    }
}

#[tokio::test]
async fn tool_call_refuses_arguments_that_are_not_an_object() {
    let dir = tempfile::tempdir().expect("temp workspace");
    let config = workspace_config(dir.path());
    crate::mcp::module_client::McpConfiguredClient::install_test_port(
        &config,
        Box::new(ToolCallPort::default()),
    )
    .expect("test module port");
    let err = McpRegistryToolCallTool::new(config)
        .execute(json!({ "server_id": "srv", "tool_name": "t", "arguments": 5 }))
        .await
        .expect_err("a number is not arguments");
    let message = err.to_string();
    assert!(message.contains("mcp_registry_tool_call"), "{message}");
    assert!(message.contains("invalid tool arguments"), "{message}");
    assert!(
        !message.contains('5'),
        "module failure must not echo arguments: {message}"
    );
}

/// A workspace of its own, so the host this opens is not shared.
fn workspace_config(dir: &std::path::Path) -> Arc<Config> {
    Arc::new(Config {
        workspace_dir: dir.join("workspace"),
        action_dir: dir.join("workspace"),
        config_path: dir.join("config.toml"),
        ..Default::default()
    })
}

#[derive(Clone, Default)]
struct ToolCallPort {
    arguments: SharedArc<Mutex<Vec<Value>>>,
    normalized_arguments: SharedArc<Mutex<Vec<Option<Value>>>>,
    connected: bool,
    tool_meta: Option<Value>,
}

#[async_trait]
impl crate::mcp::module_client::McpModulePort for ToolCallPort {
    async fn open_configured(
        &self,
        _data_dir: &str,
        _config: &tinymcp_bus::McpClientConfig,
    ) -> Result<tinymcp_bus::OpenConfiguredResponse, tinybus::module::ModuleCallError> {
        Ok(tinymcp_bus::OpenConfiguredResponse {
            object_path: "/test/mcp".to_owned(),
            transition_id: "open-test".to_owned(),
        })
    }

    async fn close(
        &self,
        _data_dir: &str,
    ) -> Result<Option<String>, tinybus::module::ModuleCallError> {
        Ok(None)
    }

    async fn transition_status(
        &self,
        _transition_id: &str,
    ) -> Result<tinymcp_bus::TransitionStatus, tinybus::module::ModuleCallError> {
        Ok(tinymcp_bus::TransitionStatus::Complete)
    }

    async fn call_object(
        &self,
        _object_path: &str,
        member: &str,
        args: Value,
        _confidential: bool,
    ) -> Result<Value, tinybus::module::ModuleCallError> {
        match member {
            tinymcp_bus::names::methods::STATUS => {
                let statuses = self.connected.then(|| tinymcp_bus::ConnStatus {
                    server_id: "srv-1".to_owned(),
                    qualified_name: "loopback".to_owned(),
                    display_name: "loopback".to_owned(),
                    status: tinymcp_bus::ServerStatus::Connected,
                    tool_count: 1,
                    last_error: None,
                    auth_hint: None,
                });
                serde_json::to_value(statuses.into_iter().collect::<Vec<_>>())
                    .map_err(|_| tinybus::module::ModuleCallError::ModuleFault)
            }
            tinymcp_bus::names::methods::LIST_TOOLS => {
                if self.connected {
                    let mut forecast = tinymcp_bus::McpTool::new("forecast");
                    forecast.meta = self.tool_meta.clone();
                    serde_json::to_value(vec![forecast])
                        .map_err(|_| tinybus::module::ModuleCallError::ModuleFault)
                } else {
                    Err(tinybus::module::ModuleCallError::ModuleFault)
                }
            }
            tinymcp_bus::names::methods::CONNECTED_OVERVIEW => {
                let overview = self
                    .connected
                    .then(|| tinymcp_bus::ConnectedServerOverview {
                        server_id: "srv-1".to_owned(),
                        qualified_name: "loopback".to_owned(),
                        display_name: "loopback".to_owned(),
                        description: None,
                        instructions: None,
                        tools: vec![tinymcp_bus::McpTool::new("forecast")],
                    });
                serde_json::to_value(overview.into_iter().collect::<Vec<_>>())
                    .map_err(|_| tinybus::module::ModuleCallError::ModuleFault)
            }
            tinymcp_bus::names::methods::TOOL_CALL => {
                let arguments = args
                    .get(2)
                    .cloned()
                    .ok_or(tinybus::module::ModuleCallError::ModuleFault)?;
                self.arguments.lock().unwrap().push(arguments);
                serde_json::to_value(tinymcp_bus::ToolCallOutcome {
                    result: json!({"content":[{"type":"text","text":"sunny"}]}),
                    is_error: false,
                    rendered: tinymcp_bus::McpToolResult::success("sunny"),
                })
                .map_err(|_| tinybus::module::ModuleCallError::ModuleFault)
            }
            _ => Err(tinybus::module::ModuleCallError::IncompatibleContract),
        }
    }

    async fn validate_server_arguments(
        &self,
        _request: tinymcp_bus::ValidateServerArgumentsRequest,
    ) -> Result<tinymcp_bus::ServerArgumentsValidation, tinybus::module::ModuleCallError> {
        Err(tinybus::module::ModuleCallError::IncompatibleContract)
    }

    async fn normalize_tool_arguments(
        &self,
        arguments: Option<Value>,
    ) -> Result<serde_json::Map<String, Value>, tinybus::module::ModuleCallError> {
        self.normalized_arguments
            .lock()
            .unwrap()
            .push(arguments.clone());
        match arguments.unwrap_or(Value::Null) {
            Value::Null => Ok(serde_json::Map::new()),
            Value::Object(arguments) => Ok(arguments),
            Value::String(encoded) => serde_json::from_str::<Value>(&encoded)
                .ok()
                .and_then(|value| value.as_object().cloned())
                .ok_or(tinybus::module::ModuleCallError::ModuleFault),
            _ => Err(tinybus::module::ModuleCallError::ModuleFault),
        }
    }

    async fn prepare_agent_tools(
        &self,
        _request: tinymcp_bus::PrepareAgentToolsRequest,
    ) -> Result<Vec<tinymcp_bus::McpToolPresentation>, tinybus::module::ModuleCallError> {
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn tool_call_with_string_arguments_reaches_the_module_as_an_object() {
    // A model can call `mcp_registry_tool_call` with
    // `"arguments": "{...}"`, a JSON-encoded string where MCP needs an object.
    let dir = tempfile::tempdir().expect("temp workspace");
    let config = workspace_config(dir.path());
    let port = ToolCallPort {
        connected: true,
        ..ToolCallPort::default()
    };
    let arguments = SharedArc::clone(&port.arguments);
    crate::mcp::module_client::McpConfiguredClient::install_test_port(&config, Box::new(port))
        .expect("test module port");

    let result = McpRegistryToolCallTool::new(Arc::clone(&config))
        .execute(json!({
            "server_id": "srv-1",
            "tool_name": "forecast",
            "arguments": "{\"city\":\"Paris\"}"
        }))
        .await
        .expect("call");
    assert!(!result.is_error, "{}", result.text());

    assert_eq!(
        arguments.lock().unwrap().as_slice(),
        &[json!({ "city": "Paris" })]
    );
}

#[tokio::test]
async fn tool_call_preserves_missing_arguments_as_distinct_from_json_null() {
    let dir = tempfile::tempdir().expect("temp workspace");
    let config = workspace_config(dir.path());
    let port = ToolCallPort {
        connected: true,
        ..ToolCallPort::default()
    };
    let arguments = SharedArc::clone(&port.arguments);
    let normalized_arguments = SharedArc::clone(&port.normalized_arguments);
    crate::mcp::module_client::McpConfiguredClient::install_test_port(&config, Box::new(port))
        .expect("test module port");
    let tool = McpRegistryToolCallTool::new(Arc::clone(&config));

    for args in [
        json!({"server_id":"srv-1", "tool_name":"forecast"}),
        json!({"server_id":"srv-1", "tool_name":"forecast", "arguments":null}),
    ] {
        tool.execute(args)
            .await
            .expect("missing and null arguments normalize successfully");
    }

    assert_eq!(
        normalized_arguments.lock().unwrap().as_slice(),
        &[None, Some(Value::Null)],
        "the host must preserve the bus distinction between omitted arguments and JSON null"
    );
    assert_eq!(
        arguments.lock().unwrap().as_slice(),
        &[json!({}), json!({})]
    );
}

#[tokio::test]
async fn registry_tool_call_attaches_the_descriptor_widget() {
    let dir = tempfile::tempdir().expect("temp workspace");
    let config = workspace_config(dir.path());
    let port = ToolCallPort {
        connected: true,
        tool_meta: Some(json!({"ui":{"resourceUri":"ui://widget/forecast.html"}})),
        ..ToolCallPort::default()
    };
    crate::mcp::module_client::McpConfiguredClient::install_test_port(&config, Box::new(port))
        .expect("test module port");

    let result = McpRegistryToolCallTool::new(Arc::clone(&config))
        .execute(json!({"server_id":"srv-1", "tool_name":"forecast", "arguments":{}}))
        .await
        .expect("call");

    assert!(!result.is_error, "{}", result.text());
    let metadata = result.metadata.expect("widget presentation");
    assert_eq!(metadata["kind"], "mcp_ui");
    assert_eq!(metadata["resource_uri"], "ui://widget/forecast.html");
    assert_eq!(metadata["server_id"], "srv-1");
    assert_eq!(metadata["tool"], "forecast");
}

#[tokio::test]
async fn registry_tool_call_without_a_descriptor_widget_has_no_presentation() {
    let dir = tempfile::tempdir().expect("temp workspace");
    let config = workspace_config(dir.path());
    let port = ToolCallPort {
        connected: true,
        ..ToolCallPort::default()
    };
    crate::mcp::module_client::McpConfiguredClient::install_test_port(&config, Box::new(port))
        .expect("test module port");

    let result = McpRegistryToolCallTool::new(Arc::clone(&config))
        .execute(json!({"server_id":"srv-1", "tool_name":"forecast", "arguments":{}}))
        .await
        .expect("call");

    assert!(!result.is_error, "{}", result.text());
    assert!(result.metadata.is_none());
}
