//! Tool execution op.

use crate::config::Config;
use crate::core::Outcome;

use super::super::execute_dispatch::execute_composio_action;
use super::super::types::ComposioExecuteResponse;
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
/// The call goes through [`execute_composio_action`], so the local-only
/// egress policy applies here as it does to the agent tool.
pub async fn composio_execute(
    config: &Config,
    tool: &str,
    arguments: Option<serde_json::Value>,
    connection_id: Option<&str>,
) -> OpResult<Outcome<ComposioExecuteResponse>> {
    tracing::debug!(tool = %tool, connection_id = ?connection_id, "[composio] rpc execute");
    // The keys this call may send, read before it: redaction covers them even
    // if the stored key changes while it runs.
    let keys = super::super::tools::composio_call_keys(config).await?;
    let started = std::time::Instant::now();
    let result = execute_composio_action(config, None, tool, arguments, connection_id).await;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    match result {
        Ok(resp) => {
            // The RPC result reaches the renderer and the RPC log: strip the
            // Composio keys from it, as the agent tools do with their results.
            let (successful, failed_with_text, cost_usd) =
                (resp.successful, resp.error.is_some(), resp.cost_usd);
            let resp = match super::super::tools::redact_composio_response(config, keys, resp).await
            {
                Ok(resp) => resp,
                Err(withheld) => {
                    // The action ran and may have cost something: record it,
                    // without the provider's error text that could not be
                    // cleaned.
                    crate::core::bus::BUS.publish(
                        crate::core::events::DomainEvent::ComposioActionExecuted {
                            tool: tool.to_string(),
                            success: successful,
                            error: failed_with_text.then(|| withheld.clone()),
                            cost_usd,
                            elapsed_ms,
                        },
                    );
                    return Err(withheld);
                }
            };
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
            Ok(Outcome::new(
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
