//! Composio execution for flow tool calls. The flow host supplies generic tier
//! and approval decisions through `FlowCallPolicy`; Composio owns gate order,
//! curation, preflight, account resolution, and dispatch.

use crate::security::approval::ApprovalTurnContext;
use crate::{
    config::Config,
    core::runtime::DispatchCtx,
    security::{GateDecision, SecurityPolicy},
};
use async_trait::async_trait;
use serde_json::Value;

pub(crate) async fn execute_for_connection(
    config: &Config,
    slug: &str,
    args: Option<Value>,
    connection_id: Option<&str>,
) -> Result<crate::integrations::composio::ComposioExecuteResponse, String> {
    crate::integrations::composio::module_client::call::<
        _,
        crate::integrations::composio::ComposioExecuteResponse,
    >(
        config,
        tinyconnectors_bus::names::methods::EXECUTE,
        crate::integrations::composio::types::ComposioExecuteRequest {
            tool: slug.to_string(),
            arguments: args,
            connection_id: connection_id.map(str::to_string),
        },
    )
    .await
    .map_err(|error| error.to_string())
}

#[async_trait]
pub(crate) trait FlowCallPolicy: Send + Sync {
    async fn tier_decision(
        &self,
        dispatch: &DispatchCtx,
        captured: &Config,
        live: &Config,
        security: &SecurityPolicy,
        class: crate::security::policy::CommandClass,
    ) -> Result<GateDecision, String>;
    async fn approve(
        &self,
        dispatch: &DispatchCtx,
        approval: &ApprovalTurnContext,
        decision: GateDecision,
        slug: &str,
        args: &Value,
    ) -> Result<Option<String>, String>;
    fn record(&self, audit_id: &str, result: &Result<Value, String>);
}

pub(crate) struct FlowToolCallCtx<'a> {
    pub dispatch: &'a DispatchCtx,
    pub config: &'a Config,
    pub security: &'a SecurityPolicy,
    pub approval: &'a ApprovalTurnContext,
    pub policy: &'a dyn FlowCallPolicy,
}

pub(crate) async fn preflight(config: &Config, slug: &str, args: &Value) -> Result<(), String> {
    crate::integrations::composio::flow_gate::preflight_composio_args(config, slug, args).await
}

pub(crate) async fn invoke(
    ctx: &FlowToolCallCtx<'_>,
    slug: &str,
    args: Value,
    conn: Option<&str>,
) -> Result<Value, String> {
    let live = crate::config::ops::reload_config_snapshot_with_timeout(ctx.config)
        .await
        .map_err(|_| policy_refresh_error())?;
    if live.recovered_from_corruption {
        return Err(policy_refresh_error());
    }

    // Keep the privacy-sensitive tier decision before curation so a read-only
    // caller cannot probe which actions are in scope.
    let class = crate::integrations::composio::flow_gate::classify_composio_action_for_tier(slug);
    let decision = ctx
        .policy
        .tier_decision(ctx.dispatch, ctx.config, &live, ctx.security, class)
        .await?;
    if !crate::integrations::composio::flow_gate::is_curated_flow_tool(ctx.dispatch, &live, slug)
        .await
    {
        return Err(format!("tool not permitted: {slug}"));
    }
    preflight(&live, slug, &args).await?;
    let audit_id = if decision == GateDecision::Allow {
        None
    } else {
        ctx.policy
            .approve(ctx.dispatch, ctx.approval, decision, slug, &args)
            .await?
    };

    let connection_id =
        conn.and_then(crate::integrations::composio::flow_gate::composio_connection_id);
    let resolved_account = match connection_id {
        Some(id) => Some((
            id,
            crate::integrations::composio::flow_gate::resolve_composio_account(&live, id).await,
        )),
        None => None,
    };
    match &resolved_account {
        Some((id, Some((toolkit, _)))) => {
            tracing::info!(target: "flows", %slug, connection_id = %id, %toolkit, mode = live.composio.mode.as_str(), "[flows] tool_call: executing against the resolved connected account")
        }
        Some((id, None)) => {
            tracing::warn!(target: "flows", %slug, connection_id = %id, mode = live.composio.mode.as_str(), "[flows] tool_call: connection_ref connection_id not found among the user's live connected accounts (stale cache or foreign id) — forwarding the exact id so the provider can reject it rather than falling back to an ambient account")
        }
        None => {
            tracing::debug!(target: "flows", %slug, mode = live.composio.mode.as_str(), "[flows] tool_call: no connection_ref — using the ambient signed-in account")
        }
    }
    let response = execute_for_connection(
        &live,
        slug,
        (!args.is_null()).then_some(args),
        connection_id,
    )
    .await
    .and_then(|result| {
        crate::integrations::composio::flow_gate::reject_unsuccessful_composio_response(
            slug, result,
        )
        .map_err(|error| error.to_string())
    })
    .and_then(|result| serde_json::to_value(result).map_err(|error| error.to_string()));
    if let Some(audit_id) = audit_id {
        ctx.policy.record(&audit_id, &response);
    }
    response
}

fn policy_refresh_error() -> String {
    format!(
        "{} flows autonomy and Composio settings could not be refreshed; refusing the node",
        tinytools::status::POLICY_BLOCKED_MARKER
    )
}

#[cfg(all(test, feature = "modules"))]
#[path = "flow_backend_tests.rs"]
mod tests;
