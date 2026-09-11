//! Tool execution op.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::super::module_client::{self as connectors, methods};
use super::super::types::{ComposioExecuteRequest, ComposioExecuteResponse};
use super::error_utils::{report_composio_op_error, OpResult};

/// The prefix the module puts on an error it has already classified.
///
/// The domain module client removes TinyBus's member-name prefix so this stays
/// at byte zero, matching the frontend formatter's parsing contract.
const CLASSIFIED: &str = "[composio:error:";

/// Run one Composio action.
///
/// # What stays here
///
/// The domain event. Execution timing and the `ComposioActionExecuted` event
/// are how the rest of this application learns an action ran — the cost line,
/// the activity feed, the agent's own budget. The module has no bus of the
/// host's to publish on.
///
/// # What the caller must still do
///
/// Apply [`crate::openhuman::security::egress`] *before* calling this. Local
/// only mode refusing an outbound tool call is policy about the user's data,
/// and the module cannot see the reasons behind it.
pub async fn composio_execute(
    config: &Config,
    tool: &str,
    arguments: Option<serde_json::Value>,
    connection_id: Option<&str>,
) -> OpResult<RpcOutcome<ComposioExecuteResponse>> {
    tracing::debug!(tool = %tool, connection_id = ?connection_id, "[composio] rpc execute");
    let started = std::time::Instant::now();
    let result = connectors::call::<_, ComposioExecuteResponse>(
        config,
        methods::EXECUTE,
        ComposioExecuteRequest {
            tool: tool.to_string(),
            arguments,
            connection_id: connection_id.map(str::to_string),
        },
    )
    .await;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    match result {
        Ok(resp) => {
            // Note that this is reached for a *refused* action too: the module
            // reports a provider's refusal as a successful reply carrying
            // `successful: false`. The event records what happened, not whether
            // the call completed.
            crate::core::bus::BUS.publish(
                crate::core::events::DomainEvent::ComposioActionExecuted {
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
            crate::core::bus::BUS.publish(
                crate::core::events::DomainEvent::ComposioActionExecuted {
                    tool: tool.to_string(),
                    success: false,
                    error: Some(e.clone()),
                    cost_usd: 0.0,
                    elapsed_ms,
                },
            );
            report_composio_op_error("execute", &anyhow::anyhow!("{e}"));
            let is_classified = e.starts_with(CLASSIFIED);
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
