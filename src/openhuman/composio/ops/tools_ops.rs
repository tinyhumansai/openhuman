//! Tool listing ops.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::super::client::{
    create_composio_client, direct_list_connections, direct_list_tools, ComposioClientKind,
};
use super::super::types::ComposioToolsResponse;
use super::error_utils::{report_composio_op_error, should_forward_tags, OpResult};

pub async fn composio_list_tools(
    config: &Config,
    toolkits: Option<Vec<String>>,
    tags: Option<Vec<String>>,
) -> OpResult<RpcOutcome<ComposioToolsResponse>> {
    let effective_tags = if should_forward_tags(toolkits.as_deref()) {
        tags
    } else {
        None
    };
    tracing::debug!(?toolkits, ?effective_tags, "[composio] rpc list_tools");
    let kind = create_composio_client(config).map_err(|e| format!("[composio] list_tools: {e}"))?;
    match kind {
        ComposioClientKind::Backend(client) => {
            tracing::debug!("[composio] list_tools: backend variant");
            let resp = client
                .list_tools(toolkits.as_deref(), effective_tags.as_deref())
                .await
                .map_err(|e| {
                    report_composio_op_error("list_tools", &e);
                    format!("[composio] list_tools failed: {e:#}")
                })?;
            let count = resp.tools.len();
            Ok(RpcOutcome::new(
                resp,
                vec![format!("composio: {count} tool(s) listed")],
            ))
        }
        ComposioClientKind::Direct(direct) => {
            let scope: Vec<String> = match toolkits {
                Some(list) if !list.is_empty() => list,
                _ => {
                    let conns = direct_list_connections(&direct).await.map_err(|e| {
                        let rendered = format!(
                            "[composio-direct] list_tools: prefetch connections failed: {e:#}"
                        );
                        report_composio_op_error("list_connections", &rendered);
                        rendered
                    })?;
                    let mut v: Vec<String> = conns
                        .connections
                        .iter()
                        .filter(|c| c.is_active())
                        .map(|c| c.normalized_toolkit())
                        .filter(|t| !t.is_empty())
                        .collect();
                    v.sort();
                    v.dedup();
                    v
                }
            };
            if scope.is_empty() {
                tracing::info!(
                    "[composio-direct] list_tools: no connected toolkits on this tenant — \
                     returning empty tool list"
                );
                return Ok(RpcOutcome::new(
                    ComposioToolsResponse::default(),
                    vec!["composio: direct mode — 0 tool(s) listed (no connected \
                         toolkits on this tenant)"
                        .to_string()],
                ));
            }
            tracing::debug!(
                toolkits = scope.len(),
                ?effective_tags,
                "[composio-direct] list_tools: fetching v3 tool schemas"
            );
            let mut resp = direct_list_tools(&direct, &scope, effective_tags.as_deref())
                .await
                .map_err(|e| {
                    let rendered = format!("[composio-direct] list_tools failed: {e:#}");
                    report_composio_op_error("list_tools", &rendered);
                    rendered
                })?;
            let before = resp.tools.len();
            filter_list_tools_response_for_direct(&mut resp).await;
            let after = resp.tools.len();
            tracing::debug!(
                before,
                after,
                dropped = before - after,
                "[composio-direct] list_tools: curated filter applied"
            );
            let count = resp.tools.len();
            Ok(RpcOutcome::new(
                resp,
                vec![format!(
                    "composio: direct mode — {count} tool(s) listed across \
                     {} toolkit(s)",
                    scope.len()
                )],
            ))
        }
    }
}

/// Apply OpenHuman's curated-whitelist + user-scope visibility filter to
/// a fresh `ComposioToolsResponse` in direct mode. Mirrors the per-call
/// filter loop in `tools.rs::filter_list_tools_response` so backend and
/// direct surfaces share the same safety net.
async fn filter_list_tools_response_for_direct(resp: &mut ComposioToolsResponse) {
    use super::super::providers::{
        catalog_for_toolkit, classify_unknown, find_curated, get_provider,
        load_user_scope_or_default, toolkit_from_slug,
    };

    let mut keep: Vec<bool> = Vec::with_capacity(resp.tools.len());
    for t in &resp.tools {
        let slug = &t.function.name;
        let Some(toolkit) = toolkit_from_slug(slug) else {
            keep.push(true);
            continue;
        };
        let pref = load_user_scope_or_default(&toolkit).await;
        let catalog = get_provider(&toolkit)
            .and_then(|p| p.curated_tools())
            .or_else(|| catalog_for_toolkit(&toolkit));
        let allowed = match catalog {
            Some(cat) => match find_curated(cat, slug) {
                Some(curated) => pref.allows(curated.scope),
                None => false,
            },
            None => pref.allows(classify_unknown(slug)),
        };
        keep.push(allowed);
    }
    let drained: Vec<_> = resp.tools.drain(..).collect();
    resp.tools = drained
        .into_iter()
        .zip(keep)
        .filter_map(|(tool, keep_it)| if keep_it { Some(tool) } else { None })
        .collect();
}
