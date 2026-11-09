use super::*;

#[test]
fn defaults_match_the_documented_values() {
    let c = InboundDebounceConfig::default();
    assert!(c.enabled);
    assert_eq!(c.debounce_ms, 1_500);
    assert_eq!(c.media_debounce_ms, 4_000);
    assert_eq!(c.max_wait_ms, 10_000);
    assert_eq!(c.max_batch_messages, 12);
    assert_eq!(c.max_batch_chars, 50_000);
    assert!(c.by_channel.is_empty());

    let s = c.settings_for("telegram");
    assert_eq!(s.debounce, Duration::from_millis(1_500));
    assert_eq!(s.media_debounce, Duration::from_millis(4_000));
    assert_eq!(s.max_wait, Duration::from_millis(10_000));
    assert_eq!(s.max_batch_messages, 12);
    assert_eq!(s.max_batch_chars, 50_000);
}

#[test]
fn channel_override_wins_over_global_and_ignores_account_suffix() {
    let c: InboundDebounceConfig = toml::from_str(
        r#"
        debounce_ms = 1000
        [by_channel.discord]
        debounce_ms = 300
        "#,
    )
    .unwrap();
    assert_eq!(c.debounce_ms, 1000);
    assert_eq!(c.media_debounce_ms, DEFAULT_MEDIA_DEBOUNCE_MS);
    let discord = c.settings_for("discord:guild-1");
    assert_eq!(discord.debounce, Duration::from_millis(300));
    assert_eq!(discord.media_debounce, Duration::from_millis(4_000));
    assert_eq!(
        c.settings_for("slack").debounce,
        Duration::from_millis(1000)
    );
}

#[test]
fn disabled_globally_or_per_channel_gives_immediate_settings() {
    assert_eq!(
        InboundDebounceConfig::disabled().settings_for("telegram"),
        DebounceSettings::immediate()
    );
    let mut c = InboundDebounceConfig::default();
    c.by_channel.insert(
        "telegram".into(),
        InboundDebounceOverride {
            enabled: Some(false),
            ..Default::default()
        },
    );
    assert_eq!(c.settings_for("telegram"), DebounceSettings::immediate());
    assert_ne!(c.settings_for("discord"), DebounceSettings::immediate());

    let mut off = InboundDebounceConfig::disabled();
    off.by_channel.insert(
        "telegram".into(),
        InboundDebounceOverride {
            enabled: Some(true),
            ..Default::default()
        },
    );
    assert_eq!(
        off.settings_for("telegram").debounce,
        Duration::from_millis(DEFAULT_DEBOUNCE_MS)
    );
}

#[test]
fn max_wait_and_caps_are_clamped() {
    let c = InboundDebounceConfig {
        max_wait_ms: 10,
        max_batch_messages: 0,
        max_batch_chars: 0,
        ..Default::default()
    };
    let s = c.settings_for("telegram");
    assert_eq!(s.max_wait, Duration::from_millis(4_000));
    assert_eq!(s.max_batch_messages, 1);
    assert_eq!(s.max_batch_chars, 1);
}

#[test]
fn toml_round_trip_keeps_every_field() {
    let mut c = InboundDebounceConfig {
        debounce_ms: 700,
        ..Default::default()
    };
    c.by_channel.insert(
        "whatsapp".into(),
        InboundDebounceOverride {
            enabled: Some(true),
            debounce_ms: Some(2000),
            media_debounce_ms: Some(5000),
            max_wait_ms: Some(12_000),
        },
    );
    let text = toml::to_string(&c).unwrap();
    let back: InboundDebounceConfig = toml::from_str(&text).unwrap();
    assert_eq!(back, c);
}
