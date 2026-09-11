use super::*;

#[test]
fn defaults_to_cli_credentials_without_env() {
    let _env = super::super::ENV_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var("ANTHROPIC_API_KEY").ok();
    std::env::remove_var("ANTHROPIC_API_KEY");

    let (src, key) = resolve();
    assert_eq!(src, AuthSource::CliCredentials);
    assert!(key.is_none());

    if let Some(v) = prev {
        std::env::set_var("ANTHROPIC_API_KEY", v);
    }
}
