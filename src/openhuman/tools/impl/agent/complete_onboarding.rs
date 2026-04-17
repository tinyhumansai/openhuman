//! Tool: complete_onboarding — inspects workspace setup status and, when
//! engagement criteria are met, finalizes the chat welcome flow.
//!
//! Used exclusively by the **welcome** agent.
//!
//! ## Normal flow
//!
//! 1. Welcome agent calls `check_status` on its first iteration. The
//!    tool returns a read-only JSON snapshot — **no side effects, no
//!    flag flips**. The snapshot includes a `ready_to_complete` bool,
//!    a `ready_to_complete_reason` string, and an `exchange_count`
//!    uint so the agent knows whether it may proceed.
//! 2. The agent converses with the user until `ready_to_complete` is
//!    `true` (either ≥ 3 back-and-forth exchanges, or at least one
//!    Composio integration connected).
//! 3. The agent calls `complete` to finalize. The `complete` action
//!    enforces the same criteria server-side and **rejects premature
//!    calls** with a descriptive error so the agent knows to keep
//!    conversing.
//!
//! ## Exchange count tracking
//!
//! A process-global [`AtomicU32`] (`WELCOME_EXCHANGE_COUNT`) counts
//! how many user messages have been dispatched to the welcome agent
//! this session. The dispatch layer calls
//! [`increment_welcome_exchange_count`] once per inbound message when
//! `chat_onboarding_completed` is still `false`. The counter is
//! intentionally process-local (not persisted) because the welcome
//! flow runs exactly once per fresh install; after `complete` flips
//! the flag the counter is never consulted again.

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult, ToolScope};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};

/// Process-global exchange counter for the welcome agent.
///
/// Incremented by [`increment_welcome_exchange_count`] (called from
/// the channel dispatch layer) once per inbound user message that
/// routes to the welcome agent. Used by [`check_status`] to surface
/// `exchange_count` and `ready_to_complete` to the agent, and by
/// [`complete`] to enforce the minimum-engagement guard.
static WELCOME_EXCHANGE_COUNT: AtomicU32 = AtomicU32::new(0);

/// Minimum number of welcome-agent exchanges required before the
/// `complete` action will accept a finalization request when no
/// Composio integrations are connected.
const MIN_EXCHANGES_TO_COMPLETE: u32 = 3;

/// Increment the welcome-agent exchange counter by one.
///
/// Called from the channel dispatch layer every time a user message
/// is routed to the welcome agent (i.e. when
/// `chat_onboarding_completed` is `false`). This is the only write
/// site — tool code and tests use
/// [`get_welcome_exchange_count`] to read it.
pub fn increment_welcome_exchange_count() {
    let prev = WELCOME_EXCHANGE_COUNT.fetch_add(1, Ordering::Relaxed);
    tracing::debug!(
        exchange_count = prev + 1,
        "[complete_onboarding] welcome exchange count incremented"
    );
}

/// Return the current welcome-agent exchange count (process-global).
///
/// Exposed for tests; production call sites should use the snapshot
/// fields returned by [`check_status`].
pub fn get_welcome_exchange_count() -> u32 {
    WELCOME_EXCHANGE_COUNT.load(Ordering::Relaxed)
}

/// Pure-logic helper: given an exchange count and the number of connected
/// Composio integrations, returns whether the engagement criteria for
/// `complete` are satisfied.
///
/// Extracted as a standalone function so tests can verify the criteria
/// without involving I/O (no config load, no Composio HTTP call).
pub(crate) fn engagement_criteria_met(exchange_count: u32, composio_connections: u32) -> bool {
    exchange_count >= MIN_EXCHANGES_TO_COMPLETE || composio_connections > 0
}

/// Build the user-facing error string for premature `complete` calls.
///
/// Kept as a pure helper so tests can lock wording and dynamic counters
/// without requiring config/auth/composio setup.
fn build_not_ready_to_complete_error(exchange_count: u32) -> String {
    let remaining = MIN_EXCHANGES_TO_COMPLETE.saturating_sub(exchange_count);
    format!(
        "Cannot complete onboarding yet: User hasn't connected any skills and \
         minimum exchanges not reached. Need at least \
         {MIN_EXCHANGES_TO_COMPLETE} back-and-forth exchanges (currently \
         {exchange_count}; {remaining} more needed) or at least one connected \
         Composio integration."
    )
}

