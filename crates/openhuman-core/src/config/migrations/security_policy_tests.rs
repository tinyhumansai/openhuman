use super::*;

fn doc(text: &str) -> Value {
    toml::from_str(text).unwrap()
}

#[test]
fn untouched_document_keeps_policy_off() {
    let mut d = doc("schema_version = 14\n");
    assert!(rewrite_document(&mut d).is_empty());
    let config = crate::config::schema::config_from_toml_str("schema_version = 14\n").unwrap();
    assert!(!config.autonomy.enabled);
}

#[test]
fn security_table_wins_over_legacy_key_by_key() {
    let text = r#"
[autonomy]
enabled = true
level = "readonly"
max_actions_per_hour = 7

[security.autonomy]
level = "full"

[sandbox]
backend = "landlock"

[security.privacy]
mode = "sensitive"
"#;
    let config = crate::config::schema::config_from_toml_str(text).unwrap();
    assert!(config.autonomy.enabled);
    assert_eq!(config.autonomy.level, crate::security::AutonomyLevel::Full);
    assert_eq!(config.autonomy.max_actions_per_hour, 7);
    assert!(matches!(
        config.sandbox.backend,
        crate::config::SandboxBackend::Landlock
    ));
    assert_eq!(config.privacy.mode, crate::config::PrivacyMode::Sensitive);
}

#[test]
fn security_only_spelling_decodes_without_legacy_tables() {
    let config = crate::config::schema::config_from_toml_str(
        "[security.autonomy]\nenabled = true\n[security.audit]\nenabled = false\n[security.approval]\nttl_secs = 30\n",
    )
    .unwrap();
    assert!(config.autonomy.enabled);
    assert!(!config.security.audit.enabled);
    assert_eq!(config.security.approval.ttl_secs, Some(30));
}

#[test]
fn max_cost_is_deleted_with_a_notice_everywhere_it_appears() {
    let mut d = doc("[autonomy]\nmax_cost_per_day_cents = 900\n[security.autonomy]\nmax_cost_per_day_cents = 1\n");
    let notices = rewrite_document(&mut d);
    assert!(notices.contains(&Notice::DroppedMaxCost { table: "autonomy" }));
    assert!(notices.contains(&Notice::DroppedMaxCost {
        table: "security.autonomy"
    }));
    assert!(d["autonomy"].get("max_cost_per_day_cents").is_none());
}

#[test]
fn rewritten_document_round_trips_through_save_shape() {
    let text = "[autonomy]\nenabled = true\nmax_cost_per_day_cents = 5\n";
    let config = crate::config::schema::config_from_toml_str(text).unwrap();
    let saved = toml::to_string(&config).unwrap();
    assert!(!saved.contains("max_cost_per_day_cents"));
    let again = crate::config::schema::config_from_toml_str(&saved).unwrap();
    assert!(again.autonomy.enabled);
}
