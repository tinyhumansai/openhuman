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

use super::onboarding_status::{build_status_snapshot, compute_state};

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
        "Read-only JSON snapshot of the user's workspace setup state. \
         No side effects, no flag flips. Takes no arguments. Call this \
         ONCE on your first iteration to craft a personalised welcome \
         and decide when to call `complete_onboarding`.\n\
         \n\
         The returned JSON has this shape:\n\
         ```\n\
         {\n\
           \"authenticated\": true,                       // bool — JWT present\n\
           \"auth_source\": \"session_token\",            // \"session_token\" | null\n\
           \"default_model\": \"reasoning-v1\",           // string\n\
           \"channels_connected\": [\"telegram\"],        // string[] — connected messaging platforms\n\
           \"active_channel\": \"web\",                   // preferred channel for proactive messages\n\
           \"integrations\": {                             // bool flags for each capability\n\
             \"composio\": true,\n\
             \"browser\": false,\n\
             \"web_search\": true,\n\
             \"http_request\": true,\n\
             \"local_ai\": true\n\
           },\n\
           \"composio_connected_toolkits\": [\"gmail\"], // Composio toolkits the user has authorised\n\
           \"webview_logins\": {                          // per-provider CEF cookie presence\n\
             \"gmail\": true, \"whatsapp\": false, \"telegram\": false,\n\
             \"slack\": false, \"discord\": false, \"linkedin\": false,\n\
             \"zoom\": false, \"google_messages\": false\n\
           },\n\
           \"memory\": { \"backend\": \"sqlite\", \"auto_save\": true },\n\
           \"delegate_agents\": [\"researcher\", \"coder\"],\n\
           \"ui_onboarding_completed\": true,             // React wizard flag\n\
           \"chat_onboarding_completed\": false,          // still false until complete_onboarding succeeds\n\
           \"exchange_count\": 1,                         // how many user messages handled so far\n\
           \"ready_to_complete\": false,                  // true when criteria for complete_onboarding are met\n\
           \"ready_to_complete_reason\": \"fewer_than_min_exchanges_and_no_skills_connected\",\n\
           \"onboarding_status\": \"pending\"             // \"pending\" | \"already_complete\" | \"unauthenticated\"\n\
         }\n\
         ```\n\
         \n\
         **Two fields matter for what to offer next:**\n\
         * `composio_connected_toolkits` lists OAuth-authorised skills \
           (e.g. gmail). Don't re-pitch a toolkit that's already here.\n\
         * `webview_logins` reports whether the embedded browser \
           already has a live session cookie for each provider. A \
           `true` means the user is signed in to that webview — don't \
           ask them to log in again, just reference it.\n\
         \n\
         `ready_to_complete` is `true` when at least one of:\n\
         * The user has had at least 3 back-and-forth exchanges, or\n\
         * The user has connected at least one Composio integration.\n\
         \n\
         `onboarding_status`:\n\
         * `\"pending\"` — authenticated, conversation in progress. \
           Check `ready_to_complete` to know if you may call \
           `complete_onboarding`.\n\
         * `\"already_complete\"` — `chat_onboarding_completed` is \
           already `true`. Welcome the user as a returning visitor.\n\
         * `\"unauthenticated\"` — the user has no valid session. \
           Explain the auth problem, point them at the desktop login \
           flow, and stop."
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

    let snapshot = build_status_snapshot(
        &config,
        state.onboarding_status,
        state.exchange_count,
        state.ready_to_complete,
        &state.ready_to_complete_reason,
        &state.composio_connected_toolkits,
        webview_logins,
    );
    let payload = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| anyhow::anyhow!("Failed to serialize status snapshot: {e}"))?;

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
    fn description_documents_new_fields() {
        let desc = CheckOnboardingStatusTool::new().description().to_string();
        assert!(
            desc.contains("composio_connected_toolkits"),
            "description should document the composio toolkits field"
        );
        assert!(
            desc.contains("webview_logins"),
            "description should document the webview logins field"
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