/// Reset the welcome exchange counter to zero.
///
/// Exposed for tests that need a clean slate. **Do not call in
/// production code** — the counter is process-lifetime state and
/// resetting it would allow premature `complete` calls.
#[cfg(test)]
pub fn reset_welcome_exchange_count() {
    WELCOME_EXCHANGE_COUNT.store(0, Ordering::Relaxed);
}

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
        "Read the user's OpenHuman config snapshot and, when engagement \
         criteria are met, finalize the chat welcome flow.\n\
         \n\
         **action=\"check_status\"** — read-only. Returns a JSON object \
         describing the user's setup state. No side effects, no flag \
         flips. Call this ONCE on your first iteration with no other \
         parameters. Use the JSON to craft a personalised welcome \
         message and decide when to call `complete`.\n\
         \n\
         The returned JSON has this shape:\n\
         ```\n\
         {\n\
           \"authenticated\": true,                       // bool — JWT present\n\
           \"auth_source\": \"session_token\",            // \"session_token\" | \"legacy_api_key\" | null\n\
           \"default_model\": \"reasoning-v1\",           // string\n\
           \"channels_connected\": [\"telegram\"],        // string[] — connected messaging platforms\n\
           \"active_channel\": \"web\",                   // preferred channel for proactive messages\n\
           \"integrations\": {                             // bool flags for each capability\n\
             \"composio\": true,\n\
             \"browser\": false,\n\
             \"web_search\": true,\n\
             \"http_request\": false,\n\
             \"local_ai\": true\n\
           },\n\
           \"memory\": { \"backend\": \"sqlite\", \"auto_save\": true },\n\
           \"delegate_agents\": [\"researcher\", \"coder\"],\n\
           \"ui_onboarding_completed\": true,             // React wizard flag\n\
           \"chat_onboarding_completed\": false,          // still false until complete() succeeds\n\
           \"exchange_count\": 1,                         // how many user messages handled so far\n\
           \"ready_to_complete\": false,                  // true when criteria for complete() are met\n\
           \"ready_to_complete_reason\": \"fewer_than_min_exchanges_and_no_skills_connected\", // reason for readiness state\n\
           \"onboarding_status\": \"pending\"             // \"pending\" | \"already_complete\" | \"unauthenticated\"\n\
         }\n\
         ```\n\
         \n\
         The `onboarding_status` field describes the current state:\n\
         * `\"pending\"` — authenticated, conversation in progress. \
           Check `ready_to_complete` to know if you may call `complete`.\n\
         * `\"already_complete\"` — `chat_onboarding_completed` is \
           already `true`. Welcome the user as a returning visitor.\n\
         * `\"unauthenticated\"` — the user has no valid session. \
           Explain the auth problem, point them at the desktop login \
           flow, and stop. They will get routed back to welcome once \
           they authenticate.\n\
         \n\
         `ready_to_complete` is `true` when at least one of:\n\
         * The user has had at least 3 back-and-forth exchanges, or\n\
         * The user has connected at least one Composio integration.\n\
         \n\
         **action=\"complete\"** — finalize the welcome flow. Flips \
         `chat_onboarding_completed` to `true` and seeds recurring \
         cron jobs. Returns `\"ok\"` on success. **Rejects** (returns \
         an error) if called prematurely: the user must have either \
         ≥ 3 exchanges or at least one connected Composio integration. \
         Call only when `ready_to_complete` is `true` in the most \
         recent `check_status` snapshot."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["check_status", "complete"],
                    "description": "\"check_status\" → read-only JSON snapshot of the user's setup state. No side effects. \"complete\" → finalize the welcome flow; enforces minimum-engagement criteria and rejects premature calls."
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

