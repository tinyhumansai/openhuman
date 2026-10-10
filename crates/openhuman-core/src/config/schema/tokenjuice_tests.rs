use super::*;

#[test]
fn defaults_are_sane() {
    let c = TokenjuiceConfig::default();
    // The router and the handle preview are on (see
    // `ContextConfig::compaction_enabled`); the on-disk copy is opt-in.
    assert!(c.router_enabled);
    assert!(c.ccr_enabled);
    assert!(c.repl_handle_enabled);
    assert!(!c.repl_save_enabled);
    assert!(c.search_enabled);
}

#[test]
fn parses_from_toml() {
    let c: TokenjuiceConfig = toml::from_str(
        r#"
        router_enabled = false
        max_cache_entries = 12
        ccr_ttl_secs = 300
        "#,
    )
    .unwrap();
    assert!(!c.router_enabled);
    assert_eq!(c.max_cache_entries, 12);
    assert_eq!(c.ccr_ttl_secs, Some(300));
    // Untouched fields keep defaults.
    assert!(c.code_enabled);
    assert!(c.repl_handle_enabled);
}

#[test]
fn an_older_config_without_the_new_keys_gets_the_on_defaults() {
    let c: TokenjuiceConfig = toml::from_str("ccr_min_tokens = 900").unwrap();
    assert!(c.router_enabled);
    assert!(c.repl_handle_enabled);
    assert!(!c.repl_save_enabled);
}
