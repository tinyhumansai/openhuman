//! Tool listing ops.

use crate::config::Config;
use crate::core::Outcome;

use super::super::module_client::{self as connectors};
use super::super::types::{ComposioListToolsRequest, ComposioToolsResponse};
use super::error_utils::{report_composio_op_error, should_forward_tags, OpResult};
use tinyconnectors_bus::names::methods;

/// List the actions the agent may pick from.
///
/// # What moved
///
/// The direct route used to be a second implementation here: prefetch the
/// connected toolkits, then fetch v3 schemas for them. That is the module's
/// now, and this function no longer branches on which route is live.
///
/// # Scope filtering
///
/// The module applies the user's stored scope preference to the listing (the
/// default request), the same preference it enforces on `Execute`, so one
/// component decides both and the two cannot disagree. A curated toolkit
/// offers only the actions in its catalog.
pub async fn composio_list_tools(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    toolkits: Option<Vec<String>>,
    tags: Option<Vec<String>>,
) -> OpResult<Outcome<ComposioToolsResponse>> {
    let effective_tags = if should_forward_tags(toolkits.as_deref()) {
        tags
    } else {
        None
    };
    tracing::debug!(?toolkits, ?effective_tags, "[composio] rpc list_tools");

    // The module filters the listing by the stored preference; make sure a
    // preference still in the legacy file is there to be applied.
    super::ensure_user_scopes_migrated(ctx, config).await;
    let request = ComposioListToolsRequest {
        toolkits: toolkits.unwrap_or_default(),
        tags: effective_tags.unwrap_or_default(),
        ..ComposioListToolsRequest::default()
    };
    let named = request.toolkits.len();

    match connectors::call::<_, ComposioToolsResponse>(config, methods::LIST_TOOLS, request).await {
        Ok(resp) => {
            let count = resp.tools.len();
            let line = if named == 0 {
                format!("composio: {count} tool(s) listed")
            } else {
                format!("composio: {count} tool(s) listed across {named} toolkit(s)")
            };
            Ok(Outcome::new(resp, vec![line]))
        }
        Err(error) => {
            report_composio_op_error("list_tools", &anyhow::anyhow!("{error}"));
            Err(format!("[composio] list_tools failed: {error}"))
        }
    }
}
