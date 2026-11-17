use super::*;
use crate::mcp::module_client::{McpConfiguredClient, McpModulePort};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tinybus::module::ModuleCallError;
use tinymcp_bus::{
    OpenConfiguredResponse, ServerArgumentsValidation, TransitionStatus,
    ValidateServerArgumentsRequest,
};

#[derive(Clone, Default)]
struct ConfigDocumentPort {
    calls: Arc<Mutex<Vec<(String, Value, bool)>>>,
}

#[async_trait]
impl McpModulePort for ConfigDocumentPort {
    async fn open_configured(
        &self,
        _data_dir: &str,
        _config: &tinymcp_bus::McpClientConfig,
    ) -> Result<OpenConfiguredResponse, ModuleCallError> {
        Ok(OpenConfiguredResponse {
            object_path: "/test/mcp".to_owned(),
            transition_id: "open-test".to_owned(),
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
        confidential: bool,
    ) -> Result<Value, ModuleCallError> {
        self.calls
            .lock()
            .unwrap()
            .push((member.to_owned(), args, confidential));
        match member {
            tinymcp_bus::names::methods::CONFIG_DOCUMENT_GET => {
                Ok(serde_json::json!({"mcpServers": {"local": {"command": "mcp"}}}))
            }
            tinymcp_bus::names::methods::CONFIG_DOCUMENT_APPLY => {
                Ok(serde_json::to_value(tinymcp_bus::ConfigApplyResponse {
                    document: serde_json::json!({"mcpServers": {}}),
                    report: tinymcp_bus::ConfigApplyReport {
                        installed: vec![tinymcp_bus::AppliedServer {
                            server_id: "server-1".to_owned(),
                            name: "local".to_owned(),
                        }],
                        ..Default::default()
                    },
                })
                .unwrap())
            }
            _ => Err(ModuleCallError::Unavailable),
        }
    }

    async fn prepare_agent_tools(
        &self,
        _: tinymcp_bus::PrepareAgentToolsRequest,
    ) -> Result<Vec<tinymcp_bus::McpToolPresentation>, ModuleCallError> {
        Ok(Vec::new())
    }

    async fn validate_server_arguments(
        &self,
        _: ValidateServerArgumentsRequest,
    ) -> Result<ServerArgumentsValidation, ModuleCallError> {
        std::future::ready(Err(ModuleCallError::IncompatibleContract)).await
    }
}

fn config(workspace: &tempfile::TempDir) -> Config {
    Config {
        workspace_dir: workspace.path().join("workspace"),
        action_dir: workspace.path().join("workspace"),
        config_path: workspace.path().join("config.toml"),
        ..Default::default()
    }
}

#[tokio::test]
async fn config_document_handlers_use_the_configured_module_and_confidential_apply() {
    let workspace = tempfile::tempdir().unwrap();
    let config = config(&workspace);
    let port = ConfigDocumentPort::default();
    let calls = Arc::clone(&port.calls);
    McpConfiguredClient::install_test_port(&config, Box::new(port)).unwrap();

    let get = mcp_clients_config_get(&config).await.unwrap();
    assert_eq!(get.value["mcpServers"]["local"]["command"], "mcp");
    assert_eq!(get.logs, ["config_get rendered 1 servers"]);

    let requested = serde_json::json!({"mcpServers": {"local": {"command": "mcp"}}});
    let set = mcp_clients_config_set(&config, requested.clone())
        .await
        .unwrap();
    assert_eq!(set.value["added"], serde_json::json!(["local"]));
    assert_eq!(set.value["mcpServers"], serde_json::json!({}));

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, tinymcp_bus::names::methods::CONFIG_DOCUMENT_GET);
    assert!(!calls[0].2);
    assert_eq!(
        calls[1].0,
        tinymcp_bus::names::methods::CONFIG_DOCUMENT_APPLY
    );
    assert_eq!(calls[1].1, serde_json::json!([requested]));
    assert!(calls[1].2);
}
