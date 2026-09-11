//! Tool listing ops.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::super::module_client::{self as connectors, methods};
use super::super::types::{ComposioListToolsRequest, ComposioToolsResponse};
use super::error_utils::{report_composio_op_error, should_forward_tags, OpResult};

/// List the actions the agent may pick from.
///
/// # What moved
///
/// The direct route used to be a second implementation here: prefetch the
/// connected toolkits, then fetch v3 schemas for them. That is the module's
/// now, and this function no longer branches on which route is live.
///
/// # Why the scope filter is asked for `false`
///
/// The module can filter a listing by the user's scope preference, and
/// eventually it should — it is the half that also refuses an `Execute`, and
/// one component deciding both is the only arrangement that cannot disagree
/// with itself.
///
/// It cannot yet, because the preference is not stored where the module can
/// see it. The rows live in the bound memory driver's KV tier
/// (`super::user_scopes`), and the module reads its own state directory. Asking
/// it to filter today would filter against *its* defaults — `read+write`, no
/// `admin` — rather than against what the user actually chose, and the two
/// would disagree the moment anyone touched the toggle.
///
/// So enforcement stays whole on this side for now: `super::super::tools`
/// applies the preference to what the agent sees, which is the path that leads
/// to an action being run. Phase 4 moves the store, and this flag flips with
/// it.
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

    let request = ComposioListToolsRequest {
        toolkits: toolkits.unwrap_or_default(),
        tags: effective_tags.unwrap_or_default(),
        apply_user_scopes: false,
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
            Ok(RpcOutcome::new(resp, vec![line]))
        }
        Err(error) => {
            report_composio_op_error("list_tools", &anyhow::anyhow!("{error}"));
            Err(format!("[composio] list_tools failed: {error}"))
        }
    }
}
