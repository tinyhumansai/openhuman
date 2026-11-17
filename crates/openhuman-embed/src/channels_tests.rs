use super::*;

fn base() -> Config {
    let mut config = Config::default();
    config.channels_config.discord = Some(
        serde_json::from_value(
            serde_json::json!({ "bot_token": "d", "guild_id": null, "channel_id": null }),
        )
        .unwrap(),
    );
    config
        .agent
        .channel_agents
        .insert("discord".into(), "someone-else".into());
    config
}

#[test]
fn the_config_serves_only_this_bot_and_binds_it_to_the_agent() {
    let spec = TelegramChannelSpec::new("123:abc", "teeny-chat")
        .allowed_users(["alice", "42"])
        .mention_only(true)
        .chat_id("-100");
    let config = telegram_config(&base(), &spec);

    let telegram = config
        .channels_config
        .telegram
        .expect("telegram configured");
    assert_eq!(telegram.bot_token, "123:abc");
    assert_eq!(telegram.allowed_users, vec!["alice", "42"]);
    assert!(telegram.mention_only);
    assert_eq!(telegram.chat_id.as_deref(), Some("-100"));
    assert!(
        config.channels_config.discord.is_none(),
        "only the bot asked for is started"
    );
    assert_eq!(
        config.agent.channel_agents,
        std::collections::HashMap::from([("telegram".to_string(), "teeny-chat".to_string())])
    );
}

#[test]
fn allow_everyone_is_the_wildcard() {
    let spec = TelegramChannelSpec::new("t", "a").allow_everyone();
    let config = telegram_config(&Config::default(), &spec);
    assert_eq!(
        config.channels_config.telegram.unwrap().allowed_users,
        vec!["*"]
    );
}

#[test]
fn a_spec_without_a_bot_or_an_agent_is_invalid() {
    assert!(matches!(
        validate(&TelegramChannelSpec::new(" ", "teeny-chat")),
        Err(ChannelError::Invalid(_))
    ));
    assert!(matches!(
        validate(&TelegramChannelSpec::new("123:abc", "")),
        Err(ChannelError::Invalid(_))
    ));
    assert!(validate(&TelegramChannelSpec::new("123:abc", "teeny-chat")).is_ok());
}

#[test]
fn debug_never_prints_the_token() {
    let rendered = format!("{:?}", TelegramChannelSpec::new("123:SECRET", "teeny-chat"));
    assert!(!rendered.contains("SECRET"), "{rendered}");
    assert!(rendered.contains("teeny-chat"), "{rendered}");
}
