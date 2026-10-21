use super::*;

#[test]
fn provider_selection_ignores_cli_and_relay_only_configuration() {
    let mut config = Config::default();
    config.channels_config.relay = Some(Default::default());
    assert!(configured_providers(&config).is_empty());
    config.channels_config.imessage = Some(tinychannels_bus::config::IMessageConfig {
        allowed_contacts: vec![],
    });
    assert_eq!(configured_providers(&config), vec!["imessage"]);
}

#[test]
fn listener_configuration_includes_channels_and_nonempty_relays_but_not_push_webhooks() {
    let mut config = Config::default();
    config.channels_config.webhook = Some(tinychannels_bus::config::WebhookConfig {
        port: 0,
        secret: None,
    });
    assert!(!has_listener_configuration(&config));

    config.channels_config.relay = Some(tinychannels_bus::config::RelayRuntimeConfig {
        url: "https://relay.example.invalid".into(),
        identities: vec![tinychannels_bus::config::RelayRuntimeIdentityConfig {
            platform: "discord".into(),
            bot_id: "doctor-test-bot".into(),
        }],
        ..Default::default()
    });
    assert!(has_listener_configuration(&config));
}
