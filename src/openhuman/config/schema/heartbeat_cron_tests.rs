use super::*;

#[test]
fn heartbeat_defaults_are_opt_in() {
    let config = HeartbeatConfig::default();
    assert!(!config.enabled);
    assert!(!config.inference_enabled);
    assert!(!config.notify_meetings);
    assert!(!config.notify_reminders);
    assert!(!config.notify_relevant_events);
    assert!(!config.external_delivery_enabled);
    assert_eq!(config.interval_minutes, 5);
    assert_eq!(config.max_calendar_connections_per_tick, 2);
    assert_eq!(config.subconscious_mode, SubconsciousMode::Off);
    // Event-driven trigger pipeline is opt-in (back-compat: old configs
    // without these keys deserialize to the legacy interval-only path).
    assert!(!config.triggers_enabled);
    assert_eq!(config.max_promotions_per_hour, 30);
}

#[test]
fn legacy_config_without_trigger_keys_deserializes() {
    // A config file predating the trigger pipeline must still parse and
    // default to the disabled (legacy) path.
    let legacy = r#"{ "enabled": true, "inference_enabled": true }"#;
    let config: HeartbeatConfig = serde_json::from_str(legacy).unwrap();
    assert!(!config.triggers_enabled);
    assert_eq!(config.max_promotions_per_hour, 30);
    assert_eq!(
        config.effective_subconscious_mode(),
        SubconsciousMode::Simple
    );
}

#[test]
fn event_driven_mode_serde_and_helpers() {
    assert_eq!(
        serde_json::to_string(&SubconsciousMode::EventDriven).unwrap(),
        r#""event_driven""#
    );
    assert_eq!(
        serde_json::from_str::<SubconsciousMode>(r#""event_driven""#).unwrap(),
        SubconsciousMode::EventDriven
    );
    assert!(SubconsciousMode::EventDriven.is_enabled());
    assert!(SubconsciousMode::EventDriven.is_event_driven());
    assert!(!SubconsciousMode::Aggressive.is_event_driven());
    assert!(!SubconsciousMode::EventDriven.is_read_only());
    assert_eq!(
        SubconsciousMode::from_str_lossy("event_driven"),
        SubconsciousMode::EventDriven
    );
}

#[test]
fn subconscious_mode_serde_round_trip() {
    assert_eq!(
        serde_json::to_string(&SubconsciousMode::Simple).unwrap(),
        r#""simple""#
    );
    assert_eq!(
        serde_json::from_str::<SubconsciousMode>(r#""aggressive""#).unwrap(),
        SubconsciousMode::Aggressive
    );
    assert_eq!(SubconsciousMode::default(), SubconsciousMode::Off);
}

#[test]
fn subconscious_mode_helpers() {
    assert!(!SubconsciousMode::Off.is_enabled());
    assert!(SubconsciousMode::Simple.is_enabled());
    assert!(SubconsciousMode::Aggressive.is_enabled());
    assert!(SubconsciousMode::Simple.is_read_only());
    assert!(!SubconsciousMode::Aggressive.is_read_only());
    assert_eq!(SubconsciousMode::Simple.default_interval_minutes(), 30);
    assert_eq!(SubconsciousMode::Aggressive.default_interval_minutes(), 5);
}

#[test]
fn effective_mode_backward_compat() {
    let mut config = HeartbeatConfig::default();
    assert_eq!(config.effective_subconscious_mode(), SubconsciousMode::Off);

    config.enabled = true;
    config.inference_enabled = true;
    assert_eq!(
        config.effective_subconscious_mode(),
        SubconsciousMode::Simple
    );

    config.subconscious_mode = SubconsciousMode::Aggressive;
    assert_eq!(
        config.effective_subconscious_mode(),
        SubconsciousMode::Aggressive
    );
}

#[test]
fn heartbeat_deserialization_fills_opt_in_defaults() {
    let config: HeartbeatConfig = serde_json::from_str("{}").unwrap();
    assert!(!config.enabled);
    assert!(!config.inference_enabled);
    assert!(!config.notify_meetings);
    assert!(!config.notify_reminders);
    assert!(!config.notify_relevant_events);
    assert!(!config.external_delivery_enabled);
    assert_eq!(config.interval_minutes, 5);
    assert_eq!(config.max_calendar_connections_per_tick, 2);
    assert_eq!(config.meeting_lookahead_minutes, 120);
    assert_eq!(config.reminder_lookahead_minutes, 30);

    let partial: HeartbeatConfig =
        serde_json::from_str(r#"{"enabled":true,"interval_minutes":15}"#).unwrap();
    assert!(partial.enabled);
    assert_eq!(partial.interval_minutes, 15);
    assert!(!partial.inference_enabled);
    assert!(!partial.notify_meetings);
    assert_eq!(partial.max_calendar_connections_per_tick, 2);

    let zero_cap: HeartbeatConfig =
        serde_json::from_str(r#"{"max_calendar_connections_per_tick":0}"#).unwrap();
    assert_eq!(zero_cap.max_calendar_connections_per_tick, 1);

    let null_cap: HeartbeatConfig =
        serde_json::from_str(r#"{"max_calendar_connections_per_tick":null}"#).unwrap();
    assert_eq!(null_cap.max_calendar_connections_per_tick, 2);

    let explicit_cap: HeartbeatConfig =
        serde_json::from_str(r#"{"max_calendar_connections_per_tick":4}"#).unwrap();
    assert_eq!(explicit_cap.max_calendar_connections_per_tick, 4);
}
