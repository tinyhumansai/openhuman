use super::*;

#[test]
fn defaults_to_the_classic_layout() {
    assert_eq!(StorageConfig::default().url, None);
    let parsed: StorageConfig = toml::from_str("").unwrap();
    assert_eq!(parsed, StorageConfig::default());
}

#[test]
fn reads_a_url_from_toml() {
    let parsed: StorageConfig = toml::from_str("url = \"memory\"").unwrap();
    assert_eq!(parsed.url.as_deref(), Some("memory"));
}

#[test]
fn debug_never_prints_credentials() {
    let config = StorageConfig {
        url: Some("mongodb://app:hunter2@db.internal/openhuman".into()),
        ..StorageConfig::default()
    };
    let shown = format!("{config:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(
        shown.contains("mongodb://***@db.internal/openhuman"),
        "{shown}"
    );
    assert_eq!(redact_url("sqlite:/tmp/x"), "sqlite:/tmp/x");
    assert_eq!(redact_url("mongodb://h/db"), "mongodb://h/db");
}

#[test]
fn redact_drops_query_and_fragment_tokens() {
    assert_eq!(
        redact_url(
            "mongodb://db.internal/openhuman?authMechanismProperties=AWS_SESSION_TOKEN%3Asecret"
        ),
        "mongodb://db.internal/openhuman?***"
    );
    assert_eq!(
        redact_url("mongodb://app:pw@db/x?token=secret#frag"),
        "mongodb://***@db/x?***"
    );
}

#[test]
fn journal_retention_defaults_to_thirty_days_and_reads_from_toml() {
    assert_eq!(StorageConfig::default().journal_retention_days, 30);
    let unset: StorageConfig = toml::from_str("url = \"memory\"").unwrap();
    assert_eq!(unset.journal_retention_days, 30);
    let set: StorageConfig = toml::from_str("journal_retention_days = 7").unwrap();
    assert_eq!(set.journal_retention_days, 7);
}
