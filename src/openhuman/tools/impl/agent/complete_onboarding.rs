//! Tool: complete_onboarding — inspects workspace setup status and (when
//! the user is authenticated) auto-finalizes the chat welcome flow.
//!
//! Used exclusively by the **welcome** agent. There is only one normal
//! path: call `check_status` once. The tool returns a structured JSON
//! snapshot of the user's config AND, if the user is authenticated,
//! flips `chat_onboarding_completed = true` + seeds proactive cron jobs
//! as a side effect. The welcome agent then drafts a personalised
//! welcome message based on the JSON in its next iteration. No second
//! tool call. No race conditions. No way to forget the flip.
//!
//! The legacy `complete` action is kept as a manual override for admin
//! tools and tests but the welcome agent should never call it directly.

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult, ToolScope};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct CompleteOnboardingTool;

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
        "Read the user's OpenHuman config snapshot and auto-finalize the \
         chat welcome flow when the user is authenticated. The welcome \
         agent's single tool call.\n\
         \n\
         **action=\"check_status\"** — returns a JSON object describing \
         the user's setup state and (as a side effect when the user has \
         a valid session JWT) flips `chat_onboarding_completed` to true \
         + seeds proactive agent cron jobs. Call this ONCE on your first \
         iteration with no other parameters. Use the JSON to draft a \
         personalised welcome message in your second iteration.\n\
         \n\
         The returned JSON has this shape:\n\
         ```\n\
         {\n\
           \"authenticated\": true,                       // bool — JWT present\n\
           \"auth_source\": \"session_token\",            // \"session_token\" | \"legacy_api_key\" | null\n\
           \"default_model\": \"reasoning-v1\",           // string\n\
           \"channels_connected\": [\"telegram\"],         // string[] — connected messaging platforms\n\
           \"active_channel\": \"web\",                   // string — preferred channel for proactive messages\n\
           \"integrations\": {                             // bool flags for each capability\n\
             \"composio\": true,\n\
             \"browser\": false,\n\
             \"web_search\": true,\n\
             \"http_request\": false,\n\
             \"local_ai\": true\n\
           },\n\
           \"memory\": { \"backend\": \"sqlite\", \"auto_save\": true },\n\
           \"delegate_agents\": [\"researcher\", \"coder\"], // configured custom agents\n\
           \"ui_onboarding_completed\": true,             // React wizard flag\n\
           \"chat_onboarding_completed\": true,           // POST-finalize value\n\
           \"finalize_action\": \"flipped\"                // \"flipped\" | \"already_complete\" | \"skipped_no_auth\"\n\
         }\n\
         ```\n\
         \n\
         The `finalize_action` field tells you what side effect this \
         call performed:\n\
         * `\"flipped\"` — the user was authenticated and the chat flow \
           was previously incomplete. Flag was just flipped to true and \
           cron jobs were seeded. Welcome the user; the next chat turn \
           will route to the orchestrator.\n\
         * `\"already_complete\"` — the user was authenticated and the \
           chat flow was already complete from a prior call. No state \
           change. Welcome the user (this is a re-entry case).\n\
         * `\"skipped_no_auth\"` — the user is not authenticated, so \
           the flag was NOT flipped. The welcome agent should explain \
           the auth problem to the user, point them at the desktop \
           login flow, and stop. The next chat turn will re-route to \
           welcome (because the flag is still false), so they'll get \
           another chance once they log in.\n\
         \n\
         Use the JSON fields directly when drafting your welcome \
         message. Don't quote the JSON back to the user — translate \
         the field values into natural prose tailored to what they \
         have and don't have. The status fields are a fact source, \
         not a draft.\n\
         \n\
         **action=\"complete\"** — legacy manual finalize-only path. \
         Flips `chat_onboarding_completed` to true unconditionally and \
         seeds cron jobs without producing a status report. Returns \
         the literal token \"ok\". Welcome agent should never call \
         this; use `check_status` instead which performs the same \
         finalize as a side effect under proper auth gating. Kept for \
         backward compatibility with admin tools and tests."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["check_status", "complete"],
                    "description": "\"check_status\" → return JSON config snapshot AND auto-finalize the chat welcome flow when authenticated (welcome agent's only call). \"complete\" → legacy manual finalize-only; do not use from welcome agent."
                }
            },
            "required": ["action"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn scope(&self) -> ToolScope {
        ToolScope::AgentOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("check_status");

        tracing::debug!("[complete_onboarding] action={action}");

        match action {
            "check_status" => check_status().await,
            "complete" => complete().await,
            other => Ok(ToolResult::error(format!(
                "Unknown action \"{other}\". Use \"check_status\" or \"complete\"."
            ))),
        }
    }
}

