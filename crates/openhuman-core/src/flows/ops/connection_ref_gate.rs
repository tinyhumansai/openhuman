use super::*;

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

/// Parses a `composio:<toolkit>:<id>` connection_ref into its `(toolkit, id)`
/// segments. Mirrors [`crate::flows::tinyflows::caps::composio_connection_id`]'s
/// rsplit for the id (everything after the LAST `:`), taking everything between
/// the `composio:` prefix and that last `:` as the toolkit. Returns `None` for
/// anything that isn't this shape (missing `composio:` prefix, no `:` after it,
/// or an empty toolkit/id segment).
fn parse_composio_connection_ref(conn_ref: &str) -> Option<(&str, &str)> {
    let rest = conn_ref.strip_prefix("composio:")?;
    let (toolkit, id) = rest.rsplit_once(':')?;
    if toolkit.trim().is_empty() || id.trim().is_empty() {
        return None;
    }
    Some((toolkit.trim(), id.trim()))
}

/// First connected account `connection_ref` for `toolkit` (case-insensitive)
/// from `conns`, used to name the correct ref in a rejection's "did you mean"
/// hint. `None` when the toolkit has no connection at all.
fn first_connection_ref_for_toolkit(conns: &[FlowConnection], toolkit: &str) -> Option<String> {
    conns
        .iter()
        .find(|c| {
            c.toolkit
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(toolkit))
        })
        .map(|c| c.connection_ref.clone())
}

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
    config: &Config,
    graph: &WorkflowGraph,
) -> Vec<String> {
    let connections: Option<Vec<FlowConnection>> =
        match crate::integrations::composio::ops::composio_list_connections(config).await {
            Ok(outcome) => Some(build_flow_connections(
                outcome.value.connections,
                Vec::new(),
                // Identity isn't needed for this existence/toolkit-mismatch
                // check — only `connection_ref` and `toolkit` are read.
                &[],
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
pub(super) fn validate_connection_refs_against(
    graph: &WorkflowGraph,
    connections: Option<&[FlowConnection]>,
) -> Vec<String> {
    use crate::integrations::composio::contract::toolkit_from_slug;

    let mut errors = Vec::new();
    for node in &graph.nodes {
        if node.kind != NodeKind::ToolCall {
            continue;
        }
        let Some(slug) = node.config.get("slug").and_then(Value::as_str) else {
            continue;
        };
        // `=`-derived slugs resolve at runtime; native `oh:` tools have no
        // Composio connection to name.
        if slug.starts_with('=') || slug.starts_with("oh:") {
            continue;
        }
        // A MISSING `connection_ref` stays allowed (unchanged): a Composio
        // tool_call with no ref runs against the ambient signed-in account and
        // the flow prompts for a connection at first run.
        let Some(conn_ref) = node.config.get("connection_ref").and_then(Value::as_str) else {
            continue;
        };
        if conn_ref.trim().is_empty() {
            continue;
        }
        let Some(slug_toolkit) = toolkit_from_slug(slug) else {
            continue;
        };

        let Some((ref_toolkit, ref_id)) = parse_composio_connection_ref(conn_ref) else {
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                toolkit = %slug_toolkit,
                %conn_ref,
                matched = false,
                "[flows] connection-ref check: malformed ref — rejecting"
            );
            errors.push(format!(
                "Node '{}': `connection_ref` `{conn_ref}` is malformed — a Composio account ref \
                 must look like `composio:<toolkit>:<connection_id>` (e.g. \
                 `composio:{slug_toolkit}:<id>`). Call list_flow_connections and copy a \
                 `connection_ref` value verbatim.",
                node.id
            ));
            continue;
        };

        // Toolkit segment vs the slug's toolkit — needs no I/O.
        if !ref_toolkit.eq_ignore_ascii_case(&slug_toolkit) {
            let suggestion = connections
                .and_then(|conns| first_connection_ref_for_toolkit(conns, &slug_toolkit));
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                toolkit = %slug_toolkit,
                %ref_toolkit,
                %ref_id,
                matched = false,
                "[flows] connection-ref check: toolkit segment does not match the slug's toolkit — rejecting"
            );
            let hint = match suggestion {
                Some(r) => format!(" — did you mean `{r}`?"),
                None => format!(
                    " — no `{slug_toolkit}` account is connected; connect one with \
                     composio_connect (or ask the user to), then use its `connection_ref`"
                ),
            };
            errors.push(format!(
                "Node '{}': `connection_ref` `{conn_ref}` names the `{ref_toolkit}` toolkit but the \
                 tool_call slug `{slug}` is a `{slug_toolkit}` action{hint}.",
                node.id
            ));
            continue;
        }

        // Existence check: the id must name a real connected account of this
        // toolkit. Skipped (fail-open) when the connection list is unavailable.
        let Some(conns) = connections else {
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                toolkit = %slug_toolkit,
                %ref_id,
                "[flows] connection-ref check: toolkit matches; id-existence check skipped (connections unavailable)"
            );
            continue;
        };
        // The id must belong to a connection OF THIS TOOLKIT — not merely
        // exist somewhere. The transcript bug was a real TIKTOK connection id
        // stamped onto a `composio:twitter:` ref: the id exists globally, but
        // it is not a Twitter account, so it must still be rejected.
        let id_exists = conns.iter().any(|c| {
            c.toolkit
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(&slug_toolkit))
                && parse_composio_connection_ref(&c.connection_ref)
                    .is_some_and(|(_, cid)| cid.eq_ignore_ascii_case(ref_id))
        });
        if id_exists {
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                toolkit = %slug_toolkit,
                %ref_id,
                matched = true,
                "[flows] connection-ref check: ref resolves to a real connected account — ok"
            );
            continue;
        }
        // Unknown id. Name the right ref for this toolkit if one exists.
        match first_connection_ref_for_toolkit(conns, &slug_toolkit) {
            Some(r) => {
                tracing::debug!(
                    target: "flows",
                    node = %node.id,
                    %slug,
                    toolkit = %slug_toolkit,
                    %ref_id,
                    matched = false,
                    "[flows] connection-ref check: unknown id; toolkit has a different connected account — rejecting"
                );
                errors.push(format!(
                    "Node '{}': `connection_ref` `{conn_ref}` does not match any connected \
                     `{slug_toolkit}` account — did you mean `{r}`? Call list_flow_connections and \
                     copy a `connection_ref` value verbatim.",
                    node.id
                ));
            }
            None => {
                tracing::debug!(
                    target: "flows",
                    node = %node.id,
                    %slug,
                    toolkit = %slug_toolkit,
                    %ref_id,
                    matched = false,
                    "[flows] connection-ref check: no connected account for this toolkit — rejecting"
                );
                errors.push(format!(
                    "Node '{}': `connection_ref` `{conn_ref}` names a `{slug_toolkit}` account, but \
                     no `{slug_toolkit}` account is connected — connect one with composio_connect \
                     (or ask the user to), then use its `connection_ref`.",
                    node.id
                ));
            }
        }
    }
    errors
}
