//! Shared response types for channel controller operations.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Result returned by `connect_channel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConnectionResult {
    /// `"connected"` for credential-based modes, `"pending_auth"` for OAuth/managed.
    pub status: String,
    /// Whether the service must be restarted for the channel to become active.
    pub restart_required: bool,
    /// For OAuth/managed modes: the action ID the frontend should handle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_action: Option<String>,
    /// Human-readable status message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Single entry returned by `channel_status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelStatusEntry {
    pub channel_id: String,
    pub auth_mode: super::super::definitions::ChannelAuthMode,
    pub connected: bool,
    pub has_credentials: bool,
    /// Live failure reason from the supervised listener when the channel is
    /// configured but its runtime listener is currently in an error state
    /// (issue #3712 — surface a real error instead of a false "Connected").
    /// `None` when healthy, still starting, or when the mode has no runtime
    /// listener (e.g. managed-DM, which routes through the backend bot).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result returned by `test_channel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelTestResult {
    pub success: bool,
    pub message: String,
}

/// Result from `telegram_login_start`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramLoginStartResult {
    /// The short-lived link token created by the backend.
    pub link_token: String,
    /// Full Telegram deep link URL the user should open.
    pub telegram_url: String,
    /// Bot username used.
    pub bot_username: String,
}

/// Result from `telegram_login_check`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramLoginCheckResult {
    /// Whether the Telegram user has been linked to the app user.
    pub linked: bool,
    /// Backend-provided status payload (may include telegramUserId, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Result from `discord_link_start`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscordLinkStartResult {
    /// The short-lived link token to paste into Discord.
    pub link_token: String,
    /// Human-readable instruction shown to the user.
    pub instructions: String,
}

/// Result from `discord_link_check`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscordLinkCheckResult {
    /// Whether the Discord account has been linked to the app user.
    pub linked: bool,
    /// Backend-provided status payload (may include discordId, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}