/// Reads the user's config, builds a structured JSON snapshot, and
/// (when the user is authenticated) flips `chat_onboarding_completed`
/// + seeds proactive cron jobs as a side effect of the read.
///
/// This is the welcome agent's single tool call. The contract is:
///
/// 1. **Read** — load `Config`, check JWT via
///    `crate::api::jwt::get_session_token`, gather every config flag
///    the welcome message might mention.
/// 2. **Auto-finalize** — if the user is authenticated AND
///    `chat_onboarding_completed` is currently `false`, flip it to
///    `true` and spawn the proactive agent cron seeder. If the user
///    is NOT authenticated, leave the flag alone (the welcome agent
///    will explain the auth problem and the next chat turn will
///    re-run welcome).
/// 3. **Return** — JSON object with all the config fields, the
///    POST-finalize `chat_onboarding_completed` value, and a
///    `finalize_action` discriminator describing what side effect
///    happened (`flipped`, `already_complete`, or `skipped_no_auth`).
///
/// The welcome agent uses the JSON to draft a personalised welcome
/// message in the next iteration. No second tool call needed.
async fn check_status() -> anyhow::Result<ToolResult> {
    let mut config = Config::load_or_init()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load config: {e}"))?;

    let (is_authenticated, _auth_source) = detect_auth(&config);

    // ── Auto-finalize side effect ─────────────────────────────────
    //
    // When the user is authenticated AND the chat welcome flow has
    // not yet completed, delegate to the legacy `complete()` action
    // — single source of truth for "what does finalize mean". This
    // is the welcome → orchestrator handoff: after the flag flips,
    // the dispatch layer routes future chat turns to the orchestrator
    // instead of the welcome agent (see
    // `web.rs::build_session_agent` and
    // `dispatch.rs::resolve_target_agent`).
    //
    // We discard `complete()`'s `ToolResult::success("ok")` return
    // value because the caller (check_status) is producing its own
    // JSON snapshot — the side effect is the only thing we want.
    // After the call we mirror the flip into our local `config`
    // variable so the JSON snapshot below reflects the post-finalize
    // state (the disk has been updated, but our in-memory copy was
    // loaded before the flip).
    let finalize_action = if !is_authenticated {
        "skipped_no_auth"
    } else if config.chat_onboarding_completed {
        "already_complete"
    } else {
        let _ = complete().await?;
        config.chat_onboarding_completed = true;
        tracing::info!(
            "[complete_onboarding] chat welcome flow auto-finalized via check_status (delegated to complete())"
        );
        "flipped"
    };

    let snapshot = build_status_snapshot(&config, finalize_action);
    let payload = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| anyhow::anyhow!("Failed to serialize status snapshot: {e}"))?;

    tracing::debug!(
        "[complete_onboarding] check_status returned authenticated={} finalize_action={} chars={}",
        is_authenticated,
        finalize_action,
        payload.len()
    );

    Ok(ToolResult::success(payload))
}

/// Detect whether the user is authenticated for the welcome flow.
///
/// Two possible auth sources, in priority order:
///
/// 1. The `app-session:default` profile in `auth-profiles.json` —
///    the canonical inference credential, populated by the desktop
///    OAuth deep-link flow's `exchange_token` Rust command. This is
///    where every production inference RPC reads from.
/// 2. `config.api_key` — legacy free-form provider key field, kept
///    for CI / dev setups that bypass the desktop login flow.
///
/// Either one counts as "authenticated".
///
/// Returned as `(is_authenticated, auth_source_json)` so callers can
/// both gate behaviour on the bool and embed the source label in a
/// JSON payload without rebuilding the logic.
pub(crate) fn detect_auth(config: &Config) -> (bool, Value) {
    let has_session_jwt = crate::api::jwt::get_session_token(config)
        .ok()
        .flatten()
        .is_some_and(|t| !t.is_empty());
    let has_legacy_api_key = config.api_key.as_ref().is_some_and(|k| !k.is_empty());
    let is_authenticated = has_session_jwt || has_legacy_api_key;
    let auth_source: Value = if has_session_jwt {
        Value::String("session_token".to_string())
    } else if has_legacy_api_key {
        Value::String("legacy_api_key".to_string())
    } else {
        Value::Null
    };
    (is_authenticated, auth_source)
}

