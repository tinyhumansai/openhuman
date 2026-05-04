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
        silent_streaming: true,
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
