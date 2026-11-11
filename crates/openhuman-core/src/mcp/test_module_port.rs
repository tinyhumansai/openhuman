//! In-memory TinyMCP port for host-side tests.
//!
//! These tests exercise OpenHuman's validation, dispatch, and audit behavior;
//! module behavior is covered in the TinyMCP repository.

use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::config::Config;
use crate::mcp::module_client::{McpConfiguredClient, McpModulePort};
use tinybus::module::ModuleCallError;
use tinymcp_bus::{
    McpToolPresentation, McpWriteRecord, NewMcpWriteRecord, OpenConfiguredResponse,
    PrepareAgentToolsRequest, ServerArgumentRule, ServerArgumentsValidation, TransitionStatus,
    ValidateServerArgumentsRequest,
};

#[derive(Default)]
struct Port {
    writes: Mutex<Vec<McpWriteRecord>>,
}

/// Install a TinyMCP test port for the workspace in `config`.
pub(crate) fn install(config: &Config) {
    McpConfiguredClient::install_test_port(config, Box::new(Port::default()))
        .expect("install the TinyMCP test port");
}

#[async_trait]
impl McpModulePort for Port {
    async fn open_configured(
        &self,
        _data_dir: &str,
        _config: &tinymcp_bus::McpClientConfig,
    ) -> Result<OpenConfiguredResponse, ModuleCallError> {
        Ok(OpenConfiguredResponse {
            object_path: "/test/tinymcp".to_owned(),
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
            tinymcp_bus::names::methods::AUDIT_RECORD_WRITE => {
                let record: NewMcpWriteRecord = tuple_arg(&args)?;
                let mut writes = self.writes.lock().unwrap();
                let id = writes.len() as i64 + 1;
                writes.push(McpWriteRecord {
                    id,
                    timestamp_ms: record.timestamp_ms,
                    client_info: record.client_info,
                    tool_name: record.tool_name,
                    args_summary: record.args_summary,
                    resulting_chunk_id: record.resulting_chunk_id,
                    success: record.success,
                    error_message: record.error_message,
                });
                Ok(json!(id))
            }
            tinymcp_bus::names::methods::AUDIT_LIST_WRITES => {
                serde_json::to_value(self.writes.lock().unwrap().clone())
                    .map_err(|_| ModuleCallError::ModuleFault)
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
        request: ValidateServerArgumentsRequest,
    ) -> Result<ServerArgumentsValidation, ModuleCallError> {
        let Some(args) = request.arguments.as_object().cloned() else {
            return Ok(invalid("tool arguments must be an object"));
        };
        if args
            .keys()
            .any(|key| !request.allowed.iter().any(|allowed| allowed == key))
        {
            return Ok(invalid("unexpected filter field"));
        }
        for rule in request.rules {
            match rule {
                ServerArgumentRule::RequiredNonEmptyString { key } => {
                    let valid = args
                        .get(&key)
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.trim().is_empty());
                    if !valid {
                        return Ok(invalid(&format!("missing required argument `{key}`")));
                    }
                }
                ServerArgumentRule::RequiredNonEmptyStringArray { key } => {
                    let valid = args
                        .get(&key)
                        .and_then(Value::as_array)
                        .is_some_and(|values| {
                            !values.is_empty()
                                && values.iter().all(|value| {
                                    value.as_str().is_some_and(|text| !text.trim().is_empty())
                                })
                        });
                    if !valid {
                        return Ok(invalid(&format!(
                            "`{key}` must be a non-empty string array"
                        )));
                    }
                }
                ServerArgumentRule::OptionalNonEmptyString { key } => {
                    if args.get(&key).is_some_and(|value| {
                        !value.is_null() && value.as_str().is_none_or(|text| text.trim().is_empty())
                    }) {
                        return Ok(invalid(&format!("`{key}` must be a non-empty string")));
                    }
                }
                ServerArgumentRule::OptionalPositiveU64 { key, max } => {
                    if args.get(&key).is_some_and(|value| {
                        !value.is_null()
                            && value
                                .as_u64()
                                .is_none_or(|number| number == 0 || number > max)
                    }) {
                        return Ok(invalid(&format!("`{key}` must be between 1 and {max}")));
                    }
                }
            }
        }
        Ok(ServerArgumentsValidation::Valid(args))
    }
}

fn tuple_arg<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, ModuleCallError> {
    args.as_array()
        .and_then(|values| values.first())
        .cloned()
        .ok_or(ModuleCallError::ModuleFault)
        .and_then(|value| serde_json::from_value(value).map_err(|_| ModuleCallError::ModuleFault))
}

fn invalid(message: &str) -> ServerArgumentsValidation {
    ServerArgumentsValidation::Invalid(tinymcp_bus::ToolCallError::InvalidParams(
        message.to_owned(),
    ))
}
