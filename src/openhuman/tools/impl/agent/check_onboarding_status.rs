//! Tool: `check_onboarding_status` — read-only snapshot of the user's
//! workspace setup state for the welcome agent.
//!
//! Pairs with [`super::complete_onboarding`] — that tool finalizes the
//! flow, this one reports what's already in place so the agent can
//! craft a personalized welcome and decide when to finalize.
//!
//! No side effects. No flag flips. Takes no arguments.

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult, ToolScope};
use async_trait::async_trait;
use serde_json::{json, Value};

use super::onboarding_status::{compute_state, format_status_markdown};

pub struct CheckOnboardingStatusTool;

impl Default for CheckOnboardingStatusTool {
    fn default() -> Self {
        Self::new()
    }
}

impl CheckOnboardingStatusTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for CheckOnboardingStatusTool {
    fn name(&self) -> &str {
        "check_onboarding_status"
    }

    fn description(&self) -> &str {
        "Read-only markdown snapshot of the user's workspace setup. \
         No side effects, no flag flips. Takes no arguments. Call this \
         ONCE on your first iteration to craft a personalised welcome \
         and decide when to call `complete_onboarding`.\n\
         \n\
         Returns a compact markdown bulleted list:\n\
         ```\n\
         # Onboarding Status\n\
         \n\
         - **status:** pending (ready_to_complete: false, reason: fewer_than_min_exchanges_and_no_skills_connected)\n\
         - **auth:** yes (session_token)\n\
         - **exchanges:** 1\n\
         - **composio:** gmail\n\
         - **webview logins:** gmail\n\
         - **channels:** telegram (active: web)\n\
         - **flags:** ui_onboarding=true, chat_onboarding=false\n\
         ```\n\
         \n\
         `composio` and `webview logins` are only listed when something is \
         connected/active — an empty list means none. Don't re-pitch a \
         toolkit that already appears under `composio`. A name under \
         `webview logins` means the embedded browser already has a live \
         session cookie for that provider; reference it instead of asking \
         them to log in again.\n\
         \n\
         `ready_to_complete` flips true when at least one of:\n\
         * The user has had at least 3 back-and-forth exchanges, or\n\
         * The user has connected at least one Composio integration.\n\
         \n\
         `status`:\n\
         * `pending` — authenticated, conversation in progress. Check \
           `ready_to_complete` before calling `complete_onboarding`.\n\
         * `already_complete` — `chat_onboarding_completed` is already \
           true. Welcome the user as a returning visitor.\n\
         * `unauthenticated` — no valid session. Explain the auth problem, \
           point them at the desktop login flow, and stop."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn scope(&self) -> ToolScope {
        ToolScope::AgentOnly
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        tracing::debug!("[check_onboarding_status] execute");
        check_status().await
    }
}

/// Reads the user's config and returns a structured JSON snapshot.
///
/// Read-only. Combines config flags, the process-global welcome
/// exchange counter, the Composio connected-toolkits list, and the
/// per-provider webview login heuristic (shared CEF cookie probe) into
/// one payload the welcome agent consumes in a single tool call.
async fn check_status() -> anyhow::Result<ToolResult> {
    let config = Config::load_or_init()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load config: {e}"))?;

    let state = compute_state(&config).await;
    let webview_logins = crate::openhuman::webview_accounts::detect_webview_logins();

    tracing::debug!(
        authenticated = state.is_authenticated,
        onboarding_status = state.onboarding_status,
        exchange_count = state.exchange_count,
        composio_connections = state.composio_connected_toolkits.len(),
        ready_to_complete = state.ready_to_complete,
        "[check_onboarding_status] snapshot built"
    );

    let payload = format_status_markdown(
        &config,
        state.onboarding_status,
        state.exchange_count,
        state.ready_to_complete,
        &state.ready_to_complete_reason,
        &state.composio_connected_toolkits,
        &webview_logins,
    );

    Ok(ToolResult::success(payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_metadata() {
        let tool = CheckOnboardingStatusTool::new();
        assert_eq!(tool.name(), "check_onboarding_status");
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
        assert_eq!(tool.scope(), ToolScope::AgentOnly);
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema.get("required").is_none());
    }

    #[test]
    fn description_documents_markdown_fields() {
        let desc = CheckOnboardingStatusTool::new().description().to_string();
        assert!(
            desc.contains("**composio:**"),
            "description should document the composio bullet"
        );
        assert!(
            desc.contains("**webview logins:**"),
            "description should document the webview logins bullet"
        );
        assert!(desc.contains("ready_to_complete"));
    }

    #[test]
    fn spec_roundtrip() {
        let tool = CheckOnboardingStatusTool::new();
        let spec = tool.spec();
        assert_eq!(spec.name, "check_onboarding_status");
    }
}
