use super::graph_json::nodes;
use crate::config::Config;
use crate::flows::ops::{build_flow_connections, FlowConnection};
use serde_json::Value;

// ─────────────────────────────────────────────────────────────────────────────
// Connection-ref gate (WS3): a Composio tool_call's `connection_ref` must name
// a real connected account of the RIGHT toolkit
// ─────────────────────────────────────────────────────────────────────────────
//
// Transcript audit: the user's connections were `twitter →
// composio:twitter:ca_JX6QU88UfSk4`, `gmail → composio:gmail:ca_vX_WA8FsqNmE`,
// `tiktok → composio:tiktok:ca_LPCp3WQpaDma`. The agent wired
// `composio:twitter:ca_LPCp3WQpaDma` and `composio:gmail:ca_LPCp3WQpaDma` (the
// TIKTOK id) onto the Twitter and Gmail tool_call nodes. dry_run / validate /
// propose all returned ok:true — nothing cross-checked the id against the user's
// real connections, nor the ref's toolkit segment against the slug — and it
// would fail on the first real run. This gate closes that gap: it parses the
// ref, enforces the toolkit segment matches the slug (needs no I/O), and — when
// the live connection list is reachable — that the id names a real connected
// account of that toolkit, naming the correct ref when it can.

/// Hard gate: for every Composio `tool_call` node carrying a `connection_ref`,
/// prove the ref names a real connected account of the SAME toolkit as the
/// slug. Fetches the live connection list once (same source
/// [`flows_list_connections`] reads) and delegates the pure matching to
/// [`validate_connection_refs_against`].
///
/// Fail-open on I/O: if the Composio connection list is unreachable (backend
/// outage), the id-existence check is SKIPPED (a `tracing::debug!` records it)
/// so a real connection is never false-rejected during an outage — but the
/// toolkit-mismatch check, which needs no I/O, still runs.
pub(crate) async fn validate_connection_refs(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    graph: &Value,
) -> Vec<String> {
    let connections: Option<Vec<FlowConnection>> =
        match crate::integrations::composio::ops::composio_list_connections(ctx, config).await {
            Ok(outcome) => Some(build_flow_connections(
                outcome.value.connections,
                Vec::new(),
                // Identity isn't needed for this existence/toolkit-mismatch
                // check — only `connection_ref` and `toolkit` are read.
                &[],
                &Default::default(),
            )),
            Err(e) => {
                tracing::debug!(
                    target: "flows",
                    error = %e,
                    "[flows] connection-ref check: composio connection list unavailable — \
                     skipping id-existence check (fail-open); toolkit-mismatch check still runs"
                );
                None
            }
        };
    validate_connection_refs_against(graph, connections.as_deref())
}

/// Pure connection-ref validator (no I/O) so the gate's decision logic is
/// unit-testable without a live Composio backend. `connections` is `Some(list)`
/// when the live connection list was fetched (possibly empty — a genuine "no
/// connections" state), or `None` when it was unavailable (fail-open: the
/// id-existence check is skipped, only the toolkit-mismatch check runs).
pub(crate) fn validate_connection_refs_against(
    graph: &Value,
    connections: Option<&[FlowConnection]>,
) -> Vec<String> {
    use crate::integrations::composio::contract::validation::{
        validate_connection_refs as validate, ConnectedAccountRef, ConnectionRefNode,
    };

    let nodes: Vec<_> = nodes(graph)
        .filter(|node| node.kind == "tool_call")
        .filter_map(|node| {
            Some(ConnectionRefNode {
                id: node.id.to_owned(),
                slug: node.config.get("slug")?.as_str()?.to_string(),
                connection_ref: node.config.get("connection_ref")?.as_str()?.to_string(),
            })
        })
        .collect();
    let accounts = connections.map(|items| {
        items
            .iter()
            .map(|item| ConnectedAccountRef {
                connection_ref: item.connection_ref.clone(),
                toolkit: item.toolkit.clone(),
            })
            .collect::<Vec<_>>()
    });
    validate(&nodes, accounts.as_deref())
}