/// Build the structured JSON snapshot that the welcome agent consumes.
///
/// The snapshot describes the user's workspace setup (connected
/// channels, integrations, delegate agents, memory settings, and
/// onboarding flags) and embeds a `finalize_action` discriminator so
/// the agent knows which message framing to use. Shared between
/// [`check_status`] (reactive, called by the tool) and the proactive
/// welcome path (fired on `onboarding_completed` false→true).
///
/// The caller is responsible for computing `finalize_action` —
/// typically `"flipped"`, `"already_complete"`, or `"skipped_no_auth"`.
pub(crate) fn build_status_snapshot(config: &Config, finalize_action: &str) -> Value {
    let (is_authenticated, auth_source) = detect_auth(config);

    // ── Connected messaging channels ──────────────────────────────
    let mut channels_connected: Vec<&str> = Vec::new();
    if config.channels_config.telegram.is_some() {
        channels_connected.push("telegram");
    }
    if config.channels_config.discord.is_some() {
        channels_connected.push("discord");
    }
    if config.channels_config.slack.is_some() {
        channels_connected.push("slack");
    }
    if config.channels_config.mattermost.is_some() {
        channels_connected.push("mattermost");
    }
    if config.channels_config.email.is_some() {
        channels_connected.push("email");
    }
    if config.channels_config.whatsapp.is_some() {
        channels_connected.push("whatsapp");
    }
    if config.channels_config.signal.is_some() {
        channels_connected.push("signal");
    }
    if config.channels_config.matrix.is_some() {
        channels_connected.push("matrix");
    }
    if config.channels_config.imessage.is_some() {
        channels_connected.push("imessage");
    }
    if config.channels_config.irc.is_some() {
        channels_connected.push("irc");
    }
    if config.channels_config.lark.is_some() {
        channels_connected.push("lark");
    }
    if config.channels_config.dingtalk.is_some() {
        channels_connected.push("dingtalk");
    }
    if config.channels_config.linq.is_some() {
        channels_connected.push("linq");
    }
    if config.channels_config.qq.is_some() {
        channels_connected.push("qq");
    }

    let composio_enabled = config.composio.enabled
        && config
            .composio
            .api_key
            .as_ref()
            .is_some_and(|k| !k.is_empty());

    let delegate_agents: Vec<&str> = config.agents.keys().map(|s| s.as_str()).collect();

    json!({
        "authenticated": is_authenticated,
        "auth_source": auth_source,
        "default_model": config
            .default_model
            .as_deref()
            .unwrap_or(crate::openhuman::config::DEFAULT_MODEL),
        "channels_connected": channels_connected,
        "active_channel": config
            .channels_config
            .active_channel
            .as_deref()
            .unwrap_or("web"),
        "integrations": {
            "composio": composio_enabled,
            "browser": config.browser.enabled,
            "web_search": config.web_search.enabled,
            "http_request": config.http_request.enabled,
            "local_ai": config.local_ai.enabled,
        },
        "memory": {
            "backend": config.memory.backend,
            "auto_save": config.memory.auto_save,
        },
        "delegate_agents": delegate_agents,
        "ui_onboarding_completed": config.onboarding_completed,
        "chat_onboarding_completed": config.chat_onboarding_completed,
        "finalize_action": finalize_action,
    })
}

