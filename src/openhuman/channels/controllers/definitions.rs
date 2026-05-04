//! Channel definitions: metadata the UI needs to render setup forms and manage connections.

use serde::{Deserialize, Serialize};

/// Which authentication mode a channel connection uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelAuthMode {
    /// User provides an API key or access token.
    #[serde(rename = "api_key")]
    ApiKey,
    /// User provides a bot token (e.g. Telegram BotFather token).
    #[serde(rename = "bot_token")]
    BotToken,
    /// User authenticates via OAuth (server-side flow).
    #[serde(rename = "oauth")]
    OAuth,
    /// User messages the platform's managed bot directly.
    #[serde(rename = "managed_dm")]
    ManagedDm,
}

impl std::fmt::Display for ChannelAuthMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiKey => write!(f, "api_key"),
            Self::BotToken => write!(f, "bot_token"),
            Self::OAuth => write!(f, "oauth"),
            Self::ManagedDm => write!(f, "managed_dm"),
        }
    }
}

impl std::str::FromStr for ChannelAuthMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "api_key" => Ok(Self::ApiKey),
            "bot_token" => Ok(Self::BotToken),
            "oauth" => Ok(Self::OAuth),
            "managed_dm" => Ok(Self::ManagedDm),
            other => Err(format!("unknown auth mode: {other}")),
        }
    }
}

/// A single field the UI must collect for a given auth mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldRequirement {
    /// Machine key, e.g. `"bot_token"`, `"api_key"`.
    pub key: &'static str,
    /// Human-readable label for the form field.
    pub label: &'static str,
    /// Field type hint: `"string"`, `"secret"`, `"boolean"`.
    pub field_type: &'static str,
    /// Whether the field must be provided.
    pub required: bool,
    /// Placeholder / help text.
    pub placeholder: &'static str,
}

/// Describes one auth mode a channel supports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthModeSpec {
    /// Which auth mode this spec describes.
    pub mode: ChannelAuthMode,
    /// Short UI description, e.g. "Provide your own Telegram bot token".
    pub description: &'static str,
    /// Fields the user must fill out for this mode.
    pub fields: Vec<FieldRequirement>,
    /// For OAuth/managed modes: an action descriptor the frontend uses to
    /// route to the correct login/auth/connect screen.
    /// Examples: `"telegram_managed_dm"`, `"discord_oauth"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_action: Option<&'static str>,
}

/// Runtime capabilities a channel may support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelCapability {
    SendText,
    SendRichText,
    ReceiveText,
    Typing,
    DraftUpdates,
    ThreadedReplies,
    FileAttachments,
    Reactions,
}

/// Complete definition of a supported channel, suitable for UI rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelDefinition {
    /// Machine identifier, e.g. `"telegram"`, `"discord"`.
    pub id: &'static str,
    /// Human-readable display name.
    pub display_name: &'static str,
    /// Short description.
    pub description: &'static str,
    /// Icon identifier (frontend maps to actual icon asset).
    pub icon: &'static str,
    /// Supported authentication modes with per-mode field requirements.
    pub auth_modes: Vec<AuthModeSpec>,
    /// Runtime capabilities this channel provides.
    pub capabilities: Vec<ChannelCapability>,
}

impl ChannelDefinition {
    /// Find the auth mode spec for a given mode, if supported.
    pub fn auth_mode_spec(&self, mode: ChannelAuthMode) -> Option<&AuthModeSpec> {
        self.auth_modes.iter().find(|s| s.mode == mode)
    }

    /// Validate that `credentials` contains all required fields for `mode`.
    /// Returns `Ok(())` or an error listing missing fields.
    pub fn validate_credentials(
        &self,
        mode: ChannelAuthMode,
        credentials: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        let spec = self.auth_mode_spec(mode).ok_or_else(|| {
            format!(
                "channel '{}' does not support auth mode '{}'",
                self.id, mode
            )
        })?;

        let missing: Vec<&str> = spec
            .fields
            .iter()
            .filter(|f| f.required)
            .filter(|f| {
                credentials
                    .get(f.key)
                    .is_none_or(|v| v.as_str().is_some_and(|s| s.is_empty()))
            })
            .map(|f| f.key)
            .collect();

        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "missing required fields for {}.{}: {}",
                self.id,
                mode,
                missing.join(", ")
            ))
        }
    }
}

/// Return the static registry of all supported channel definitions.
pub fn all_channel_definitions() -> Vec<ChannelDefinition> {
    vec![
        telegram_definition(),
        discord_definition(),
        web_definition(),
        imessage_definition(),
    ]
}

