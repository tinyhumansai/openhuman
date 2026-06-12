//! Tool execution op.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::super::client::create_composio_client;
use super::super::types::ComposioExecuteResponse;
use super::error_utils::{report_composio_op_error, OpResult};

pub async fn composio_execute(
    config: &Config,
    tool: &str,
    arguments: Option<serde_json::Value>,
    connection_id: Option<&str>,
) -> OpResult<RpcOutcome<ComposioExecuteResponse>> {
    tracing::debug!(tool = %tool, connection_id = ?connection_id, "[composio] rpc execute");
    let kind = create_composio_client(config).map_err(|e| format!("[composio] execute: {e}"))?;
    let started = std::time::Instant::now();
    let result = super::super::execute_dispatch::execute_composio_action_kind_with_connection(
        kind,
        tool,
        arguments,
        &config.composio.entity_id,
        connection_id,
    )
    .await;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    match result {
        Ok(resp) => {
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                    tool: tool.to_string(),
                    success: resp.successful,
                    error: resp.error.clone(),
                    cost_usd: resp.cost_usd,
                    elapsed_ms,
                },
            );
            Ok(RpcOutcome::new(
                resp,
                vec![format!("composio: executed {tool} ({elapsed_ms}ms)")],
            ))
        }
        Err(e) => {
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                    tool: tool.to_string(),
                    success: false,
                    error: Some(e.to_string()),
                    cost_usd: 0.0,
                    elapsed_ms,
                },
            );
            report_composio_op_error("execute", &e);
            let is_classified = e.starts_with("[composio:error:");
            tracing::debug!(
                tool = %tool,
                elapsed_ms,
                classified = is_classified,
                "[composio] rpc execute error mapped"
            );
            if is_classified {
                Err(e)
            } else {
                Err(format!("[composio] execute failed: {e}"))
            }
        }
    }
}
