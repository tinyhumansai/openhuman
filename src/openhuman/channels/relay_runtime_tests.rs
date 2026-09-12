use super::*;
use tinychannels::config::{RelayRuntimeConfig, RelayRuntimeIdentityConfig};

#[test]
fn relay_runtime_fronts_only_configured_identity_platforms() {
    let mut config = ChannelsConfig::default();
    assert!(!relay_runtime_fronts_channel(&config, "discord"));

    config.relay = Some(RelayRuntimeConfig {
        url: "wss://relay.example/relay".to_string(),
        identities: vec![RelayRuntimeIdentityConfig {
            platform: "discord".to_string(),
            bot_id: "app-1".to_string(),
        }],
        ..Default::default()
    });

    assert!(relay_runtime_fronts_channel(&config, "discord"));
    assert!(!relay_runtime_fronts_channel(&config, "telegram"));
}

#[test]
fn relay_result_maps_message_id_and_failures() {
    let result =
        relay_send_message_result(serde_json::json!({"success": true, "message_id": "m1"}))
            .expect("successful relay result");
    assert_eq!(result.message_id.as_deref(), Some("m1"));
    assert_eq!(
        result.raw,
        Some(serde_json::json!({"success": true, "message_id": "m1"}))
    );

    let err = relay_send_message_result(serde_json::json!({"success": false, "error": "denied"}))
        .unwrap_err();
    assert_eq!(err.to_string(), "relay outbound failed: denied");
}
