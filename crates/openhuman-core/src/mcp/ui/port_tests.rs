use super::*;
use serde_json::json;

fn port() -> (tempfile::TempDir, HostUiPort) {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    (workspace, HostUiPort::new(Arc::new(config)))
}

#[tokio::test]
async fn an_unknown_server_is_unavailable() {
    let (_workspace, port) = port();
    assert!(!port.is_available("nowhere").await);
}

#[tokio::test]
async fn an_unknown_server_has_no_tool_descriptor() {
    let (_workspace, port) = port();
    assert!(port.tool_descriptor("nowhere", "order").await.is_none());
}

#[tokio::test]
async fn reading_from_an_unknown_server_fails() {
    let (_workspace, port) = port();
    assert!(port.read_resource("nowhere", "ui://x").await.is_err());
}

#[tokio::test]
async fn calling_a_tool_on_an_unknown_server_fails() {
    let (_workspace, port) = port();
    assert!(port.call_tool("nowhere", "order", json!({})).await.is_err());
}

mod module_backed {
    use super::*;
    use std::sync::Mutex;

    use crate::mcp::module_client::{McpConfiguredClient, McpModulePort};
    use tinybus::module::ModuleCallError;
    use tinymcp_bus::{
        agent_tools::{McpToolPresentation, PrepareAgentToolsRequest},
        config::{McpClientConfig, OpenConfiguredResponse, TransitionStatus},
        names, ServerArgumentsValidation, ValidateServerArgumentsRequest,
    };

    #[derive(Default)]
    struct ModulePort {
        installed: Vec<&'static str>,
        statics: Vec<&'static str>,
        fail_reads: bool,
        members: Mutex<Vec<String>>,
    }

    impl ModulePort {
        fn called(&self) -> Vec<String> {
            self.members.lock().unwrap().clone()
        }
    }

    fn resource(uri: &str) -> tinymcp_bus::McpResourceContents {
        tinymcp_bus::McpResourceContents {
            uri: uri.to_string(),
            mime_type: Some("text/html;profile=mcp-app".to_string()),
            text: Some("<p>card</p>".to_string()),
            blob: None,
            meta: Some(json!({"ui": {"prefersBorder": true}})),
        }
    }

    #[async_trait]
    impl McpModulePort for std::sync::Arc<ModulePort> {
        async fn open_configured(
            &self,
            _data_dir: &str,
            _config: &McpClientConfig,
        ) -> Result<OpenConfiguredResponse, ModuleCallError> {
            Ok(OpenConfiguredResponse {
                object_path: "/workspace/1".to_string(),
                transition_id: "open-1".to_string(),
            })
        }

        async fn close(&self, _data_dir: &str) -> Result<Option<String>, ModuleCallError> {
            Ok(None)
        }

        async fn transition_status(
            &self,
            _transition_id: &str,
        ) -> Result<TransitionStatus, ModuleCallError> {
            Ok(TransitionStatus::Complete)
        }

        async fn call_object(
            &self,
            _object_path: &str,
            member: &str,
            args: Value,
            _confidential: bool,
        ) -> Result<Value, ModuleCallError> {
            self.members.lock().unwrap().push(member.to_string());
            let uri = args
                .get(1)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            match member {
                names::methods::STATUS => Ok(serde_json::to_value(
                    self.installed
                        .iter()
                        .map(|id| tinymcp_bus::ConnStatus {
                            server_id: (*id).to_string(),
                            qualified_name: (*id).to_string(),
                            display_name: (*id).to_string(),
                            status: tinymcp_bus::ServerStatus::Connected,
                            tool_count: 1,
                            last_error: None,
                            auth_hint: None,
                        })
                        .collect::<Vec<_>>(),
                )
                .unwrap()),
                names::methods::STATIC_LIST => Ok(json!(self.statics)),
                names::methods::LIST_TOOLS => Ok(json!([{
                    "name": "show",
                    "description": "Shows a card",
                    "input_schema": {},
                    "_meta": {"ui": {"resourceUri": "ui://card"}}
                }])),
                names::methods::READ_RESOURCE | names::methods::STATIC_READ_RESOURCE
                    if self.fail_reads =>
                {
                    Err(ModuleCallError::ModuleFault)
                }
                names::methods::READ_RESOURCE | names::methods::STATIC_READ_RESOURCE => {
                    Ok(serde_json::to_value(vec![resource(&uri)]).unwrap())
                }
                _ => Err(ModuleCallError::ModuleFault),
            }
        }

