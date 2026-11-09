//! Agent-facing tools for the platform update domain.
//!
//! These tools preserve the stable `update_check` and `update_apply` names
//! while keeping their implementation beside the RPC behavior they wrap.

use crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool;
use crate::core::runtime::DispatchCtx;
use crate::platform::update;
use crate::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tinytools::{PermissionLevel, Tool, ToolResult};

// Read-only release discovery.
pub struct UpdateCheckTool;

impl UpdateCheckTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UpdateCheckTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UpdateCheckTool {
    fn name(&self) -> &str {
        "update_check"
    }

    fn description(&self) -> &str {
        "Check whether a newer OpenHuman core build is available on GitHub \
         Releases. Read-only — performs one HTTPS request to the releases \
         feed and returns version info plus, if a newer build exists, its \
         download URL, asset name, and release notes. Use this when the \
         user asks 'am I up to date', 'is there a new version', or before \
         offering to apply an update with `update_apply`. Does NOT download \
         or install anything."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        tracing::debug!("[update_check] execute start");
        let outcome = update::rpc::update_check().await;
        let body = serde_json::to_string_pretty(&outcome.value)?;
        for log in &outcome.logs {
            tracing::debug!(target: "update_check", "{log}");
        }
        // `update_check` is read-only — both "newer release available"
        // and "you're already on the latest version" are normal success
        // outcomes here, so we only error out when the underlying RPC
        // explicitly surfaces an `error` key (e.g. release feed
        // unreachable, asset metadata malformed). The applied=false
        // tightening that `update_apply` performs is intentionally NOT
        // mirrored — read-only callers must keep treating "no update"
        // as a happy answer.
        let is_error = outcome.value.get("error").is_some();
        tracing::debug!(
            is_error,
            body_len = body.len(),
            "[update_check] execute done"
        );
        Ok(if is_error {
            ToolResult::error(body)
        } else {
            ToolResult::success(body)
        })
    }
}

// Consent-gated download, staging, and restart request.
pub struct UpdateApplyTool {
    security: Arc<SecurityPolicy>,
}

impl UpdateApplyTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    fn require_consent(args: &Value) -> Option<ToolResult> {
        let confirmed = args
            .get("user_confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if confirmed {
            return None;
        }
        Some(ToolResult::error(
            "update_apply blocked: needs explicit user consent. Confirm with the user via \
             `ask_user_clarification` (e.g. 'Should I download and stage the new core build now?'), \
             then call this tool again with `user_confirmed: true`.",
        ))
    }
}

#[async_trait]
impl Tool for UpdateApplyTool {
    fn name(&self) -> &str {
        "update_apply"
    }

    fn description(&self) -> &str {
        "Download, verify, and stage the latest OpenHuman core binary, then \
         request a restart per the configured restart strategy. HIGH IMPACT — \
         replaces the running binary on disk and (under `self_replace`) \
         restarts the core process. ALWAYS confirm with the user via \
         `ask_user_clarification` first, then call this tool with \
         `user_confirmed: true`. Use `update_check` first to verify a newer \
         version actually exists. Disabled when \
         `config.update.rpc_mutations_enabled = false`."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "user_confirmed": {
                    "type": "boolean",
                    "description": "Must be true. Set ONLY after the user has explicitly approved \
                                    applying the update via `ask_user_clarification`. The tool \
                                    refuses to run when this is missing or false."
                }
            },
            "required": ["user_confirmed"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.run(args, &dispatch_ctx_for_tool(None)).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.run(args, &dispatch_ctx_for_tool(context)).await
    }
}

impl UpdateApplyTool {
    async fn run(&self, args: Value, ctx: &DispatchCtx) -> anyhow::Result<ToolResult> {
        tracing::debug!(args = %args, "[update_apply] execute start");

        if let Some(blocked) = Self::require_consent(&args) {
            tracing::warn!("[update_apply] blocked: missing user_confirmed");
            return Ok(blocked);
        }
        let outcome = update::rpc::update_run(ctx).await;
        for log in &outcome.logs {
            tracing::debug!(target: "update_apply", "{log}");
        }
        let body = serde_json::to_string_pretty(&outcome.value)?;
        // `Outcome<Value>` does not carry an explicit status flag, so
        // we have to read the shape: a `{"error": …}` body is the obvious
        // failure case, but `update_run`'s soft-failure paths
        // ("already current", "no platform asset for this target",
        // "download/stage failed") return `applied: false` with a
        // descriptive `message` and no `error` key. Surfacing those as
        // `ToolResult::success` would mean the user sees a green tick in
        // chat even though nothing was installed — treat any non-applied
        // outcome as a tool error so the LLM has to acknowledge it.
        let applied = outcome
            .value
            .get("applied")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let has_error_key = outcome.value.get("error").is_some();
        let is_error = has_error_key || !applied;
        tracing::debug!(
            applied,
            has_error_key,
            is_error,
            "[update_apply] execute done"
        );
        Ok(if is_error {
            ToolResult::error(body)
        } else {
            ToolResult::success(body)
        })
    }
}

/// Build the update domain's tools in their persisted catalogue order.
pub(crate) fn registered_tools(security: Arc<SecurityPolicy>) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(UpdateCheckTool::new()),
        Box::new(UpdateApplyTool::new(security)),
    ]
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
