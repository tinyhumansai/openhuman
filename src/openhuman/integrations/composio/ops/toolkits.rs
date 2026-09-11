//! Toolkit and capability listing ops.

use super::super::module_client::{self as connectors, methods};
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::super::providers::agent_ready_toolkits;
use super::super::types::{ComposioCapabilitiesResponse, ComposioToolkitsResponse};
use super::error_utils::{report_composio_op_error, OpResult};

pub async fn composio_list_toolkits(
    config: &Config,
) -> OpResult<RpcOutcome<ComposioToolkitsResponse>> {
    tracing::debug!("[composio] rpc list_toolkits");

    match connectors::call_bare::<ComposioToolkitsResponse>(config, methods::LIST_TOOLKITS).await {
        Ok(resp) => {
            let count = resp.toolkits.len();
            Ok(RpcOutcome::new(
                resp,
                vec![format!("composio: {count} toolkit(s) enabled")],
            ))
        }
        // Direct mode has no per-user allowlist to report, and the module says
        // so by name rather than returning an empty list. The empty list is
        // still the right *answer* for this surface — the user manages their
        // toolkits at app.composio.dev — so the host renders it here, with the
        // note it has always shown, instead of the module inventing it.
        Err(error) if connectors::is_unsupported_by_route(&error) => {
            tracing::info!(
                "[composio] list_toolkits: the live route enforces no server-side allowlist; \
                 returning an empty list"
            );
            Ok(RpcOutcome::new(
                ComposioToolkitsResponse::default(),
                vec!["composio: direct mode — no curated allowlist (toolkits \
                     managed via app.composio.dev)"
                    .to_string()],
            ))
        }
        Err(error) => {
            report_composio_op_error("list_toolkits", &anyhow::anyhow!("{error}"));
            Err(format!("[composio] list_toolkits failed: {error}"))
        }
    }
}

pub async fn composio_list_capabilities(
    config: &Config,
) -> OpResult<RpcOutcome<ComposioCapabilitiesResponse>> {
    tracing::debug!("[composio] rpc list_capabilities");
    // Used to be built host-side from `tinymemory`'s engine provider
    // registry via `capability_matrix()`, deleted with the rest of the
    // in-process pipeline by tinymemory v1.13.4. The connector module now
    // answers this directly — `ListCapabilities` already returns exactly
    // this reply shape, so there is no host-side matrix or conversion left.
    let resp =
        connectors::call_bare::<ComposioCapabilitiesResponse>(config, methods::LIST_CAPABILITIES)
            .await
            .map_err(|error| {
                report_composio_op_error("list_capabilities", &anyhow::anyhow!("{error}"));
                format!("[composio] list_capabilities failed: {error}")
            })?;
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
