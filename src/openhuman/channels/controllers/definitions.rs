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
    /// Default state for `field_type == "boolean"` fields. The UI seeds the
    /// checkbox from this so its visible state matches what persists when the
    /// user doesn't touch it (e.g. `smtp_tls` defaults on). `None` for
    /// non-boolean fields and booleans that default off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_bool: Option<bool>,
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
        lark_definition(),
        dingtalk_definition(),
        email_definition(),
        yuanbao_definition(),
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
                        default_bool: None,
                    },
                    FieldRequirement {
                        key: "chat_id",
                        label: "Chat ID",
                        field_type: "string",
                        required: false,
                        placeholder: "Optional: default chat for outbound messages",
                        default_bool: None,
                    },
                    FieldRequirement {
                        key: "allowed_users",
                        label: "Allowed Users",
                        field_type: "string",
                        required: false,
                        placeholder: "Comma-separated Telegram usernames",
                        default_bool: None,
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
                        default_bool: None,
                    },
                    FieldRequirement {
                        key: "guild_id",
                        label: "Server (Guild) ID",
                        field_type: "string",
                        required: false,
                        placeholder: "Optional: restrict to a specific server",
                        default_bool: None,
                    },
                    FieldRequirement {
                        key: "channel_id",
                        label: "Channel ID",
                        field_type: "string",
                        required: false,
                        placeholder: "Optional: default channel for outbound messages",
                        default_bool: None,
                    },
                    FieldRequirement {
                        key: "allowed_users",
                        label: "Allowed Users",
                        field_type: "string",
                        required: false,
                        placeholder:
                            "Comma-separated Discord user IDs, or * for everyone (blank = everyone)",
                        default_bool: None,
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
                default_bool: None,
            }],
            auth_action: None,
        }],
        capabilities: vec![ChannelCapability::SendText, ChannelCapability::ReceiveText],
    }
}

/// Lark (国际版) / Feishu (国内版) — Stream WebSocket channel. Wire-protocol
/// already implemented in `src/openhuman/channels/providers/lark.rs`; this
/// definition exposes the existing config surface to the Settings UI so
/// users no longer need to hand-edit `config.toml` to enable it.
///
/// Field names match `config::schema::channels::LarkConfig` exactly so the
/// frontend can persist credentials through the same RPC the other channels
/// use. See #2048.
fn lark_definition() -> ChannelDefinition {
    ChannelDefinition {
        id: "lark",
        display_name: "Lark / Feishu",
        description: "Send and receive via Lark (international) or Feishu (中国版).",
        icon: "lark",
        auth_modes: vec![AuthModeSpec {
            mode: ChannelAuthMode::ApiKey,
            description: "Provide your Lark/Feishu app credentials from the Open Platform.",
            fields: vec![
                FieldRequirement {
                    key: "app_id",
                    label: "App ID",
                    field_type: "string",
                    required: true,
                    placeholder: "cli_xxxxxxxxxxxx",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "app_secret",
                    label: "App Secret",
                    field_type: "secret",
                    required: true,
                    placeholder: "Your Lark app secret",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "encrypt_key",
                    label: "Encrypt Key",
                    field_type: "secret",
                    required: false,
                    placeholder: "Optional — required only if you enabled message encryption",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "verification_token",
                    label: "Verification Token",
                    field_type: "secret",
                    required: false,
                    placeholder: "Optional — used for HTTP webhook verification",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "use_feishu",
                    label: "Use Feishu (中国版)",
                    field_type: "boolean",
                    required: false,
                    placeholder: "On = open.feishu.cn (China); off = open.larksuite.com",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "receive_mode",
                    label: "Receive Mode",
                    field_type: "string",
                    required: false,
                    placeholder: "websocket (default) or webhook",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "port",
                    label: "Webhook Port",
                    // FieldRequirement.field_type currently accepts
                    // "string" / "secret" / "boolean" only (see the doc
                    // comment on FieldRequirement). Numeric port values
                    // are typed in as plain strings; the LarkConfig
                    // deserialiser parses them back to u16.
                    field_type: "string",
                    required: false,
                    placeholder:
                        "Optional — local HTTP port when receive_mode = webhook (e.g. 8080)",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "allowed_users",
                    label: "Allowed Users",
                    field_type: "string",
                    required: false,
                    placeholder: "Comma-separated open_id / union_id; leave empty to allow any",
                    default_bool: None,
                },
            ],
            auth_action: None,
        }],
        capabilities: vec![
            ChannelCapability::SendText,
            ChannelCapability::ReceiveText,
            ChannelCapability::ThreadedReplies,
        ],
    }
}

