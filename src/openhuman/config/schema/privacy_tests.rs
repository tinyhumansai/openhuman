use super::*;

#[test]
fn privacy_mode_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&PrivacyMode::LocalOnly).unwrap(),
        "\"local_only\""
    );
    assert_eq!(
        serde_json::to_string(&PrivacyMode::Standard).unwrap(),
        "\"standard\""
    );
    assert_eq!(
        serde_json::to_string(&PrivacyMode::Sensitive).unwrap(),
        "\"sensitive\""
    );
}

#[test]
fn privacy_mode_deserializes_snake_case_roundtrip() {
    for (json, expected) in [
        ("\"local_only\"", PrivacyMode::LocalOnly),
        ("\"standard\"", PrivacyMode::Standard),
        ("\"sensitive\"", PrivacyMode::Sensitive),
    ] {
        let got: PrivacyMode = serde_json::from_str(json).unwrap();
        assert_eq!(got, expected);
    }
}

#[test]
fn privacy_mode_default_is_standard() {
    assert_eq!(PrivacyMode::default(), PrivacyMode::Standard);
    assert_eq!(PrivacyConfig::default().mode, PrivacyMode::Standard);
}

#[test]
fn missing_privacy_block_defaults_to_standard() {
    // A config fragment with no `[privacy]` table at all.
    #[derive(serde::Deserialize)]
    struct Fragment {
        #[serde(default)]
        privacy: PrivacyConfig,
    }
    let parsed: Fragment = toml::from_str("").expect("empty toml deserializes");
    assert_eq!(parsed.privacy.mode, PrivacyMode::Standard);
}

#[test]
fn missing_mode_key_defaults_to_standard() {
    // `[privacy]` table present but no `mode` key.
    let parsed: PrivacyConfig =
        toml::from_str("").expect("empty privacy block deserializes via serde(default)");
    assert_eq!(parsed.mode, PrivacyMode::Standard);
}

#[test]
fn explicit_mode_key_parses() {
    let parsed: PrivacyConfig =
        toml::from_str("mode = \"local_only\"").expect("explicit mode parses");
    assert_eq!(parsed.mode, PrivacyMode::LocalOnly);
}
