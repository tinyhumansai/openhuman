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
    /// The user's preferred *external* channel for proactive messages
    /// (morning briefings, welcome messages, cron output, etc.).
    ///
    /// Delivery is **web-first, then mirror**: the proactive message
    /// handler in [`crate::openhuman::channels::proactive`] always
    /// delivers to the in-app web channel first (via Socket.IO), then
    /// sends a copy to this external channel if it is set and
    /// connected. When `None` or `"web"`, only the web channel
    /// receives the message.
    ///
    /// Valid values: any channel name (`"telegram"`, `"discord"`,
    /// `"slack"`, etc.) or `None` for web-only delivery.
    #[serde(default)]
    pub active_channel: Option<String>,
}

fn default_channel_message_timeout_secs() -> u64 {
    300
}

impl ChannelsConfig {
    /// Whether [`crate::openhuman::channels::start_channels`] has any integrations to listen on.
    /// Used to avoid spawning the channel runtime when only RPC/outbound paths are needed.
    pub fn has_listening_integrations(&self) -> bool {
        self.telegram.is_some()
            || self.discord.is_some()
            || self.slack.is_some()
            || self.mattermost.is_some()
            || self.imessage.is_some()
            || self.signal.is_some()
            || self.linq.is_some()
            || self.email.is_some()
            || self.irc.is_some()
            || self.lark.is_some()
            || self.dingtalk.is_some()
            || self.qq.is_some()
            || self.matrix.is_some()
            || self.whatsapp.is_some()
    }
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
            active_channel: None,
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
    pub channel_id: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_config_deserializes_with_channel_id() {
        let toml = r#"
            bot_token = "test-token"
            guild_id = "123"
            channel_id = "456"
        "#;
        let config: DiscordConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.bot_token, "test-token");
        assert_eq!(config.guild_id.as_deref(), Some("123"));
        assert_eq!(config.channel_id.as_deref(), Some("456"));
    }

    #[test]
    fn discord_config_deserializes_without_channel_id() {
        let toml = r#"
            bot_token = "test-token"
        "#;
        let config: DiscordConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.bot_token, "test-token");
        assert!(config.guild_id.is_none());
        assert!(config.channel_id.is_none());
        assert!(config.allowed_users.is_empty());
        assert!(!config.listen_to_bots);
        assert!(!config.mention_only);
    }

    #[test]
    fn default_channels_config_has_no_integrations() {
        let cfg = ChannelsConfig::default();
        assert!(cfg.cli);
        assert!(!cfg.has_listening_integrations());
        assert_eq!(cfg.message_timeout_secs, 300);
        assert!(cfg.active_channel.is_none());
    }

    #[test]
    fn has_listening_integrations_detects_telegram() {
        let mut cfg = ChannelsConfig::default();
        cfg.telegram = Some(TelegramConfig {
            bot_token: "tok".into(),
            allowed_users: vec![],
            stream_mode: StreamMode::Off,
            draft_update_interval_ms: 1000,
            mention_only: false,
        });
        assert!(cfg.has_listening_integrations());
    }

    #[test]
    fn has_listening_integrations_detects_discord() {
        let mut cfg = ChannelsConfig::default();
        cfg.discord = Some(DiscordConfig {
            bot_token: "tok".into(),
            guild_id: None,
            channel_id: None,
            allowed_users: vec![],
            listen_to_bots: false,
            mention_only: false,
        });
        assert!(cfg.has_listening_integrations());
    }

    #[test]
    fn has_listening_integrations_detects_slack() {
        let mut cfg = ChannelsConfig::default();
        cfg.slack = Some(SlackConfig {
            bot_token: "tok".into(),
            app_token: None,
            channel_id: None,
            allowed_users: vec![],
        });
        assert!(cfg.has_listening_integrations());
    }

    #[test]
    fn stream_mode_default_is_off() {
        assert_eq!(StreamMode::default(), StreamMode::Off);
    }

    #[test]
    fn stream_mode_serde_roundtrip() {
        let json = serde_json::to_string(&StreamMode::Partial).unwrap();
        let back: StreamMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, StreamMode::Partial);
    }

    fn empty_whatsapp() -> WhatsAppConfig {
        WhatsAppConfig {
            access_token: None,
            phone_number_id: None,
            verify_token: None,
            app_secret: None,
            session_path: None,
            pair_phone: None,
            pair_code: None,
            allowed_numbers: vec![],
        }
    }

    #[test]
    fn whatsapp_backend_type_cloud_when_phone_number_id() {
        let mut cfg = empty_whatsapp();
        cfg.phone_number_id = Some("123".into());
        assert_eq!(cfg.backend_type(), "cloud");
    }

    #[test]
    fn whatsapp_backend_type_web_when_session_path() {
        let mut cfg = empty_whatsapp();
        cfg.session_path = Some("/tmp/session".into());
        assert_eq!(cfg.backend_type(), "web");
    }

    #[test]
    fn whatsapp_backend_type_defaults_to_cloud() {
        let cfg = empty_whatsapp();
        assert_eq!(cfg.backend_type(), "cloud");
    }

    #[test]
    fn whatsapp_is_cloud_config_requires_all_three() {
        let mut cfg = empty_whatsapp();
        cfg.phone_number_id = Some("123".into());
        cfg.access_token = Some("tok".into());
        cfg.verify_token = Some("vtok".into());
        assert!(cfg.is_cloud_config());

        let mut incomplete = empty_whatsapp();
        incomplete.phone_number_id = Some("123".into());
        assert!(!incomplete.is_cloud_config());
    }

    #[test]
    fn whatsapp_is_web_config() {
        let mut cfg = empty_whatsapp();
        cfg.session_path = Some("/path".into());
        assert!(cfg.is_web_config());
        assert!(!empty_whatsapp().is_web_config());
    }

    #[test]
    fn security_config_defaults() {
        let sec = SecurityConfig::default();
        assert!(sec.audit.enabled);
        assert_eq!(sec.audit.log_path, "audit.log");
        assert_eq!(sec.audit.max_size_mb, 100);
        assert!(!sec.audit.sign_events);
        assert_eq!(sec.resources.max_memory_mb, 512);
        assert_eq!(sec.resources.max_cpu_time_seconds, 60);
        assert_eq!(sec.resources.max_subprocesses, 10);
        assert!(sec.resources.memory_monitoring);
    }

    #[test]
    fn sandbox_config_default() {
        let sb = SandboxConfig::default();
        assert!(sb.enabled.is_none());
        assert!(matches!(sb.backend, SandboxBackend::Auto));
        assert!(sb.firejail_args.is_empty());
    }

    #[test]
    fn lark_receive_mode_default_is_websocket() {
        assert_eq!(LarkReceiveMode::default(), LarkReceiveMode::Websocket);
    }

    #[test]
    fn default_irc_port_is_6697() {
        let toml = r#"
            server = "irc.libera.chat"
            nickname = "bot"
        "#;
        let cfg: IrcConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.port, 6697);
    }

    #[test]
    fn default_draft_update_interval_ms_is_1000() {
        assert_eq!(default_draft_update_interval_ms(), 1000);
    }

    #[test]
    fn channels_config_serde_roundtrip() {
        let cfg = ChannelsConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ChannelsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.message_timeout_secs, 300);
        assert!(back.cli);
    }

    #[test]
    fn discord_config_roundtrip_json() {
        let config = DiscordConfig {
            bot_token: "tok".into(),
            guild_id: Some("g1".into()),
            channel_id: Some("c1".into()),
            allowed_users: vec!["user1".into()],
            listen_to_bots: true,
            mention_only: false,
        };
        let json = serde_json::to_string(&config).unwrap();
        let restored: DiscordConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.channel_id.as_deref(), Some("c1"));
        assert_eq!(restored.allowed_users, vec!["user1"]);
    }
}
