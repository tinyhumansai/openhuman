//! Host entry for running one Composio action through the connector module.
//!
//! The prepare -> retry -> error-mapping pipeline (#1797) lives in the
//! `tinyconnectors` module (`execute::execute_action`), which serves both the
//! backend and direct routes. What stays here is the part the module
//! deliberately leaves to the host: local-only egress enforcement and the
//! external-transfer disclosure, applied once before the call.

use crate::config::Config;
use crate::security::approval::ApprovalChatContext;
use crate::security::egress::{emit_external_transfer, enforce_egress, EgressDescriptor};

use super::module_client::{self};
use super::types::{ComposioExecuteRequest, ComposioExecuteResponse};
use tinyconnectors_bus::names::methods;

/// Run `tool` against the ambient (or `connection_id`-pinned) account.
///
/// # Errors
///
/// Returns the message when the tool slug is empty, egress is refused by the
/// local-only policy, or the module reports a transport failure. A provider
/// refusal is `Ok` with `successful: false`.
pub async fn execute_composio_action(
    config: &Config,
    chat: Option<&ApprovalChatContext>,
    tool: &str,
    arguments: Option<serde_json::Value>,
    connection_id: Option<&str>,
) -> Result<ComposioExecuteResponse, String> {
    let tool = tool.trim();
    if tool.is_empty() {
        return Err("composio: tool slug must not be empty".to_string());
    }
    // Privacy epic S2/S7 (#4436, #4441): refuse under LocalOnly, otherwise
    // disclose the transfer, before the arguments leave the device.
    // The arguments are the payload: under Sensitive the module assesses them
    // (`security::egress::assess`) so a clean call is permitted and one carrying
    // credentials or identifiers is refused.
    let egress = EgressDescriptor::composio(tool).assessed(
        &arguments
            .as_ref()
            .map(serde_json::Value::to_string)
            .unwrap_or_default(),
    );
    if let Err(e) = enforce_egress(&egress).await {
        tracing::debug!(tool = %tool, "[composio][dispatch] local-only egress block");
        return Err(e.to_string());
    }
    emit_external_transfer(chat, egress);

    // Resolve the mode-aware route on the host first: a module configured
    // without a route answers with an opaque "no connector route" error, so a
    // missing direct-mode key or backend session must fail here with the
    // actionable message (#1710).
    if let Err(e) = super::client::resolve_composio_route(config) {
        tracing::debug!(tool = %tool, "[composio][dispatch] route unavailable");
        return Err(format!("{e:#}"));
    }

    tracing::debug!(
        tool = %tool,
        connection_id = ?connection_id,
        "[composio][dispatch] execute via connector module"
    );
    module_client::call::<_, ComposioExecuteResponse>(
        config,
        methods::EXECUTE,
        ComposioExecuteRequest {
            tool: tool.to_string(),
            arguments,
            connection_id: connection_id
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string),
        },
    )
    .await
}