/// DingTalk (钉钉) Stream Mode WebSocket channel. Wire-protocol already
/// implemented in `src/openhuman/channels/providers/dingtalk.rs`. See #2048.
fn dingtalk_definition() -> ChannelDefinition {
    ChannelDefinition {
        id: "dingtalk",
        display_name: "DingTalk (钉钉)",
        description: "Send and receive via DingTalk Stream Mode (钉钉).",
        icon: "dingtalk",
        auth_modes: vec![AuthModeSpec {
            mode: ChannelAuthMode::ApiKey,
            description: "Provide your DingTalk app credentials from the developer console.",
            fields: vec![
                FieldRequirement {
                    key: "client_id",
                    label: "Client ID (AppKey)",
                    field_type: "string",
                    required: true,
                    placeholder: "ding_xxxxxxxxxxxx",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "client_secret",
                    label: "Client Secret (AppSecret)",
                    field_type: "secret",
                    required: true,
                    placeholder: "Your DingTalk app secret",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "allowed_users",
                    label: "Allowed Users",
                    field_type: "string",
                    required: false,
                    placeholder: "Comma-separated DingTalk userIds; leave empty to allow any",
                    default_bool: None,
                },
            ],
            auth_action: None,
        }],
        capabilities: vec![ChannelCapability::SendText, ChannelCapability::ReceiveText],
    }
}

/// Native IMAP/SMTP email channel for any standard mailbox (Fastmail, Proton
/// Bridge, iCloud, self-hosted, …) — the option non-Gmail/non-Outlook users
/// lacked (#4280). The IMAP IDLE + SMTP wire-protocol already lives in
/// `src/openhuman/channels/providers/email_channel.rs`; this definition exposes
/// its config surface to the Connections UI so users no longer need to
/// hand-edit `config.toml`.
///
/// Field keys map 1:1 to `config::schema::channels::EmailConfig` so the
/// frontend persists credentials through the same `channels_connect` RPC every
/// other channel uses. `imap_port` / `smtp_port` are typed as plain strings
/// (FieldRequirement only supports string/secret/boolean); `connect_channel`
/// parses them back to `u16`.
fn email_definition() -> ChannelDefinition {
    ChannelDefinition {
        id: "email",
        display_name: "Email (IMAP/SMTP)",
        description: "Send and receive email via any standard IMAP/SMTP mailbox.",
        icon: "email",
        auth_modes: vec![AuthModeSpec {
            mode: ChannelAuthMode::ApiKey,
            description: "Provide your mailbox's IMAP/SMTP server settings and an app password.",
            fields: vec![
                FieldRequirement {
                    key: "imap_host",
                    label: "IMAP Host",
                    field_type: "string",
                    required: true,
                    placeholder: "imap.fastmail.com",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "imap_port",
                    label: "IMAP Port",
                    field_type: "string",
                    required: false,
                    placeholder: "993 (TLS)",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "username",
                    label: "Email Address",
                    field_type: "string",
                    required: true,
                    placeholder: "you@example.com",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "password",
                    label: "Password / App Password",
                    field_type: "secret",
                    required: true,
                    placeholder: "App-specific password (recommended)",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "smtp_host",
                    label: "SMTP Host",
                    field_type: "string",
                    required: true,
                    placeholder: "smtp.fastmail.com",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "smtp_port",
                    label: "SMTP Port",
                    field_type: "string",
                    required: false,
                    placeholder: "465 (TLS)",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "smtp_tls",
                    label: "Use TLS for SMTP",
                    field_type: "boolean",
                    required: false,
                    placeholder: "On = TLS (recommended)",
                    default_bool: Some(true),
                },
                FieldRequirement {
                    key: "from_address",
                    label: "From Address",
                    field_type: "string",
                    required: false,
                    placeholder: "Optional — defaults to the email address above",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "imap_folder",
                    label: "IMAP Folder",
                    field_type: "string",
                    required: false,
                    placeholder: "Optional — defaults to INBOX",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "allowed_senders",
                    label: "Allowed Senders",
                    field_type: "string",
                    required: false,
                    placeholder: "Comma-separated addresses or @domain; * to allow any",
                    default_bool: None,
                },
            ],
            auth_action: None,
        }],
        capabilities: vec![
            ChannelCapability::SendText,
            ChannelCapability::ReceiveText,
            ChannelCapability::FileAttachments,
        ],
    }
}

fn yuanbao_definition() -> ChannelDefinition {
    // Endpoint URLs (api_domain / ws_domain) are not user-facing — the
    // channel derives them from the `env` field of `YuanbaoConfig`
    // (default: production). Advanced users can override via TOML.
    ChannelDefinition {
        id: "yuanbao",
        display_name: "元宝",
        description: "通过元宝（Yuanbao）机器人收发消息。",
        icon: "yuanbao",
        auth_modes: vec![AuthModeSpec {
            mode: ChannelAuthMode::ApiKey,
            description: "提供元宝开放平台的 AppID 和 AppSecret。",
            fields: vec![
                FieldRequirement {
                    key: "app_key",
                    label: "AppID",
                    field_type: "string",
                    required: true,
                    placeholder: "元宝开放平台 AppID",
                    default_bool: None,
                },
                FieldRequirement {
                    key: "app_secret",
                    label: "AppSecret",
                    field_type: "secret",
                    required: true,
                    placeholder: "元宝开放平台 AppSecret",
                    default_bool: None,
                },
            ],
            auth_action: None,
        }],
        capabilities: vec![
            ChannelCapability::SendText,
            ChannelCapability::ReceiveText,
            ChannelCapability::Typing,
        ],
    }
}

#[cfg(test)]
#[path = "definitions_tests.rs"]
mod tests;