/// Legacy manual finalize-only path. Flips `chat_onboarding_completed`
/// to true unconditionally (no auth check) and seeds proactive cron
/// jobs. Welcome agent should NOT call this — use `check_status`
/// instead, which performs the same finalize as a side effect under
/// proper auth gating. Kept for backward compatibility with admin
/// tools and tests.
async fn complete() -> anyhow::Result<ToolResult> {
    let mut config = Config::load_or_init()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load config: {e}"))?;

    if config.chat_onboarding_completed {
        tracing::debug!("[complete_onboarding] chat welcome flow already completed — no-op");
        return Ok(ToolResult::success("ok"));
    }

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
        "[complete_onboarding] chat welcome flow marked complete via legacy complete action"
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
        assert!(schema["properties"]["action"].is_object());
        assert_eq!(schema["required"], serde_json::json!(["action"]));
    }

    #[test]
    fn build_status_snapshot_carries_finalize_action_and_core_fields() {
        // A default Config is "bare install" — no channels, no
        // integrations. This test locks in the JSON shape the welcome
        // agent's prompt.md depends on, and is the contract shared
        // between `check_status` (reactive) and the proactive welcome
        // path — if a future refactor drops or renames a field, this
        // fails loudly.
        let config = Config::default();
        let snapshot = build_status_snapshot(&config, "flipped");

        assert_eq!(snapshot["finalize_action"], "flipped");
        assert_eq!(snapshot["chat_onboarding_completed"], false);
        assert_eq!(snapshot["ui_onboarding_completed"], false);
        assert_eq!(snapshot["active_channel"], "web");
        assert_eq!(
            snapshot["channels_connected"]
                .as_array()
                .expect("channels_connected is an array")
                .len(),
            0,
            "default Config should report zero connected channels"
        );
        assert!(snapshot["integrations"].is_object());
        assert!(snapshot["memory"].is_object());
        // Every integration flag present so the welcome prompt can
        // branch on bare-install handling without optional-chain checks.
        for key in [
            "composio",
            "browser",
            "web_search",
            "http_request",
            "local_ai",
        ] {
            assert!(
                snapshot["integrations"][key].is_boolean(),
                "integrations.{key} must be a bool"
            );
        }
    }

    #[test]
    fn detect_auth_on_default_config_is_unauthenticated() {
        let config = Config::default();
        let (is_auth, source) = detect_auth(&config);
        assert!(!is_auth);
        assert!(source.is_null());
    }

    // ── description ───────────────────────────────────────────────────────────

    #[test]
    fn description_mentions_key_actions() {
        let tool = CompleteOnboardingTool::new();
        let desc = tool.description();
        assert!(!desc.is_empty());
        assert!(
            desc.contains("check_status"),
            "description should mention check_status"
        );
        assert!(
            desc.contains("complete"),
            "description should mention complete"
        );
    }

    // ── schema enum values ────────────────────────────────────────────────────

    #[test]
    fn schema_action_enum_has_both_values() {
        let tool = CompleteOnboardingTool::new();
        let schema = tool.parameters_schema();
        let enum_vals = schema["properties"]["action"]["enum"]
            .as_array()
            .expect("action enum should be an array");
        let names: Vec<&str> = enum_vals.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(
            names.contains(&"check_status"),
            "enum should contain check_status"
        );
        assert!(names.contains(&"complete"), "enum should contain complete");
    }

    // ── spec roundtrip ────────────────────────────────────────────────────────

    #[test]
    fn spec_roundtrip() {
        let tool = CompleteOnboardingTool::new();
        let spec = tool.spec();
        assert_eq!(spec.name, "complete_onboarding");
        assert!(spec.parameters.is_object());
    }

    // ── execute: unknown action ───────────────────────────────────────────────

    #[tokio::test]
    async fn execute_unknown_action_returns_error() {
        let tool = CompleteOnboardingTool::new();
        let result = tool
            .execute(serde_json::json!({"action": "unknown_action"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.output().contains("Unknown action"),
            "error message should contain 'Unknown action', got: {}",
            result.output()
        );
    }

    // ── execute: missing action defaults to check_status ─────────────────────

    #[tokio::test]
    async fn execute_missing_action_defaults_to_check_status() {
        // When action is absent it defaults to "check_status", which calls
        // Config::load_or_init() — that may succeed or fail depending on env,
        // but it should not return the "Unknown action" error.
        let tool = CompleteOnboardingTool::new();
        let result = tool.execute(serde_json::json!({})).await;
        // Either Ok (config loaded) or Err (config failed) — just must not be
        // the "Unknown action" variant.
        if let Ok(r) = result {
            assert!(
                !r.output().contains("Unknown action"),
                "missing action should default to check_status, not 'Unknown action'"
            );
        }
        // Err from config loading is also acceptable here.
    }
}
