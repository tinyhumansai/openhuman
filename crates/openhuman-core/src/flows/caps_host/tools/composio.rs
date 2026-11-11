//! Thin `tinyflows` adapter for the Composio-owned flow backend.

use super::super::{EngineError, Result};
use super::{ToolBackend, ToolCallCtx};
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::integrations::composio::flow_backend::{self, FlowCallPolicy, FlowToolCallCtx};
use crate::security::approval::ApprovalTurnContext;
use crate::security::{GateDecision, SecurityPolicy};
use async_trait::async_trait;
use serde_json::Value;

pub(crate) struct ComposioToolBackend;

struct FlowPolicy;

#[async_trait]
impl FlowCallPolicy for FlowPolicy {
    async fn tier_decision(
        &self,
        dispatch: &DispatchCtx,
        captured: &Config,
        live: &Config,
        security: &SecurityPolicy,
        class: crate::security::policy::CommandClass,
    ) -> std::result::Result<GateDecision, String> {
        let policy =
            super::super::tier::tier_policy_from_config(dispatch, captured, Some(live), security)
                .map_err(capability_payload)?;
        super::super::enforce_node_tier_gate(&policy, class, "tool_call")
            .await
            .map_err(capability_payload)
    }
    async fn approve(
        &self,
        dispatch: &DispatchCtx,
        approval: &ApprovalTurnContext,
        decision: GateDecision,
        slug: &str,
        args: &Value,
    ) -> std::result::Result<Option<String>, String> {
        let summary = crate::security::approval::summarize_action(slug, args);
        let (outcome, audit_id) = super::super::gate_call_for_tier(
            dispatch,
            approval,
            decision,
            slug,
            &summary,
            args.clone(),
        )
        .await;
        match outcome {
            crate::security::approval::GateOutcome::Allow => Ok(audit_id),
            crate::security::approval::GateOutcome::Deny { reason } => Err(reason),
        }
    }
    fn record(&self, audit_id: &str, result: &std::result::Result<Value, String>) {
        let audit_result = result
            .as_ref()
            .map(|_| ())
            .map_err(|message| capability_error_display(message));
        super::super::close_audit(Some(audit_id), &audit_result);
    }
}

fn capability_payload(error: EngineError) -> String {
    match error {
        EngineError::Capability(message) => message,
    }
}

fn capability_error_display(message: &str) -> String {
    EngineError::Capability(message.to_string()).to_string()
}

#[async_trait]
impl ToolBackend for ComposioToolBackend {
    fn name(&self) -> &'static str {
        "composio"
    }
    fn prefix(&self) -> &'static str {
        ""
    }
    async fn preflight(&self, config: &Config, slug: &str, args: &Value) -> Result<()> {
        flow_backend::preflight(config, slug, args)
            .await
            .map_err(EngineError::Capability)
    }
    async fn invoke(
        &self,
        ctx: &ToolCallCtx<'_>,
        slug: &str,
        args: Value,
        conn: Option<&str>,
    ) -> Result<Value> {
        let policy = FlowPolicy;
        let owner_ctx = FlowToolCallCtx {
            dispatch: ctx.dispatch,
            config: ctx.config,
            security: ctx.security,
            approval: ctx.approval,
            policy: &policy,
        };
        flow_backend::invoke(&owner_ctx, slug, args, conn)
            .await
            .map_err(EngineError::Capability)
    }
}

#[cfg(test)]
#[path = "composio_tests.rs"]
mod tests;