        async fn prepare_agent_tools(
            &self,
            _request: PrepareAgentToolsRequest,
        ) -> Result<Vec<McpToolPresentation>, ModuleCallError> {
            Ok(Vec::new())
        }

        async fn validate_server_arguments(
            &self,
            _request: ValidateServerArgumentsRequest,
        ) -> Result<ServerArgumentsValidation, ModuleCallError> {
            Err(ModuleCallError::ModuleFault)
        }
    }

    fn port_over(fake: &Arc<ModulePort>) -> HostUiPort {
        let client = McpConfiguredClient::with_test_port(
            "/private/workspace".to_string(),
            Box::new(Arc::clone(fake)),
        );
        HostUiPort::with_client(Arc::new(Config::default()), Arc::new(client))
    }

    #[tokio::test]
    async fn an_installed_servers_resource_is_read_through_the_module() {
        let fake = Arc::new(ModulePort {
            installed: vec!["srv-1"],
            ..ModulePort::default()
        });
        let contents = port_over(&fake)
            .read_resource("srv-1", "ui://card")
            .await
            .expect("contents");
        assert_eq!(
            contents,
            vec![json!({
                "uri": "ui://card",
                "mimeType": "text/html;profile=mcp-app",
                "text": "<p>card</p>",
                "_meta": {"ui": {"prefersBorder": true}}
            })]
        );
        let called = fake.called();
        assert!(called.contains(&names::methods::READ_RESOURCE.to_string()));
        assert!(!called.contains(&names::methods::STATIC_READ_RESOURCE.to_string()));
    }

    #[tokio::test]
    async fn a_static_servers_resource_is_read_through_the_static_member() {
        let fake = Arc::new(ModulePort {
            statics: vec!["docs"],
            ..ModulePort::default()
        });
        let contents = port_over(&fake)
            .read_resource("docs", "ui://card")
            .await
            .expect("contents");
        assert_eq!(contents[0]["text"], json!("<p>card</p>"));
        let called = fake.called();
        assert!(called.contains(&names::methods::STATIC_READ_RESOURCE.to_string()));
        assert!(!called.contains(&names::methods::READ_RESOURCE.to_string()));
    }

    #[tokio::test]
    async fn a_failed_resource_read_reports_a_generic_error() {
        let fake = Arc::new(ModulePort {
            installed: vec!["srv-1"],
            fail_reads: true,
            ..ModulePort::default()
        });
        let error = port_over(&fake)
            .read_resource("srv-1", "ui://card")
            .await
            .expect_err("read must fail");
        assert_eq!(error, "MCP resource read failed");
    }

    #[tokio::test]
    async fn an_installed_tool_descriptor_carries_the_tools_meta() {
        let fake = Arc::new(ModulePort {
            installed: vec!["srv-1"],
            ..ModulePort::default()
        });
        let descriptor = port_over(&fake)
            .tool_descriptor("srv-1", "show")
            .await
            .expect("descriptor");
        assert_eq!(
            descriptor.meta,
            Some(json!({"ui": {"resourceUri": "ui://card"}}))
        );
    }

    #[tokio::test]
    async fn a_tool_the_server_does_not_list_has_no_descriptor() {
        let fake = Arc::new(ModulePort {
            installed: vec!["srv-1"],
            ..ModulePort::default()
        });
        assert!(port_over(&fake)
            .tool_descriptor("srv-1", "missing")
            .await
            .is_none());
    }
}
