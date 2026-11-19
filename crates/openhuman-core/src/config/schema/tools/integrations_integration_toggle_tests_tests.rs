use super::*;

#[test]
fn managed_mode_active_when_enabled_without_key() {
    let toggle = IntegrationToggle {
        enabled: true,
        mode: INTEGRATION_MODE_MANAGED.into(),
        api_key: None,
    };
    assert!(toggle.is_active());
}

#[test]
fn managed_mode_inactive_when_disabled() {
    let toggle = IntegrationToggle {
        enabled: false,
        mode: INTEGRATION_MODE_MANAGED.into(),
        api_key: Some("ignored".into()),
    };
    assert!(!toggle.is_active());
}

#[test]
fn byo_mode_requires_non_empty_key() {
    let mut toggle = IntegrationToggle {
        enabled: true,
        mode: INTEGRATION_MODE_BYO.into(),
        api_key: None,
    };
    assert!(!toggle.is_active(), "missing key");

    toggle.api_key = Some("   ".into());
    assert!(!toggle.is_active(), "whitespace key");

    toggle.api_key = Some("real-key".into());
    assert!(toggle.is_active());
}

#[test]
fn byo_mode_inactive_when_disabled_even_with_key() {
    let toggle = IntegrationToggle {
        enabled: false,
        mode: INTEGRATION_MODE_BYO.into(),
        api_key: Some("real-key".into()),
    };
    assert!(!toggle.is_active());
}

#[test]
fn default_is_managed_and_active() {
    let toggle = IntegrationToggle::default();
    assert_eq!(toggle.mode, INTEGRATION_MODE_MANAGED);
    assert!(toggle.api_key.is_none());
    assert!(toggle.is_active());
}

#[test]
fn integrations_config_ignores_retired_twilio_apify_and_parallel_keys() {
    let raw = r#"
        [twilio]
        enabled = true
        mode = "managed"

        [apify]
        enabled = false

        [parallel]
        enabled = false
        api_key = "enc2:00"

        [google_places]
        enabled = false
    "#;
    let cfg: IntegrationsConfig =
        toml::from_str(raw).expect("old config with retired keys must still load");
    assert!(!cfg.google_places.enabled);
    assert!(cfg.tinyfish.enabled);
}
