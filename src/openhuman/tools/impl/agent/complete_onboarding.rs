//! Tool: complete_onboarding — inspects workspace setup status and marks onboarding done.
//!
//! Used exclusively by the **welcome** agent. On `action: "check_status"` it
//! reads the current config and app state to report what the user has and
//! hasn't configured. On `action: "complete"` it flips
//! `config.onboarding_completed = true` and seeds the default proactive
//! cron jobs (morning briefing, etc.).

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult, ToolScope};
use async_trait::async_trait;
use serde_json::json;

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
        "Check onboarding status or mark onboarding as complete. \
         Use action=\"check_status\" to inspect what the user has configured \
         (API key, channels, integrations, memory, etc.) and what still needs \
         attention. Use action=\"complete\" to finalize onboarding once the \
         user is ready."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["check_status", "complete"],
                    "description": "\"check_status\" to inspect current setup, \"complete\" to mark onboarding done."
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

/// Reads the current config and produces a human-readable status report.
async fn check_status() -> anyhow::Result<ToolResult> {
    let config = Config::load_or_init()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load config: {e}"))?;

    let mut report = String::new();
    report.push_str("## Onboarding Status\n\n");

    // ── Core setup ──────────────────────────────────────────────────
    report.push_str("### Core\n");

    // Authentication can come from EITHER:
    // 1. `config.api_key` — the legacy free-form provider key field,
    //    usually `None` for users who go through the desktop login
    //    flow (the deep-link OAuth handshake doesn't write here);
    // 2. The `app-session:default` profile in `auth-profiles.json`,
    //    populated by `exchange_token` after the desktop OAuth flow
    //    completes. This is the canonical inference credential — it
    //    holds the openhuman backend session JWT, encrypted with
    //    `.secret_key`. Every production inference RPC reads from
    //    here via `crate::api::jwt::get_session_token`.
    //
    // Previously this status check looked only at `config.api_key`,
    // which meant any user logged in through the desktop OAuth flow
    // (the only way to get an account today) was reported as having
    // "no API key" because their JWT lives in the auth profile store,
    // not the config TOML. The welcome agent then refused to call
    // `complete_onboarding(complete)` and re-ran on every chat turn,
    // even though the user was fully authenticated. Fix: check both
    // sources and report authenticated when either is present.
    let has_legacy_api_key = config.api_key.as_ref().map_or(false, |k| !k.is_empty());
    let has_session_jwt = crate::api::jwt::get_session_token(&config)
        .ok()
        .flatten()
        .is_some_and(|t| !t.is_empty());
    let is_authenticated = has_legacy_api_key || has_session_jwt;
    report.push_str(&format!(
        "- Authentication: {}\n",
        if is_authenticated {
            if has_session_jwt {
                "configured ✓ (session token from desktop login)"
            } else {
                "configured ✓ (legacy api_key)"
            }
        } else {
            "**missing** — log in via the desktop app or set `api_key` in config to enable inference"
        }
    ));
    report.push_str(&format!(
        "- Default model: {}\n",
        config
            .default_model
            .as_deref()
            .unwrap_or(crate::openhuman::config::DEFAULT_MODEL)
    ));
    // Two distinct flags after the chat / UI split:
    // * `onboarding_completed` — React wizard (Tauri desktop UI) gate
    // * `chat_onboarding_completed` — welcome agent's own gate, which
    //   determines whether YOU (the welcome agent reading this report)
    //   are routed to handle the next chat turn. Use the chat flag,
    //   not the UI flag, when deciding whether your work here is done.
    report.push_str(&format!(
        "- UI onboarding wizard completed: {}\n",
        config.onboarding_completed
    ));
    report.push_str(&format!(
        "- Chat welcome flow completed: {}\n",
        config.chat_onboarding_completed
    ));

    // ── Channels ────────────────────────────────────────────────────
    report.push_str("\n### Channels\n");
    let mut connected_channels: Vec<&str> = Vec::new();
    if config.channels_config.telegram.is_some() {
        connected_channels.push("Telegram");
    }
    if config.channels_config.discord.is_some() {
        connected_channels.push("Discord");
    }
    if config.channels_config.slack.is_some() {
        connected_channels.push("Slack");
    }
    if config.channels_config.mattermost.is_some() {
        connected_channels.push("Mattermost");
    }
    if config.channels_config.email.is_some() {
        connected_channels.push("Email");
    }
    if config.channels_config.whatsapp.is_some() {
        connected_channels.push("WhatsApp");
    }
    if config.channels_config.signal.is_some() {
        connected_channels.push("Signal");
    }
    if config.channels_config.matrix.is_some() {
        connected_channels.push("Matrix");
    }
    if config.channels_config.imessage.is_some() {
        connected_channels.push("iMessage");
    }
    if config.channels_config.irc.is_some() {
        connected_channels.push("IRC");
    }
    if config.channels_config.lark.is_some() {
        connected_channels.push("Lark");
    }
    if config.channels_config.dingtalk.is_some() {
        connected_channels.push("DingTalk");
    }
    if config.channels_config.linq.is_some() {
        connected_channels.push("Linq");
    }
    if config.channels_config.qq.is_some() {
        connected_channels.push("QQ");
    }
    if connected_channels.is_empty() {
        report.push_str("- No messaging channels connected yet (Telegram, Discord, Slack, etc.)\n");
    } else {
        report.push_str(&format!("- Connected: {}\n", connected_channels.join(", ")));
    }
    report.push_str(&format!(
        "- Active channel for proactive messages: {}\n",
        config
            .channels_config
            .active_channel
            .as_deref()
            .unwrap_or("web (default)")
    ));

    // ── Integrations ────────────────────────────────────────────────
    report.push_str("\n### Integrations\n");
    let has_composio = config.composio.enabled
        && config
            .composio
            .api_key
            .as_ref()
            .map_or(false, |k| !k.is_empty());
    report.push_str(&format!(
        "- Composio (1000+ OAuth apps): {}\n",
        if has_composio {
            "enabled ✓"
        } else {
            "not configured"
        }
    ));
    report.push_str(&format!(
        "- Browser automation: {}\n",
        if config.browser.enabled {
            "enabled ✓"
        } else {
            "disabled"
        }
    ));
    report.push_str(&format!(
        "- Web search: {}\n",
        if config.web_search.enabled {
            "enabled ✓"
        } else {
            "disabled"
        }
    ));
    report.push_str(&format!(
        "- HTTP requests: {}\n",
        if config.http_request.enabled {
            "enabled ✓"
        } else {
            "disabled"
        }
    ));

    // ── Memory ──────────────────────────────────────────────────────
    report.push_str("\n### Memory\n");
    report.push_str(&format!("- Backend: {}\n", config.memory.backend));
    report.push_str(&format!(
        "- Auto-save: {}\n",
        if config.memory.auto_save { "on" } else { "off" }
    ));

    // ── Local AI ────────────────────────────────────────────────────
    report.push_str("\n### Local AI\n");
    report.push_str(&format!(
        "- Local model: {}\n",
        if config.local_ai.enabled {
            "enabled ✓"
        } else {
            "not enabled"
        }
    ));

    // ── Delegate agents ─────────────────────────────────────────────
    if !config.agents.is_empty() {
        report.push_str("\n### Delegate Agents\n");
        for (name, agent_cfg) in &config.agents {
            report.push_str(&format!("- {name}: model={}\n", agent_cfg.model));
        }
    }

    tracing::debug!(
        "[complete_onboarding] status report generated, length={}",
        report.len()
    );

    Ok(ToolResult::success(report))
}

