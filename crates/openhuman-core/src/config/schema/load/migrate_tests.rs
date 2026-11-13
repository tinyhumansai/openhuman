use super::*;

#[test]
fn migration_log_url_always_removes_url_secrets() {
    let raw = "https://user:password@example.test/v1/chat/completions?api_key=secret#fragment";

    let logged = migration_url_for_log(raw);

    assert!(!logged.contains("user"));
    assert!(!logged.contains("password"));
    assert!(!logged.contains("secret"));
    assert!(!logged.contains("fragment"));
    assert!(logged.contains("api_key"));
}
