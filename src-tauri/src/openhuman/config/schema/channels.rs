//! Channels configuration (Telegram, Discord, Slack, Matrix, etc.) and security/sandbox.

use crate::openhuman::channels::email_channel::EmailConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChannelsConfig {
    pub cli: bool,
    pub telegram: Option<TelegramConfig>,
    pub discord: Option<DiscordConfig>,
    pub slack: Option<SlackConfig>,
    pub mattermost: Option<MattermostConfig>,
    pub webhook: Option<WebhookConfig>,
    pub imessage: Option<IMessageConfig>,
    pub matrix: Option<MatrixConfig>,
    pub signal: Option<SignalConfig>,
    pub whatsapp: Option<WhatsAppConfig>,
    pub linq: Option<LinqConfig>,
    pub email: Option<EmailConfig>,
    pub irc: Option<IrcConfig>,
    pub lark: Option<LarkConfig>,
    pub dingtalk: Option<DingTalkConfig>,
    pub qq: Option<QQConfig>,
    #[serde(default = "default_channel_message_timeout_secs")]
    pub message_timeout_secs: u64,
}

fn default_channel_message_timeout_secs() -> u64 {
    300
}

impl Default for ChannelsConfig {
    fn default() -> Self {
        Self {
            cli: true,
            telegram: None,
            discord: None,
            slack: None,
            mattermost: None,
            webhook: None,
            imessage: None,
            matrix: None,
            signal: None,
            whatsapp: None,
            linq: None,
            email: None,
            irc: None,
            lark: None,
            dingtalk: None,
            qq: None,
            message_timeout_secs: default_channel_message_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StreamMode {
    #[default]
    Off,
    Partial,
}

pub(crate) fn default_draft_update_interval_ms() -> u64 {
    1000
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub allowed_users: Vec<String>,
    #[serde(default)]
    pub stream_mode: StreamMode,
    #[serde(default = "default_draft_update_interval_ms")]
    pub draft_update_interval_ms: u64,
    #[serde(default)]
    pub mention_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscordConfig {
    pub bot_token: String,
    pub guild_id: Option<String>,
    #[serde(default)]
    pub allowed_users: Vec<String>,
    #[serde(default)]
    pub listen_to_bots: bool,
    #[serde(default)]
    pub mention_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SlackConfig {
    pub bot_token: String,
    pub app_token: Option<String>,
    pub channel_id: Option<String>,
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MattermostConfig {
    pub url: String,
    pub bot_token: String,
    pub channel_id: Option<String>,
    #[serde(default)]
    pub allowed_users: Vec<String>,
    #[serde(default)]
    pub thread_replies: Option<bool>,
    #[serde(default)]
    pub mention_only: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebhookConfig {
    pub port: u16,
    pub secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IMessageConfig {
    pub allowed_contacts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MatrixConfig {
    pub homeserver: String,
    pub access_token: String,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    pub room_id: String,
    pub allowed_users: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SignalConfig {
    pub http_url: String,
    pub account: String,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub allowed_from: Vec<String>,
    #[serde(default)]
    pub ignore_attachments: bool,
    #[serde(default)]
    pub ignore_stories: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WhatsAppConfig {
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub phone_number_id: Option<String>,
    #[serde(default)]
    pub verify_token: Option<String>,
    #[serde(default)]
    pub app_secret: Option<String>,
    #[serde(default)]
    pub session_path: Option<String>,
    #[serde(default)]
    pub pair_phone: Option<String>,
    #[serde(default)]
    pub pair_code: Option<String>,
    #[serde(default)]
    pub allowed_numbers: Vec<String>,
}

impl WhatsAppConfig {
    pub fn backend_type(&self) -> &'static str {
        if self.phone_number_id.is_some() {
            "cloud"
        } else if self.session_path.is_some() {
            "web"
        } else {
            "cloud"
        }
    }

    pub fn is_cloud_config(&self) -> bool {
        self.phone_number_id.is_some() && self.access_token.is_some() && self.verify_token.is_some()
    }

    pub fn is_web_config(&self) -> bool {
        self.session_path.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinqConfig {
    pub api_token: String,
    pub from_phone: String,
    #[serde(default)]
    pub signing_secret: Option<String>,
    #[serde(default)]
    pub allowed_senders: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IrcConfig {
    pub server: String,
    #[serde(default = "default_irc_port")]
    pub port: u16,
    pub nickname: String,
    pub username: Option<String>,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub allowed_users: Vec<String>,
    pub server_password: Option<String>,
    pub nickserv_password: Option<String>,
    pub sasl_password: Option<String>,
    pub verify_tls: Option<bool>,
}

fn default_irc_port() -> u16 {
    6697
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LarkReceiveMode {
    #[default]
    Websocket,
    Webhook,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LarkConfig {
    pub app_id: String,
    pub app_secret: String,
    #[serde(default)]
    pub encrypt_key: Option<String>,
    #[serde(default)]
    pub verification_token: Option<String>,
    #[serde(default)]
    pub allowed_users: Vec<String>,
    #[serde(default)]
    pub use_feishu: bool,
    #[serde(default)]
    pub receive_mode: LarkReceiveMode,
    #[serde(default)]
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct SecurityConfig {
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub resources: ResourceLimitsConfig,
    #[serde(default)]
    pub audit: AuditConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SandboxConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub backend: SandboxBackend,
    #[serde(default)]
    pub firejail_args: Vec<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: None,
            backend: SandboxBackend::Auto,
            firejail_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SandboxBackend {
    #[default]
    Auto,
    Landlock,
    Firejail,
    Bubblewrap,
    Docker,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResourceLimitsConfig {
    #[serde(default = "default_max_memory_mb")]
    pub max_memory_mb: u32,
    #[serde(default = "default_max_cpu_time_seconds")]
    pub max_cpu_time_seconds: u64,
    #[serde(default = "default_max_subprocesses")]
    pub max_subprocesses: u32,
    #[serde(default = "default_memory_monitoring_enabled")]
    pub memory_monitoring: bool,
}

fn default_max_memory_mb() -> u32 {
    512
}
fn default_max_cpu_time_seconds() -> u64 {
    60
}
fn default_max_subprocesses() -> u32 {
    10
}
fn default_memory_monitoring_enabled() -> bool {
    true
}

impl Default for ResourceLimitsConfig {
    fn default() -> Self {
        Self {
            max_memory_mb: default_max_memory_mb(),
            max_cpu_time_seconds: default_max_cpu_time_seconds(),
            max_subprocesses: default_max_subprocesses(),
            memory_monitoring: default_memory_monitoring_enabled(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuditConfig {
    #[serde(default = "default_audit_enabled")]
    pub enabled: bool,
    #[serde(default = "default_audit_log_path")]
    pub log_path: String,
    #[serde(default = "default_audit_max_size_mb")]
    pub max_size_mb: u32,
    #[serde(default)]
    pub sign_events: bool,
}

fn default_audit_enabled() -> bool {
    true
}
fn default_audit_log_path() -> String {
    "audit.log".to_string()
}
fn default_audit_max_size_mb() -> u32 {
    100
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: default_audit_enabled(),
            log_path: default_audit_log_path(),
            max_size_mb: default_audit_max_size_mb(),
            sign_events: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DingTalkConfig {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QQConfig {
    pub app_id: String,
    pub app_secret: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
}
