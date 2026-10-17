//! Handler for the `tools_composio_execute` controller schema.

use crate::core::runtime::DispatchCtx;
use serde_json::{json, Map, Value};

use crate::core::all::ControllerFuture;
use crate::core::Outcome;

pub(super) fn handle_composio_execute(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let action = params
            .get("action")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "missing required `action`".to_string())?;
        let action_args = params.get("params").cloned();

        let config = ctx.load_config().await?;
        // The connector module owns the mode split (backend proxy vs. the
        // user's personal direct tenant, #1710); this only supplies the config.
        let resp = crate::integrations::composio::execute_dispatch::execute_composio_action(
            &config,
            // An RPC call is not a chat turn: no chat to route the disclosure to.
            None,
            &action,
            action_args,
            None,
        )
        .await
        .map_err(|e| format!("composio execute failed: {e}"))?;
        tracing::debug!(
            action = %action,
            successful = resp.successful,
            "[tools][composio_execute] complete"
        );

        let payload = json!({
            "successful": resp.successful,
            "data": resp.data,
            "error": resp.error,
            "cost_usd": resp.cost_usd,
            "markdown_formatted": resp.markdown_formatted,
        });
        let log = vec![format!(
            "tools.composio_execute: action={action} successful={}",
            resp.successful
        )];
        Outcome::new(payload, log).into_cli_compatible_json()
    })
}
