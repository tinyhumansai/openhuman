use super::*;

#[test]
fn defaults_are_sane() {
    let c = TokenjuiceConfig::default();
    assert!(c.router_enabled);
    assert!(c.ccr_enabled);
    assert!(c.search_enabled);
    assert!(!c.ml_compression_enabled);
    assert_eq!(c.ml_device, "cpu");
}

#[test]
fn parses_from_toml() {
    let c: TokenjuiceConfig = toml::from_str(
        r#"
        router_enabled = false
        ml_compression_enabled = true
        max_cache_entries = 12
        ccr_ttl_secs = 300
        "#,
    )
    .unwrap();
    assert!(!c.router_enabled);
    assert!(c.ml_compression_enabled);
    assert_eq!(c.max_cache_entries, 12);
    assert_eq!(c.ccr_ttl_secs, Some(300));
    // Untouched fields keep defaults.
    assert!(c.code_enabled);
}
