//! Toolkit and capability listing ops.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::super::client::{create_composio_client, ComposioClientKind};
use super::super::providers::{agent_ready_toolkits, capability_matrix};
use super::super::types::{ComposioCapabilitiesResponse, ComposioToolkitsResponse};
use super::error_utils::{report_composio_op_error, OpResult};

pub async fn composio_list_toolkits(
    config: &Config,
) -> OpResult<RpcOutcome<ComposioToolkitsResponse>> {
    tracing::debug!("[composio] rpc list_toolkits");
    let kind =
        create_composio_client(config).map_err(|e| format!("[composio] list_toolkits: {e}"))?;
    match kind {
        ComposioClientKind::Backend(client) => {
            tracing::debug!("[composio] list_toolkits: backend variant");
            let resp = client.list_toolkits().await.map_err(|e| {
                report_composio_op_error("list_toolkits", &e);
                format!("[composio] list_toolkits failed: {e:#}")
            })?;
            let count = resp.toolkits.len();
            Ok(RpcOutcome::new(
                resp,
                vec![format!("composio: {count} toolkit(s) enabled")],
            ))
        }
        ComposioClientKind::Direct(_) => {
            tracing::info!(
                "[composio-direct] list_toolkits: direct mode active — no \
                 server-side allowlist is enforced; returning empty toolkits \
                 list. Users manage available toolkits via app.composio.dev."
            );
            Ok(RpcOutcome::new(
                ComposioToolkitsResponse::default(),
                vec!["composio: direct mode — no curated allowlist (toolkits \
                     managed via app.composio.dev)"
                    .to_string()],
            ))
        }
    }
}

pub async fn composio_list_capabilities(
    _config: &Config,
) -> OpResult<RpcOutcome<ComposioCapabilitiesResponse>> {
    tracing::debug!("[composio] rpc list_capabilities");
    let resp = ComposioCapabilitiesResponse {
        capabilities: capability_matrix(),
    };
    let count = resp.capabilities.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: {count} capability row(s) listed")],
    ))
}

/// List every toolkit slug that ships an agent-ready curated catalog.
///
/// Connected toolkits that are NOT in this list can still be
/// authorized via OAuth, but the agent has no curated action surface
/// for them — the UI should label such connections as
/// "preview / agent integration coming soon" so users aren't led into
/// a broken `composio_list_tools` → max-iterations loop. See #2283.
pub async fn composio_list_agent_ready_toolkits(
) -> OpResult<RpcOutcome<super::super::types::ComposioAgentReadyToolkitsResponse>> {
    tracing::debug!("[composio] rpc list_agent_ready_toolkits");
    let toolkits: Vec<String> = agent_ready_toolkits()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    let count = toolkits.len();
    let resp = super::super::types::ComposioAgentReadyToolkitsResponse { toolkits };
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: {count} agent-ready toolkit(s) listed")],
    ))
}
