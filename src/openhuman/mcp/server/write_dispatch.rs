//! Write-dispatch and audit pipeline for MCP write tools.
//!
//! Extracted from `tools.rs` to keep the write/audit concern in a focused
//! module. The public surface is consumed only by `tools::call_tool` and
//! the protocol layer.

use serde_json::{json, Map, Value};

use crate::core::all;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::mcp::audit::{self, NewMcpWriteRecord};
use crate::openhuman::security::{SecurityPolicy, ToolOperation};

use super::tools::{tool_error, tool_success, ToolCallError};

pub(super) async fn load_write_config(tool_name: &str) -> Result<Config, ToolCallError> {
    match config_rpc::load_config_with_timeout().await {
        Ok(config) => Ok(config),
        Err(err) => {
            log::warn!(
                "[mcp_server] enforce_write_policy config load failed tool={tool_name} error={err}"
            );
            Err(ToolCallError::Internal(format!(
                "failed to load config: {err}"
            )))
        }
    }
}

pub(super) fn enforce_write_policy_for_config(
    tool_name: &str,
    config: &Config,
) -> Result<(), ToolCallError> {
    let policy =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    match policy.enforce_tool_operation(ToolOperation::Act, tool_name) {
        Ok(()) => Ok(()),
        Err(message) => {
            log::debug!(
                "[mcp_server] enforce_write_policy denied tool={} decision={}",
                tool_name,
                message
            );
            Err(ToolCallError::InvalidParams(message))
        }
    }
}

/// Dispatch a write tool to its underlying RPC method with provenance and
/// audit logging.
pub(super) async fn dispatch_write_tool(
    tool_name: &str,
    params: &Map<String, Value>,
    audit_arguments: &Value,
    client_info: &str,
    config: &Config,
) -> Result<Value, ToolCallError> {
    let rpc_method = "openhuman.memory_doc_put";

    tracing::debug!(
        tool = tool_name,
        rpc_method = rpc_method,
        client = client_info,
        "[mcp_server] write dispatch"
    );

    tracing::trace!(
        tool = tool_name,
        rpc_method = rpc_method,
        param_keys = ?params.keys().collect::<Vec<_>>(),
        "[mcp_server] write dispatch invoking rpc"
    );

    match all::try_invoke_registered_rpc(rpc_method, params.clone()).await {
        Some(Ok(value)) => {
            let document_id = extract_document_id(&value);
            audit_write(
                config,
                NewMcpWriteRecord {
                    timestamp_ms: now_ms(),
                    client_info: client_info.to_string(),
                    tool_name: tool_name.to_string(),
                    args_summary: summarize_write_args(tool_name, audit_arguments),
                    resulting_chunk_id: document_id.clone(),
                    success: true,
                    error_message: None,
                },
            );
            tracing::debug!(
                tool = tool_name,
                chunk_id = document_id.as_deref().unwrap_or("<unknown>"),
                client = client_info,
                "[mcp_server] write success"
            );
            Ok(tool_success(value))
        }
        Some(Err(message)) => {
            audit_write(
                config,
                NewMcpWriteRecord {
                    timestamp_ms: now_ms(),
                    client_info: client_info.to_string(),
                    tool_name: tool_name.to_string(),
                    args_summary: summarize_write_args(tool_name, audit_arguments),
                    resulting_chunk_id: None,
                    success: false,
                    error_message: Some(message.clone()),
                },
            );
            log::warn!(
                "[mcp_server] write handler error tool={} error={}",
                tool_name,
                message
            );
            Ok(tool_error(format!("{} failed: {message}", tool_name)))
        }
        None => {
            let message = format!("mapped RPC method `{rpc_method}` is not registered");
            audit_write(
                config,
                NewMcpWriteRecord {
                    timestamp_ms: now_ms(),
                    client_info: client_info.to_string(),
                    tool_name: tool_name.to_string(),
                    args_summary: summarize_write_args(tool_name, audit_arguments),
                    resulting_chunk_id: None,
                    success: false,
                    error_message: Some(message.clone()),
                },
            );
            log::error!(
                "[mcp_server] write mapping missing registered RPC method tool={} rpc_method={}",
                tool_name,
                rpc_method
            );
            Ok(tool_error(format!("{tool_name} is unavailable: {message}")))
        }
    }
}

