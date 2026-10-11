use super::*;

#[test]
fn report_lines_cover_every_state() {
    assert_eq!(
        report_line("Telegram", ChannelHealthState::Healthy),
        "  ✅ Telegram  healthy"
    );
    assert!(report_line("Slack", ChannelHealthState::Unhealthy).contains("unhealthy"));
    assert!(report_line("Slack", ChannelHealthState::Timeout).contains("timed out"));
}

#[test]
fn channel_labels_use_definition_display_names() {
    assert_eq!(channel_label("telegram"), "Telegram");
    assert_eq!(channel_label("not-a-channel"), "not-a-channel");
}

#[tokio::test]
async fn doctor_with_no_channels_is_ok() {
    doctor_channels(Config::default()).await.unwrap();
}

#[tokio::test]
async fn doctor_with_configured_provider_requires_the_module() {
    let mut config = Config::default();
    config.modules.enabled = false;
    config.channels_config.imessage = Some(tinychannels_bus::config::IMessageConfig {
        allowed_contacts: vec![],
    });
    let error = doctor_channels(config).await.unwrap_err();
    assert!(error.to_string().contains("module unavailable"), "{error}");
}

#[test]
fn doctor_provider_selection_ignores_cli_and_relay_only_configuration() {
    let mut config = Config::default();
    config.channels_config.relay = Some(Default::default());
    assert!(configured_providers(&config).is_empty());
    config.channels_config.imessage = Some(tinychannels_bus::config::IMessageConfig {
        allowed_contacts: vec![],
    });
    assert_eq!(configured_providers(&config), vec!["imessage"]);
}
