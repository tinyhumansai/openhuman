use super::*;

#[test]
fn hosting_is_off_and_points_at_vercel_by_default() {
    let config = HostingConfig::default();

    assert!(!config.enabled);
    assert_eq!(config.provider, "vercel");
    assert!(!config.has_api_key());
    assert_eq!(config.team(), None);
}

#[test]
fn a_blank_key_or_team_is_the_same_as_none() {
    let config = HostingConfig {
        api_key: "   ".to_string(),
        team: "  ".to_string(),
        ..HostingConfig::default()
    };

    assert!(!config.has_api_key());
    assert_eq!(config.team(), None);
}

#[test]
fn a_configured_key_and_team_are_reported() {
    let config = HostingConfig {
        enabled: true,
        api_key: "token".to_string(),
        team: " team_abc ".to_string(),
        ..HostingConfig::default()
    };

    assert!(config.has_api_key());
    assert_eq!(config.team(), Some("team_abc"));
}

#[test]
fn the_section_round_trips_through_toml() {
    let config: HostingConfig =
        toml::from_str("enabled = true\napi_key = \"token\"\n").expect("parses");

    assert!(config.enabled);
    assert_eq!(config.provider, "vercel");
    assert_eq!(config.api_key, "token");
}

#[test]
fn the_team_scope_round_trips_through_toml() {
    let config: HostingConfig =
        toml::from_str("enabled = true\nteam = \"team_abc\"\n").expect("parses");

    assert_eq!(config.team(), Some("team_abc"));
}

#[test]
fn debug_formatting_never_renders_the_raw_key() {
    let config = HostingConfig {
        api_key: "super-secret-token".to_string(),
        ..HostingConfig::default()
    };

    let rendered = format!("{config:?}");

    assert!(!rendered.contains("super-secret-token"));
    assert!(rendered.contains("<redacted>"));
}
