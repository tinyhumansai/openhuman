//! Tool: `complete_onboarding` — finalize the chat welcome flow.
//!
//! Used exclusively by the **welcome** agent. This is the finalizer
//! half of the pair; the read-only inspection lives in
//! [`crate::openhuman::tools::implementations::agent::check_onboarding_status`].
//!
//! Flips `chat_onboarding_completed` to `true` and seeds recurring
//! proactive cron jobs. Rejects (returns a
//! [`ToolResult::error`]) if the user has not yet met the minimum
//! engagement threshold — either at least
//! [`onboarding_status::MIN_EXCHANGES_TO_COMPLETE`] welcome-agent exchanges,
//! or at least one connected Composio integration.

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult, ToolScope};
use async_trait::async_trait;
use serde_json::{json, Value};

use super::onboarding_status::{build_not_ready_to_complete_error, compute_state, detect_auth};

pub struct CompleteOnboardingTool;

impl Default for CompleteOnboardingTool {
    fn default() -> Self {
        Self::new()
    }
}

impl CompleteOnboardingTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for CompleteOnboardingTool {
    fn name(&self) -> &str {
        "complete_onboarding"
    }

    fn description(&self) -> &str {
        "Finalize the chat welcome flow. Flips `chat_onboarding_completed` \
         to `true` and seeds recurring cron jobs. Returns `\"ok\"` on \
         success.\n\
         \n\
         Takes no arguments. Call only when the most recent \
         `check_onboarding_status` snapshot showed \
         `ready_to_complete: true` — the tool re-checks the criteria \
         server-side and **rejects** premature calls with a descriptive \
         error so the agent knows to keep conversing. Rejects when the \
         user is unauthenticated, or when they have fewer than the \
         required exchange count AND no connected Composio integrations."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn scope(&self) -> ToolScope {
        ToolScope::AgentOnly
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        tracing::debug!("[complete_onboarding] execute");
        complete().await
    }
}

/// Finalize the welcome flow. See the tool description for guard rules.
async fn complete() -> anyhow::Result<ToolResult> {
    let mut config = Config::load_or_init()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load config: {e}"))?;

    // Idempotent — already done.
    if config.chat_onboarding_completed {
        tracing::debug!("[complete_onboarding] chat welcome flow already completed — no-op");
        return Ok(ToolResult::success("ok"));
    }

    // ── Auth guard ────────────────────────────────────────────────
    let (is_authenticated, _) = detect_auth(&config);
    if !is_authenticated {
        tracing::debug!("[complete_onboarding] rejected — user not authenticated");
        return Ok(ToolResult::error(
            "Cannot complete onboarding: the user is not authenticated. \
             Please guide them to log in via the desktop login flow first.",
        ));
    }

    // ── Engagement guard ──────────────────────────────────────────
    let state = compute_state(&config).await;
    tracing::debug!(
        exchange_count = state.exchange_count,
        composio_connections = state.composio_connected_toolkits.len(),
        ready = state.ready_to_complete,
        "[complete_onboarding] engagement guard check"
    );

    if !state.ready_to_complete {
        return Ok(ToolResult::error(build_not_ready_to_complete_error(
            state.exchange_count,
        )));
    }

    // ── Finalize ──────────────────────────────────────────────────
    config.chat_onboarding_completed = true;
    config
        .save()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to save config: {e}"))?;

    let seed_config = config.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::openhuman::cron::seed::seed_proactive_agents(&seed_config) {
            tracing::warn!("[complete_onboarding] failed to seed proactive cron jobs: {e}");
        }
    });

    tracing::info!(
        exchange_count = state.exchange_count,
        composio_connections = state.composio_connected_toolkits.len(),
        "[complete_onboarding] chat welcome flow finalized"
    );

    Ok(ToolResult::success("ok"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_metadata() {
        let tool = CompleteOnboardingTool::new();
        assert_eq!(tool.name(), "complete_onboarding");
        assert_eq!(tool.permission_level(), PermissionLevel::Write);
        assert_eq!(tool.scope(), ToolScope::AgentOnly);
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        // No required params — call it with `{}`.
        assert!(schema.get("required").is_none());
    }

    #[test]
    fn description_mentions_check_onboarding_status() {
        let desc = CompleteOnboardingTool::new().description().to_string();
        assert!(
            desc.contains("check_onboarding_status"),
            "description should point agents at the companion status tool: {desc}"
        );
        assert!(desc.contains("ready_to_complete"));
    }

    #[test]
    fn spec_roundtrip() {
        let tool = CompleteOnboardingTool::new();
        let spec = tool.spec();
        assert_eq!(spec.name, "complete_onboarding");
        assert!(spec.parameters.is_object());
    }
}
