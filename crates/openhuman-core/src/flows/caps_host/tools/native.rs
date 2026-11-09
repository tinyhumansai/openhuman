//! Thin TinyFlows registration and error adapter for native OpenHuman tools.

use super::super::{EngineError, Result};
use crate::security::policy::CommandClass;
use async_trait::async_trait;
use serde_json::Value;

use super::{ToolBackend, ToolCallCtx};

#[path = "native_backend.rs"]
mod flow_backend;

pub(crate) use flow_backend::NATIVE_TOOL_PREFIX;

pub(crate) struct NativeToolBackend;

#[async_trait]
impl ToolBackend for NativeToolBackend {
    fn name(&self) -> &'static str {
        "native"
    }

    fn prefix(&self) -> &'static str {
        NATIVE_TOOL_PREFIX
    }

    async fn invoke(
        &self,
        ctx: &ToolCallCtx<'_>,
        slug: &str,
        args: Value,
        _conn: Option<&str>,
    ) -> Result<Value> {
        flow_backend::invoke(ctx.config, slug, args, &NativeFlowPolicyHooks { ctx })
            .await
            .map_err(EngineError::Capability)
    }
}

struct NativeFlowPolicyHooks<'a, 'ctx> {
    ctx: &'a ToolCallCtx<'ctx>,
}

#[async_trait]
impl flow_backend::FlowPolicyHooks for NativeFlowPolicyHooks<'_, '_> {
    async fn resolve_policy(
        &self,
        config: &crate::config::Config,
    ) -> std::result::Result<std::sync::Arc<crate::security::SecurityPolicy>, String> {
        super::super::current_tier_policy(self.ctx.dispatch, config, self.ctx.security)
            .await
            .map(std::sync::Arc::new)
            .map_err(|EngineError::Capability(body)| body)
    }

    async fn authorize(
        &self,
        policy: &crate::security::SecurityPolicy,
        class: CommandClass,
        tool_name: &str,
        args: &Value,
    ) -> std::result::Result<Option<String>, String> {
        let tier_decision = super::super::enforce_node_tier_gate(policy, class, "tool_call")
            .await
            .map_err(|EngineError::Capability(body)| body)?;
        let summary = crate::security::approval::summarize_action(tool_name, args);
        let (outcome, audit_id) = super::super::gate_call_for_tier(
            self.ctx.dispatch,
            self.ctx.approval,
            tier_decision,
            tool_name,
            &summary,
            args.clone(),
        )
        .await;
        match outcome {
            crate::security::approval::GateOutcome::Deny { reason } => Err(reason),
            crate::security::approval::GateOutcome::Allow => Ok(audit_id),
        }
    }

    fn record_execution(
        &self,
        audit_id: Option<&str>,
        result: &std::result::Result<Value, String>,
    ) {
        let audit_result = result
            .as_ref()
            .map(|_| ())
            .map_err(|body| EngineError::Capability(body.clone()));
        tracing::debug!(
            target: "flows",
            audit_id = ?audit_id,
            success = result.is_ok(),
            "[flows] recording native tool execution outcome on the approval audit trail"
        );
        super::super::close_audit(audit_id, &audit_result);
    }
}