/// Reads the user's config and returns a structured JSON snapshot.
///
/// **Read-only** — this function has no side effects. It does not flip
/// `chat_onboarding_completed`, does not seed cron jobs, and does not
/// call `complete`. The agent must call `complete` explicitly when it
/// judges the user ready.
///
/// The snapshot includes:
/// * All config flags the welcome message might mention.
/// * `exchange_count` — how many user messages have been dispatched
///   to the welcome agent so far (process-global counter).
/// * `ready_to_complete` — `true` when either exchange_count ≥
///   [`MIN_EXCHANGES_TO_COMPLETE`] or at least one Composio
///   integration is connected.
/// * `ready_to_complete_reason` — machine-friendly reason string that
///   explains why completion is ready (or blocked).
/// * `onboarding_status` — discriminator for the current state
///   (`"pending"`, `"already_complete"`, or `"unauthenticated"`).
async fn check_status() -> anyhow::Result<ToolResult> {
    let config = Config::load_or_init()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load config: {e}"))?;

    let (is_authenticated, _auth_source) = detect_auth(&config);

    let onboarding_status = if !is_authenticated {
        "unauthenticated"
    } else if config.chat_onboarding_completed {
        "already_complete"
    } else {
        "pending"
    };

    let exchange_count = get_welcome_exchange_count();
    let composio_connections = crate::openhuman::composio::fetch_connected_integrations(&config)
        .await
        .len() as u32;
    let ready_to_complete = is_authenticated
        && !config.chat_onboarding_completed
        && engagement_criteria_met(exchange_count, composio_connections);
    let ready_to_complete_reason = if !is_authenticated {
        "unauthenticated".to_string()
    } else if config.chat_onboarding_completed {
        "already_complete".to_string()
    } else if ready_to_complete {
        "criteria_met".to_string()
    } else {
        "fewer_than_min_exchanges_and_no_skills_connected".to_string()
    };

    tracing::debug!(
        authenticated = is_authenticated,
        onboarding_status,
        exchange_count,
        composio_connections,
        ready_to_complete,
        ready_to_complete_reason = ready_to_complete_reason.as_str(),
        "[complete_onboarding] check_status snapshot built"
    );

    let snapshot = build_status_snapshot(
        &config,
        onboarding_status,
        exchange_count,
        ready_to_complete,
        &ready_to_complete_reason,
    );
    let payload = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| anyhow::anyhow!("Failed to serialize status snapshot: {e}"))?;

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
/// onboarding flags). Shared between [`check_status`] (reactive,
/// called by the tool) and the proactive welcome path (fired on
/// `onboarding_completed` false→true).
///
/// * `onboarding_status` — `"pending"` | `"already_complete"` | `"unauthenticated"`
/// * `exchange_count` — messages dispatched to welcome agent this session
/// * `ready_to_complete` — whether the `complete` action would succeed
/// * `ready_to_complete_reason` — machine-friendly reason string for
///   the readiness state.
pub(crate) fn build_status_snapshot(
    config: &Config,
    onboarding_status: &str,
    exchange_count: u32,
    ready_to_complete: bool,
    ready_to_complete_reason: &str,
) -> Value {
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
        "exchange_count": exchange_count,
        "ready_to_complete": ready_to_complete,
        "ready_to_complete_reason": ready_to_complete_reason,
        "onboarding_status": onboarding_status,
    })
}

