use super::*;

#[test]
fn classify_health_result_maps_all_outcomes() {
    assert_eq!(
        classify_health_result(&Ok(true)),
        ChannelHealthState::Healthy
    );
    assert_eq!(
        classify_health_result(&Ok(false)),
        ChannelHealthState::Unhealthy
    );
}

#[tokio::test]
async fn classify_health_result_maps_timeout() {
    let elapsed = tokio::time::timeout(
        std::time::Duration::from_millis(1),
        std::future::pending::<()>(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        classify_health_result(&Err(elapsed)),
        ChannelHealthState::Timeout
    );
}

#[tokio::test]
async fn doctor_channels_returns_ok_when_no_channels_are_configured() {
    let mut config = Config::default();
    config.channels_config = crate::openhuman::config::ChannelsConfig::default();
    doctor_channels(config).await.unwrap();
}

#[tokio::test]
async fn doctor_channels_runs_with_telegram_config() {
    use crate::openhuman::config::{StreamMode, TelegramConfig};
    let mut config = Config::default();
    config.channels_config = crate::openhuman::config::ChannelsConfig::default();
    config.channels_config.telegram = Some(TelegramConfig {
        bot_token: "fake:token".into(),
        chat_id: None,
        allowed_users: vec!["user1".into()],
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 2000,
        silent_streaming: true,
        mention_only: false,
    });
    let _ = doctor_channels(config).await;
}

#[tokio::test]
async fn doctor_channels_runs_with_discord_config() {
    use crate::openhuman::config::DiscordConfig;
    let mut config = Config::default();
    config.channels_config = crate::openhuman::config::ChannelsConfig::default();
    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "fake".into(),
        guild_id: Some("123".into()),
        channel_id: Some("456".into()),
        allowed_users: vec![],
        listen_to_bots: false,
        mention_only: true,
    });
    let _ = doctor_channels(config).await;
}

#[tokio::test]
async fn doctor_channels_runs_with_slack_config() {
    use crate::openhuman::config::SlackConfig;
    let mut config = Config::default();
    config.channels_config = crate::openhuman::config::ChannelsConfig::default();
    config.channels_config.slack = Some(SlackConfig {
        bot_token: "fake".into(),
        app_token: None,
        channel_id: Some("C123".into()),
        allowed_users: vec![],
    });
    let _ = doctor_channels(config).await;
}

#[tokio::test]
async fn doctor_channels_runs_with_imessage_config() {
    use crate::openhuman::config::IMessageConfig;
    let mut config = Config::default();
    config.channels_config = crate::openhuman::config::ChannelsConfig::default();
    config.channels_config.imessage = Some(IMessageConfig {
        allowed_contacts: vec!["a@b.com".into()],
    });
    let _ = doctor_channels(config).await;
}

#[tokio::test]
async fn doctor_channels_runs_with_multiple_channels() {
    use crate::openhuman::config::{DiscordConfig, SlackConfig, StreamMode, TelegramConfig};
    let mut config = Config::default();
    config.channels_config = crate::openhuman::config::ChannelsConfig::default();
    config.channels_config.telegram = Some(TelegramConfig {
        bot_token: "fake".into(),
        chat_id: None,
        allowed_users: vec![],
        stream_mode: StreamMode::default(),
        draft_update_interval_ms: 2000,
        silent_streaming: true,
        mention_only: false,
    });
    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "fake".into(),
        guild_id: Some("123".into()),
        channel_id: Some("456".into()),
        allowed_users: vec![],
        listen_to_bots: false,
        mention_only: false,
    });
    config.channels_config.slack = Some(SlackConfig {
        bot_token: "fake".into(),
        app_token: None,
        channel_id: Some("C123".into()),
        allowed_users: vec![],
    });
    let _ = doctor_channels(config).await;
}
