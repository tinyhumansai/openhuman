//! Adapts native tool results to the value and failure contract expected by
//! TinyFlows. Tool execution remains in `tools::native_ops`, which owns the
//! native registry and its policy snapshot; this flow adapter stays with the
//! CapsHost capability that calls it.

use std::sync::Arc;

use crate::security::policy::CommandClass;
use async_trait::async_trait;
use serde_json::{json, Value};
use tinytools::ToolResult;

use crate::{config::Config, security::SecurityPolicy};

pub(crate) const NATIVE_TOOL_PREFIX: &str = "oh:";

/// Flow-specific policy and approval operations supplied by the host adapter.
#[async_trait]
pub(crate) trait FlowPolicyHooks: Send + Sync {
    async fn resolve_policy(&self, config: &Config) -> Result<Arc<SecurityPolicy>, String>;

    async fn authorize(
        &self,
        policy: &SecurityPolicy,
        class: CommandClass,
        tool_name: &str,
        args: &Value,
    ) -> Result<Option<String>, String>;

    fn record_execution(&self, audit_id: Option<&str>, result: &Result<Value, String>);
}

/// Invokes a native tool for a flow while keeping TinyFlows policy operations
/// behind neutral host hooks.
pub(crate) async fn invoke(
    config: &Config,
    slug: &str,
    args: Value,
    hooks: &dyn FlowPolicyHooks,
) -> Result<Value, String> {
    let tool_name = slug.strip_prefix(NATIVE_TOOL_PREFIX).unwrap_or(slug).trim();
    if tool_name.is_empty() {
        return Err("tool_call node: native tool slug is empty (expected `oh:<tool_name>`)".into());
    }

    let policy = hooks.resolve_policy(config).await?;
    let class = crate::tools::native_ops::classify_tool_call_with_security(
        config,
        policy.clone(),
        tool_name,
        &args,
    )
    .await?;
    let audit_id = hooks.authorize(&policy, class, tool_name, &args).await?;
    tracing::debug!(target: "flows", %tool_name, ?class, "[flows] dispatching NATIVE OpenHuman tool");
    let result = crate::tools::native_ops::execute_tool_with_security(
        config, policy, tool_name, args, false,
    )
    .await
    .and_then(|outcome| {
        reject_failed_native_tool_result(slug, &outcome.result)
            .map(|()| native_tool_payload(&outcome.result))
    });
    hooks.record_execution(audit_id.as_deref(), &result);
    result
}

/// Turns a failed native tool result into the flow-facing error body.
pub(crate) fn reject_failed_native_tool_result(
    slug: &str,
    result: &ToolResult,
) -> Result<(), String> {
    if !result.is_error {
        return Ok(());
    }
    let rendered = result.output();
    let detail = match rendered.trim() {
        "" => "no error detail returned by the tool",
        detail => detail,
    };
    tracing::warn!(
        target: "flows",
        %slug,
        %detail,
        "[flows] tool_call: native tool reported is_error — failing the step"
    );
    Err(format!("tool_call `{slug}` failed: {detail}"))
}

/// Projects a tool result to the value downstream flow expressions bind.
/// A single JSON block is returned directly; text and mixed blocks expose the
/// rendered output under `text`.
pub(crate) fn native_tool_payload(result: &ToolResult) -> Value {
    use tinytools::ToolContent;
    match result.content.as_slice() {
        [ToolContent::Json { data }] => data.clone(),
        _ => json!({ "text": result.output() }),
    }
}

#[cfg(test)]
#[path = "native_backend_tests.rs"]
mod tests;