/// Finalize the welcome flow.
///
/// Flips `chat_onboarding_completed` to `true` and seeds recurring
/// proactive cron jobs. Returns `"ok"` on success.
///
/// ## Guard criteria
///
/// Rejects (returns a [`ToolResult::error`]) if the user has not yet
/// met the minimum engagement threshold:
///
/// * **Exchange count** — at least [`MIN_EXCHANGES_TO_COMPLETE`] user
///   messages have been dispatched to the welcome agent, **or**
/// * **Composio connection** — at least one Composio integration is
///   connected.
///
/// Either criterion is sufficient. The intent is that onboarding
/// completion reflects a real interaction, not a race between the
/// welcome agent and an auto-finalizer.
///
/// ## Auth requirement
///
/// Requires the user to be authenticated. If there is no valid session
/// JWT or legacy API key, the call is rejected with an explanation so
/// the agent can instruct the user to log in.
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
        tracing::debug!("[complete_onboarding] complete rejected — user not authenticated");
        return Ok(ToolResult::error(
            "Cannot complete onboarding: the user is not authenticated. \
             Please guide them to log in via the desktop login flow first.",
        ));
    }

    // ── Engagement guard ──────────────────────────────────────────
    let exchange_count = get_welcome_exchange_count();
    let composio_connections = crate::openhuman::composio::fetch_connected_integrations(&config)
        .await
        .len() as u32;

    let criteria_met = engagement_criteria_met(exchange_count, composio_connections);

    tracing::debug!(
        exchange_count,
        composio_connections,
        criteria_met,
        "[complete_onboarding] engagement guard check"
    );

    if !criteria_met {
        return Ok(ToolResult::error(build_not_ready_to_complete_error(
            exchange_count,
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
        exchange_count,
        composio_connections,
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
        assert!(schema["properties"]["action"].is_object());
        assert_eq!(schema["required"], serde_json::json!(["action"]));
    }

    #[test]
    fn build_status_snapshot_carries_new_fields() {
        // A default Config is "bare install" — no channels, no
        // integrations. This test locks in the JSON shape the welcome
        // agent's prompt.md depends on. Dropping or renaming a field
        // breaks this test loudly.
        let config = Config::default();
        let snapshot = build_status_snapshot(
            &config,
            "pending",
            0,
            false,
            "fewer_than_min_exchanges_and_no_skills_connected",
        );

        assert_eq!(snapshot["onboarding_status"], "pending");
        assert_eq!(snapshot["exchange_count"], 0);
        assert_eq!(snapshot["ready_to_complete"], false);
        assert_eq!(
            snapshot["ready_to_complete_reason"],
            "fewer_than_min_exchanges_and_no_skills_connected"
        );
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
    fn build_status_snapshot_ready_to_complete_reflected() {
        let config = Config::default();
        let snapshot = build_status_snapshot(&config, "pending", 5, true, "criteria_met");
        assert_eq!(snapshot["ready_to_complete"], true);
        assert_eq!(snapshot["ready_to_complete_reason"], "criteria_met");
        assert_eq!(snapshot["exchange_count"], 5);
        assert_eq!(snapshot["onboarding_status"], "pending");
    }

    #[test]
    fn build_status_snapshot_unauthenticated_reason_reflected() {
        let config = Config::default();
        let snapshot =
            build_status_snapshot(&config, "unauthenticated", 0, false, "unauthenticated");
        assert_eq!(snapshot["ready_to_complete"], false);
        assert_eq!(snapshot["ready_to_complete_reason"], "unauthenticated");
        assert_eq!(snapshot["onboarding_status"], "unauthenticated");
    }

    #[test]
    fn detect_auth_on_default_config_is_unauthenticated() {
        let config = Config::default();
        let (is_auth, source) = detect_auth(&config);
        assert!(!is_auth);
        assert!(source.is_null());
    }

    // ── exchange counter ──────────────────────────────────────────────────────

    #[test]
    fn exchange_counter_increments_and_resets() {
        reset_welcome_exchange_count();
        assert_eq!(get_welcome_exchange_count(), 0);
        increment_welcome_exchange_count();
        assert_eq!(get_welcome_exchange_count(), 1);
        increment_welcome_exchange_count();
        increment_welcome_exchange_count();
        assert_eq!(get_welcome_exchange_count(), 3);
        reset_welcome_exchange_count();
        assert_eq!(get_welcome_exchange_count(), 0);
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
        assert!(
            desc.contains("ready_to_complete"),
            "description should mention ready_to_complete"
        );
        assert!(
            desc.contains("ready_to_complete_reason"),
            "description should mention ready_to_complete_reason"
        );
    }

    #[test]
    fn premature_complete_error_mentions_skills_and_exchanges() {
        let msg = build_not_ready_to_complete_error(1);
        assert!(
            msg.contains("User hasn't connected any skills and minimum exchanges not reached"),
            "expected issue #596 wording in error message, got: {msg}"
        );
        assert!(
            msg.contains("currently 1; 2 more needed"),
            "expected dynamic exchange counters in error message, got: {msg}"
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
        if let Ok(r) = result {
            assert!(
                !r.output().contains("Unknown action"),
                "missing action should default to check_status, not 'Unknown action'"
            );
        }
    }

    // ── guard: engagement_criteria_met ───────────────────────────────────────

    /// Zero exchanges, no composio → criteria NOT met.
    #[test]
    fn criteria_not_met_zero_exchanges_no_composio() {
        assert!(!engagement_criteria_met(0, 0));
    }

    /// One exchange below threshold, no composio → criteria NOT met.
    #[test]
    fn criteria_not_met_below_threshold() {
        assert!(!engagement_criteria_met(MIN_EXCHANGES_TO_COMPLETE - 1, 0));
    }

    /// Exactly at the exchange threshold, no composio → criteria MET.
    #[test]
    fn criteria_met_at_exchange_threshold() {
        assert!(engagement_criteria_met(MIN_EXCHANGES_TO_COMPLETE, 0));
    }

    /// Above the exchange threshold → criteria MET.
    #[test]
    fn criteria_met_above_threshold() {
        assert!(engagement_criteria_met(MIN_EXCHANGES_TO_COMPLETE + 5, 0));
    }

    /// Zero exchanges but one composio connection → criteria MET
    /// (composio is an OR shortcut, not AND).
    #[test]
    fn criteria_met_via_composio_zero_exchanges() {
        assert!(engagement_criteria_met(0, 1));
    }

    /// One exchange and one composio connection → criteria MET.
    #[test]
    fn criteria_met_via_composio_with_exchanges() {
        assert!(engagement_criteria_met(1, 1));
    }

    /// Exchange count at u32::MAX — no panic, criteria met.
    #[test]
    fn criteria_met_saturating_exchange_count() {
        assert!(engagement_criteria_met(u32::MAX, 0));
    }
}
