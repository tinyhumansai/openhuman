use super::*;

#[test]
fn default_engine_is_local() {
    assert_eq!(
        SubconsciousConfig::default().engine,
        SubconsciousEngine::Local
    );
    assert!(!SubconsciousConfig::default().engine.is_medulla());
}

#[test]
fn missing_block_deserializes_to_local() {
    let config: SubconsciousConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(config.engine, SubconsciousEngine::Local);
}

#[test]
fn engine_serde_round_trip() {
    assert_eq!(
        serde_json::to_string(&SubconsciousEngine::Medulla).unwrap(),
        r#""medulla""#
    );
    assert_eq!(
        serde_json::from_str::<SubconsciousEngine>(r#""local""#).unwrap(),
        SubconsciousEngine::Local
    );
}

#[test]
fn explicit_serve_entry_wins_over_env() {
    // An explicit config value is used verbatim, ignoring any env override.
    assert_eq!(
        MedullaLocalConfig::resolve_entry("/tmp/serve.js", Some("/env/serve.js".to_string())),
        Some(std::path::PathBuf::from("/tmp/serve.js"))
    );
    // Surrounding whitespace is trimmed.
    assert_eq!(
        MedullaLocalConfig::resolve_entry("  /tmp/serve.js  ", None),
        Some(std::path::PathBuf::from("/tmp/serve.js"))
    );
}

#[test]
fn serve_entry_falls_back_to_env_override() {
    assert_eq!(
        MedullaLocalConfig::resolve_entry("", Some("/env/serve.js".to_string())),
        Some(std::path::PathBuf::from("/env/serve.js"))
    );
    assert_eq!(
        MedullaLocalConfig::resolve_entry("   ", Some("  /env/serve.js  ".to_string())),
        Some(std::path::PathBuf::from("/env/serve.js"))
    );
}

#[test]
fn serve_entry_unconfigured_resolves_to_none() {
    // No machine-local default is baked in: unset config + unset (or blank)
    // env resolves to None so the engine reports it unconfigured.
    assert_eq!(MedullaLocalConfig::resolve_entry("", None), None);
    assert_eq!(
        MedullaLocalConfig::resolve_entry("", Some("   ".to_string())),
        None
    );
}

#[test]
fn request_deadline_defaults_and_zero_falls_back() {
    // Both construction paths — `Default` and a config that omits the
    // field — land on the same documented default.
    assert_eq!(
        MedullaLocalConfig::default().request_deadline_secs,
        DEFAULT_REQUEST_DEADLINE_SECS
    );
    let omitted: MedullaLocalConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(omitted.request_deadline_secs, DEFAULT_REQUEST_DEADLINE_SECS);

    // A configured value is honoured…
    let configured: MedullaLocalConfig =
        serde_json::from_str(r#"{ "request_deadline_secs": 120 }"#).unwrap();
    assert_eq!(
        configured.request_deadline(),
        std::time::Duration::from_secs(120)
    );

    // …and an explicit zero (which would disable the ceiling) falls back
    // to the default instead.
    let zeroed: MedullaLocalConfig =
        serde_json::from_str(r#"{ "request_deadline_secs": 0 }"#).unwrap();
    assert_eq!(
        zeroed.request_deadline(),
        std::time::Duration::from_secs(DEFAULT_REQUEST_DEADLINE_SECS)
    );
}

#[test]
fn request_deadline_clamps_oversized_values_to_the_ceiling() {
    // A near-u64::MAX duration would panic in `Instant + Duration`
    // arithmetic on the request path; the accessor clamps instead.
    let oversized: MedullaLocalConfig = serde_json::from_str(&format!(
        r#"{{ "request_deadline_secs": {} }}"#,
        u64::MAX - 1
    ))
    .unwrap();
    assert_eq!(
        oversized.request_deadline(),
        std::time::Duration::from_secs(MAX_REQUEST_DEADLINE_SECS)
    );
    // The boundary value itself is accepted un-clamped.
    let at_max: MedullaLocalConfig = serde_json::from_str(&format!(
        r#"{{ "request_deadline_secs": {} }}"#,
        MAX_REQUEST_DEADLINE_SECS
    ))
    .unwrap();
    assert_eq!(
        at_max.request_deadline(),
        std::time::Duration::from_secs(MAX_REQUEST_DEADLINE_SECS)
    );
}