/// Marks the **chat-based welcome agent flow** as complete and seeds
/// proactive cron jobs.
///
/// After the #525 chat/UI onboarding split this tool flips
/// [`Config::chat_onboarding_completed`] — NOT the React UI's
/// [`Config::onboarding_completed`] flag. The welcome agent gates on
/// the chat flag, so flipping it here is what tells dispatch to route
/// the next chat turn to the orchestrator instead of welcome.
///
/// The React UI manages its own `onboarding_completed` flag via the
/// `config.set_onboarding_completed` JSON-RPC method (called by
/// `OnboardingOverlay.tsx::handleDone` and `Onboarding.tsx`). The two
/// flags are intentionally orthogonal so that:
///   * a Tauri user who completes the React wizard still sees the
///     welcome agent on their first chat turn (because the chat flag
///     is still `false` until the agent runs);
///   * a Telegram/Discord user (no React wizard) sees the welcome
///     agent on their first inbound message (same reason).
async fn complete() -> anyhow::Result<ToolResult> {
    let mut config = Config::load_or_init()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load config: {e}"))?;

    if config.chat_onboarding_completed {
        tracing::debug!("[complete_onboarding] chat welcome flow already completed — no-op");
        return Ok(ToolResult::success(
            "Chat welcome flow was already marked as complete.",
        ));
    }

    config.chat_onboarding_completed = true;
    config
        .save()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to save config: {e}"))?;

    // Seed proactive agents (morning briefing, etc.) on the false→true transition.
    let seed_config = config.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::openhuman::cron::seed::seed_proactive_agents(&seed_config) {
            tracing::warn!("[complete_onboarding] failed to seed proactive cron jobs: {e}");
        }
    });

    tracing::info!(
        "[complete_onboarding] chat welcome flow marked complete, proactive agents seeded"
    );

    Ok(ToolResult::success(
        "Chat welcome flow marked as complete. Morning briefing and proactive agent jobs have \
         been set up. The user is all set!",
    ))
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
}
