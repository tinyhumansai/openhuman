//! Tool: `update_check` — surface the orchestrator's view of the
//! self-update domain.
//!
//! Read-only wrapper around [`crate::openhuman::update::rpc::update_check`].
//! Lets the orchestrator answer "is there a new version?" in chat without
//! routing the user to Settings → Developer Options. Implementation is a
//! thin pass-through — release URL discovery and version comparison stay
//! in the `update` domain.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use crate::openhuman::update;

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

#[cfg(test)]
#[path = "update_check_tests.rs"]
mod tests;
