//! Tool: `update_apply` — orchestrated check → download → stage →
//! restart of the OpenHuman core binary.
//!
//! Wraps [`crate::openhuman::update::rpc::update_run`] so the LLM can
//! finish an "update me to the latest version" intent in chat. The
//! underlying RPC already enforces:
//!
//!   * `config.update.rpc_mutations_enabled` — fail-closed if disabled.
//!   * GitHub host + asset-name validation before any download.
//!   * The configured `restart_strategy` (`self_replace` vs
//!     `supervisor`) — staging logic stays in the `update` domain.
//!
//! On top of that, this tool layers a tool-level autonomy check
//! (`SecurityPolicy::can_act`) so a read-only session can never trigger
//! a download, even if the policy gate is on.
//!
//! The tool description tells the orchestrator to confirm with the user
//! via `ask_user_clarification` before invoking — applying an update is
//! high impact (replaces a binary on disk, optionally restarts the core
//! process) and must not happen silently.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::security::{SecurityPolicy, ToolOperation};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use crate::openhuman::update;

pub struct UpdateApplyTool {
    security: Arc<SecurityPolicy>,
}

impl UpdateApplyTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    /// Delegate to the shared `SecurityPolicy::enforce_tool_operation`
    /// path so this tool stays in lock-step with every other act-level
    /// tool's autonomy + rate-limit handling. Hand-rolling
    /// `can_act` + `record_action` here used to drift from the canonical
    /// `[openhuman:policy]` log format and would silently miss any
    /// future gate the enforcer grows (token budget, supervised
    /// approval, etc.).
    fn require_write_access(&self) -> Option<ToolResult> {
        self.security
            .enforce_tool_operation(ToolOperation::Act, "update_apply")
            .err()
            .map(ToolResult::error)
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
        tracing::debug!(args = %args, "[update_apply] execute start");

        if let Some(blocked) = Self::require_consent(&args) {
            tracing::warn!("[update_apply] blocked: missing user_confirmed");
            return Ok(blocked);
        }
        if let Some(blocked) = self.require_write_access() {
            tracing::warn!("[update_apply] blocked: autonomy gate");
            return Ok(blocked);
        }

        let outcome = update::rpc::update_run().await;
        for log in &outcome.logs {
            tracing::debug!(target: "update_apply", "{log}");
        }
        let body = serde_json::to_string_pretty(&outcome.value)?;
        // `RpcOutcome<Value>` does not carry an explicit status flag, so
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

#[cfg(test)]
#[path = "update_apply_tests.rs"]
mod tests;
