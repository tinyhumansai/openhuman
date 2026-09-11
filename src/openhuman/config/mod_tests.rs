use super::*;

#[test]
fn reexported_config_default_is_constructible() {
    let config = Config::default();

    assert!(config.default_model.is_some());
    assert!(config.default_temperature > 0.0);
}

#[test]
fn reexported_channel_configs_are_constructible() {
    let telegram = TelegramConfig {
        bot_token: "token".into(),
        chat_id: None,
        allowed_users: vec!["alice".into()],
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    };

    let discord = DiscordConfig {
        bot_token: "token".into(),
        guild_id: Some("123".into()),
        channel_id: None,
        allowed_users: vec![],
        listen_to_bots: false,
        mention_only: false,
    };

    let lark = LarkConfig {
        app_id: "app-id".into(),
        app_secret: "app-secret".into(),
        encrypt_key: None,
        verification_token: None,
        allowed_users: vec![],
        use_feishu: false,
        receive_mode: crate::openhuman::config::schema::LarkReceiveMode::Websocket,
        port: None,
    };

    assert_eq!(telegram.allowed_users.len(), 1);
    assert_eq!(discord.guild_id.as_deref(), Some("123"));
    assert_eq!(lark.app_id, "app-id");
}
