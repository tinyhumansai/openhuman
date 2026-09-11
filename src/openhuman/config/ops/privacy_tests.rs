use super::*;

#[test]
fn parse_privacy_mode_accepts_canonical_and_variants() {
    assert_eq!(
        parse_privacy_mode("local_only").unwrap(),
        PrivacyMode::LocalOnly
    );
    assert_eq!(
        parse_privacy_mode("Local-Only").unwrap(),
        PrivacyMode::LocalOnly
    );
    assert_eq!(
        parse_privacy_mode(" STANDARD ").unwrap(),
        PrivacyMode::Standard
    );
    assert_eq!(
        parse_privacy_mode("sensitive").unwrap(),
        PrivacyMode::Sensitive
    );
    assert!(parse_privacy_mode("bogus").is_err());
}

#[test]
fn privacy_mode_value_shape() {
    assert_eq!(
        privacy_mode_value(PrivacyMode::LocalOnly),
        serde_json::json!({ "mode": "local_only" })
    );
}
