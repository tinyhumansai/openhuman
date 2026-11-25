//! Unit tests for the MCP clients RPC handlers.
//!
//! The operations themselves are covered in `tinymcp`. What this layer adds is
//! the RPC shape, which the end-to-end suites exercise against a live process,
//! and how a refusal reads to the caller, which a host over a temp workspace
//! can pin without connecting anything.

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

// ── list_tools refusals (#6313) ─────────────────────────────────────────────
//
// A per-workspace fake module port keeps these handler tests independent of a
// live module process while exercising the production TinyBus adapter.

const QUALIFIED: &str = "io.github.ryudi84/uuid";

fn temp_config(workspace: &tempfile::TempDir) -> Config {
    Config {
        workspace_dir: workspace.path().join("workspace"),
        action_dir: workspace.path().join("workspace"),
        config_path: workspace.path().join("config.toml"),
        ..Default::default()
    }
}

#[derive(Clone, Default)]
struct RegistryPort {
    statuses: Arc<Mutex<Vec<tinymcp_bus::ConnStatus>>>,
}

#[async_trait]
impl McpModulePort for RegistryPort {
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
        _confidential: bool,
    ) -> Result<Value, ModuleCallError> {
        match member {
            tinymcp_bus::names::methods::STATUS => {
                serde_json::to_value(self.statuses.lock().unwrap().clone())
                    .map_err(|_| ModuleCallError::ModuleFault)
            }
            tinymcp_bus::names::methods::CONNECT => {
                let server_id = args
                    .get(0)
                    .and_then(Value::as_str)
                    .ok_or(ModuleCallError::ModuleFault)?;
                let mut statuses = self.statuses.lock().unwrap();
                let status = statuses
                    .iter_mut()
                    .find(|status| status.server_id == server_id)
                    .ok_or(ModuleCallError::ModuleFault)?;
                status.status = tinymcp_bus::ServerStatus::Connected;
                status.tool_count = 1;
                let outcome = tinymcp_bus::ConnectOutcome {
                    server_id: server_id.to_owned(),
                    tools: vec![tinymcp_bus::McpTool::new("forecast")],
                };
                serde_json::to_value(outcome).map_err(|_| ModuleCallError::ModuleFault)
            }
            tinymcp_bus::names::methods::LIST_TOOLS => {
                let server_id = args
                    .get(0)
                    .and_then(Value::as_str)
                    .ok_or(ModuleCallError::ModuleFault)?;
                let statuses = self.statuses.lock().unwrap();
                let connected = statuses.iter().any(|status| {
                    status.server_id == server_id
                        && status.status == tinymcp_bus::ServerStatus::Connected
                });
                if connected {
                    serde_json::to_value(vec![tinymcp_bus::McpTool::new("forecast")])
                        .map_err(|_| ModuleCallError::ModuleFault)
                } else {
                    Err(ModuleCallError::ModuleFault)
                }
            }
            _ => Err(ModuleCallError::IncompatibleContract),
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

fn status(server_id: &str, qualified_name: &str) -> tinymcp_bus::ConnStatus {
    tinymcp_bus::ConnStatus {
        server_id: server_id.to_owned(),
        qualified_name: qualified_name.to_owned(),
        display_name: "fixture".to_owned(),
        status: tinymcp_bus::ServerStatus::Disconnected,
        tool_count: 0,
        last_error: None,
        auth_hint: None,
    }
}

fn install_port(config: &Config, statuses: Vec<tinymcp_bus::ConnStatus>) {
    McpConfiguredClient::install_test_port(
        config,
        Box::new(RegistryPort {
            statuses: Arc::new(Mutex::new(statuses)),
        }),
    )
    .unwrap();
}

#[tokio::test]
async fn agent_refusal_names_no_rpc_method() {
    let workspace = tempfile::tempdir().unwrap();
    let config = temp_config(&workspace);
    let server_id = "fixture-install";
    install_port(&config, vec![status(server_id, QUALIFIED)]);

    let error = mcp_clients_list_tools(&config, server_id.to_owned(), Caller::Agent)
        .await
        .expect_err("a disconnected server has no tools to list");
    assert!(!error.contains("mcp_clients_"), "{error}");
    assert!(
        error.starts_with(&format!(
            "server_id={server_id} is disconnected; it has to be connected"
        )),
        "{error}"
    );
}

/// Every tool an agent refusal names must be callable by every agent that can
/// receive it, i.e. every built-in agent with `mcp_registry_list_tools` on its
/// belt. The orchestrator's refusal messages name only tools it can call.
#[test]
fn refusals_name_only_tools_on_every_listing_belt() {
    let install = tinymcp_bus::ConnStatus {
        server_id: "srv-1".into(),
        qualified_name: QUALIFIED.into(),
        display_name: "uuid".into(),
        status: tinymcp_bus::ServerStatus::Disconnected,
        tool_count: 0,
        last_error: None,
        auth_hint: None,
    };
    let refusals: Vec<String> = ["srv-1", "io.github.other/absent"]
        .into_iter()
        .map(|requested| {
            match explain_not_connected(
                requested,
                std::slice::from_ref(&install),
                Caller::Agent.remedy(),
            ) {
                NotConnectedExplanation::Refused(message) => message,
                other => panic!("{requested} was not refused: {other:?}"),
            }
        })
        .collect();
    let named: Vec<&str> = refusals
        .iter()
        .flat_map(|message| message.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')))
        .filter(|word| word.starts_with("mcp_"))
        .collect();
    assert!(
        !named.is_empty(),
        "the unknown-id refusal names the status tool"
    );

    let belts: Vec<_> = crate::agent_registry::agents::load_builtins()
        .expect("built-ins load")
        .into_iter()
        .filter_map(|def| {
            let tinyagents_definition::ToolScope::Named(tools) = &def.tools else {
                return None;
            };
            let belt: Vec<String> = tools.iter().chain(&def.deferred_tools).cloned().collect();
            belt.iter()
                .any(|tool| tool == "mcp_registry_list_tools")
                .then(|| (def.id.clone(), belt))
        })
        .collect();
    assert!(
        belts.iter().any(|(id, _)| id == "orchestrator"),
        "fixture: the orchestrator lists MCP tools, got {:?}",
        belts.iter().map(|(id, _)| id).collect::<Vec<_>>()
    );
    for (id, belt) in &belts {
        for tool in &named {
            assert!(
                belt.iter().any(|t| t == tool),
                "{id} can receive a refusal naming `{tool}`, which is not on its belt"
            );
        }
    }
}

#[tokio::test]
async fn rpc_refusal_still_names_the_rpc_method() {
    let workspace = tempfile::tempdir().unwrap();
    let config = temp_config(&workspace);
    let server_id = "fixture-install";
    install_port(&config, vec![status(server_id, QUALIFIED)]);

    let error = mcp_clients_list_tools(&config, server_id.to_owned(), Caller::Rpc)
        .await
        .expect_err("a disconnected server has no tools to list");
    assert!(error.contains("mcp_clients_connect"), "{error}");
    assert!(!error.contains("mcp_registry_connect"), "{error}");
}

#[tokio::test]
async fn a_qualified_name_resolves_to_its_install() {
    let workspace = tempfile::tempdir().unwrap();
    let config = temp_config(&workspace);
    let server_id = "fixture-install";
    install_port(&config, vec![status(server_id, QUALIFIED)]);

    // Resolved, then refused for the install it names — in terms of that
    // install's server_id, not the registry name the caller passed.
    let error = mcp_clients_list_tools(&config, QUALIFIED.to_string(), Caller::Agent)
        .await
        .expect_err("the resolved server is not connected");
    assert!(
        error.starts_with(&format!("server_id={server_id} is disconnected")),
        "{error}"
    );
}

#[tokio::test]
async fn an_uninstalled_id_is_not_reported_as_not_connected() {
    let workspace = tempfile::tempdir().unwrap();
    let config = temp_config(&workspace);
    install_port(&config, vec![]);

    let error = mcp_clients_list_tools(&config, "io.github.other/absent".into(), Caller::Agent)
        .await
        .expect_err("nothing is installed under that name");
    assert!(!error.contains("not connected"), "{error}");
    assert!(error.contains("no installed MCP server"), "{error}");
    assert!(error.contains("mcp_registry_status"), "{error}");
}

/// The #6313 case itself: the module owns the connection and answers a
/// qualified-name lookup with the connected install's tools.
#[tokio::test]
async fn a_qualified_name_lists_the_tools_of_its_connected_install() {
    let workspace = tempfile::tempdir().unwrap();
    let config = temp_config(&workspace);
    let server_id = "fixture-install";
    install_port(&config, vec![status(server_id, QUALIFIED)]);
    mcp_clients_connect(&config, server_id.to_owned())
        .await
        .unwrap();

    let listed = mcp_clients_list_tools(&config, QUALIFIED.to_string(), Caller::Agent)
        .await
        .expect("a connected server's registry name lists its tools");
    assert_eq!(listed.value["server_id"], json!(server_id));
    assert!(
        !listed.value["tools"].as_array().unwrap().is_empty(),
        "{}",
        listed.value
    );
}