/// Look up a channel definition by id.
pub fn find_channel_definition(channel_id: &str) -> Option<ChannelDefinition> {
    all_channel_definitions()
        .into_iter()
        .find(|d| d.id == channel_id)
}

fn telegram_definition() -> ChannelDefinition {
    ChannelDefinition {
        id: "telegram",
        display_name: "Telegram",
        description: "Send and receive messages via Telegram.",
        icon: "telegram",
        auth_modes: vec![
            AuthModeSpec {
                mode: ChannelAuthMode::ManagedDm,
                description: "Message the OpenHuman Telegram bot directly.",
                fields: vec![],
                auth_action: Some("telegram_managed_dm"),
            },
            AuthModeSpec {
                mode: ChannelAuthMode::BotToken,
                description: "Provide your own Telegram Bot token from @BotFather.",
                fields: vec![
                    FieldRequirement {
                        key: "bot_token",
                        label: "Bot Token",
                        field_type: "secret",
                        required: true,
                        placeholder: "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11",
                    },
                    FieldRequirement {
                        key: "allowed_users",
                        label: "Allowed Users",
                        field_type: "string",
                        required: false,
                        placeholder: "Comma-separated Telegram usernames",
                    },
                ],
                auth_action: None,
            },
        ],
        capabilities: vec![
            ChannelCapability::SendText,
            ChannelCapability::ReceiveText,
            ChannelCapability::Typing,
            ChannelCapability::DraftUpdates,
        ],
    }
}

fn discord_definition() -> ChannelDefinition {
    ChannelDefinition {
        id: "discord",
        display_name: "Discord",
        description: "Send and receive messages via Discord.",
        icon: "discord",
        auth_modes: vec![
            AuthModeSpec {
                mode: ChannelAuthMode::BotToken,
                description: "Provide your own Discord bot token.",
                fields: vec![
                    FieldRequirement {
                        key: "bot_token",
                        label: "Bot Token",
                        field_type: "secret",
                        required: true,
                        placeholder: "Your Discord bot token",
                    },
                    FieldRequirement {
                        key: "guild_id",
                        label: "Server (Guild) ID",
                        field_type: "string",
                        required: false,
                        placeholder: "Optional: restrict to a specific server",
                    },
                    FieldRequirement {
                        key: "channel_id",
                        label: "Channel ID",
                        field_type: "string",
                        required: false,
                        placeholder: "Optional: default channel for outbound messages",
                    },
                ],
                auth_action: None,
            },
            AuthModeSpec {
                mode: ChannelAuthMode::OAuth,
                description: "Install the OpenHuman bot to your Discord server via OAuth.",
                fields: vec![],
                auth_action: Some("discord_oauth"),
            },
            AuthModeSpec {
                mode: ChannelAuthMode::ManagedDm,
                description: "Link your personal Discord account to the OpenHuman bot.",
                fields: vec![],
                auth_action: Some("discord_managed_link"),
            },
        ],
        capabilities: vec![
            ChannelCapability::SendText,
            ChannelCapability::ReceiveText,
            ChannelCapability::Typing,
            ChannelCapability::ThreadedReplies,
        ],
    }
}

fn web_definition() -> ChannelDefinition {
    ChannelDefinition {
        id: "web",
        display_name: "Web",
        description: "Chat via the built-in web UI.",
        icon: "web",
        auth_modes: vec![AuthModeSpec {
            mode: ChannelAuthMode::ManagedDm,
            description: "Use the embedded web chat — no setup required.",
            fields: vec![],
            auth_action: None,
        }],
        capabilities: vec![
            ChannelCapability::SendText,
            ChannelCapability::SendRichText,
            ChannelCapability::ReceiveText,
        ],
    }
}

fn imessage_definition() -> ChannelDefinition {
    ChannelDefinition {
        id: "imessage",
        display_name: "iMessage",
        description: "Send and receive via macOS Messages (local, AppleScript bridge).",
        icon: "imessage",
        auth_modes: vec![AuthModeSpec {
            mode: ChannelAuthMode::ManagedDm,
            description: "Local-only — no credentials. Grant Full Disk Access to OpenHuman.",
            fields: vec![FieldRequirement {
                key: "allowed_contacts",
                label: "Allowed Contacts",
                field_type: "string",
                required: false,
                placeholder: "Comma-separated phone numbers or emails; * to allow any",
            }],
            auth_action: None,
        }],
        capabilities: vec![ChannelCapability::SendText, ChannelCapability::ReceiveText],
    }
}

#[cfg(test)]
#[path = "definitions_tests.rs"]
mod tests;