fn audit_write(config: &Config, record: NewMcpWriteRecord) {
    let config = config.clone();
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        std::mem::drop(handle.spawn_blocking(move || {
            if let Err(err) = audit::record_write(&config, record) {
                log::warn!("[mcp_server] mcp write audit insert failed: {err}");
            }
        }));
    } else {
        let _ = std::thread::spawn(move || {
            if let Err(err) = audit::record_write(&config, record) {
                log::warn!("[mcp_server] mcp write audit insert failed: {err}");
            }
        });
    }
}

pub(super) fn audit_write_rejection(
    config: &Config,
    tool_name: &str,
    audit_arguments: &Value,
    params: Option<&Map<String, Value>>,
    client_info: &str,
    err: &ToolCallError,
) {
    log::debug!(
        "[mcp_server] write rejected before dispatch tool={} client={} error={}",
        tool_name,
        client_info,
        err.message()
    );
    audit_write(
        config,
        NewMcpWriteRecord {
            timestamp_ms: now_ms(),
            client_info: client_info.to_string(),
            tool_name: tool_name.to_string(),
            args_summary: summarize_rejected_write_args(tool_name, audit_arguments, params),
            resulting_chunk_id: None,
            success: false,
            error_message: Some(err.message().to_string()),
        },
    );
}

pub(super) fn audit_write_rejection_without_config(
    tool_name: &str,
    audit_arguments: &Value,
    client_info: &str,
    error_message: &str,
) {
    log::debug!(
        "[mcp_server] write rejected before config load tool={} client={} error={}",
        tool_name,
        client_info,
        error_message
    );

    let tool_name = tool_name.to_string();
    let client_info = client_info.to_string();
    let error_message = error_message.to_string();
    let args_summary = summarize_write_args(&tool_name, audit_arguments);
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            std::mem::drop(handle.spawn(async move {
                match config_rpc::load_config_with_timeout().await {
                    Ok(config) => audit_write(
                        &config,
                        NewMcpWriteRecord {
                            timestamp_ms: now_ms(),
                            client_info,
                            tool_name,
                            args_summary,
                            resulting_chunk_id: None,
                            success: false,
                            error_message: Some(error_message),
                        },
                    ),
                    Err(err) => log::warn!(
                        "[mcp_server] write rejection audit skipped tool={} config load failed error={}",
                        tool_name,
                        err
                    ),
                }
            }));
        }
        Err(err) => log::warn!(
            "[mcp_server] write rejection audit skipped tool={} runtime unavailable error={}",
            tool_name,
            err
        ),
    }
}

pub(super) fn is_write_tool(tool_name: &str) -> bool {
    matches!(tool_name, "memory.store" | "memory.note" | "tree.tag")
}

fn summarize_rejected_write_args(
    tool_name: &str,
    audit_arguments: &Value,
    params: Option<&Map<String, Value>>,
) -> Value {
    let mut summary = summarize_write_args(tool_name, audit_arguments);
    if let (Value::Object(summary), Some(params)) = (&mut summary, params) {
        let mut param_keys = params.keys().cloned().collect::<Vec<_>>();
        param_keys.sort();
        summary.insert(
            "param_keys".to_string(),
            Value::Array(param_keys.into_iter().map(Value::String).collect()),
        );
    }
    summary
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn extract_document_id(value: &Value) -> Option<String> {
    value
        .get("document_id")
        .or_else(|| {
            value
                .get("result")
                .and_then(|result| result.get("document_id"))
        })
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn summarize_write_args(tool_name: &str, arguments: &Value) -> Value {
    let Some(args) = arguments.as_object() else {
        return json!({});
    };
    match tool_name {
        "memory.store" => json!({
            "title": args
                .get("title")
                .and_then(Value::as_str)
                .map(|title| first_chars(title, 128))
                .unwrap_or_default(),
            "namespace": args
                .get("namespace")
                .and_then(Value::as_str)
                .unwrap_or("mcp"),
            "tag_count": args
                .get("tags")
                .and_then(Value::as_array)
                .map(|tags| tags.len())
                .unwrap_or(0),
        }),
        "memory.note" => json!({
            "chunk_id": args
                .get("chunk_id")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "note_text_length": args
                .get("note_text")
                .and_then(Value::as_str)
                .map(|note| note.chars().count())
                .unwrap_or(0),
        }),
        "tree.tag" => json!({
            "chunk_id": args
                .get("chunk_id")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "tags": args
                .get("tags")
                .and_then(Value::as_array)
                .map(|tags| {
                    tags.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        }),
        _ => json!({}),
    }
}

fn first_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
#[path = "write_dispatch_tests.rs"]
mod tests;
