use super::*;

#[test]
fn test_name() {
    let ch = QQChannel::new("id".into(), "secret".into(), vec![]);
    assert_eq!(ch.name(), "qq");
}

#[test]
fn test_user_allowed_wildcard() {
    let ch = QQChannel::new("id".into(), "secret".into(), vec!["*".into()]);
    assert!(ch.is_user_allowed("anyone"));
}

#[test]
fn test_user_allowed_specific() {
    let ch = QQChannel::new("id".into(), "secret".into(), vec!["user123".into()]);
    assert!(ch.is_user_allowed("user123"));
    assert!(!ch.is_user_allowed("other"));
}

#[test]
fn test_user_denied_empty() {
    let ch = QQChannel::new("id".into(), "secret".into(), vec![]);
    assert!(!ch.is_user_allowed("anyone"));
}

#[tokio::test]
async fn test_dedup() {
    let ch = QQChannel::new("id".into(), "secret".into(), vec![]);
    assert!(!ch.is_duplicate("msg1").await);
    assert!(ch.is_duplicate("msg1").await);
    assert!(!ch.is_duplicate("msg2").await);
}

#[tokio::test]
async fn test_dedup_empty_id() {
    let ch = QQChannel::new("id".into(), "secret".into(), vec![]);
    // Empty IDs should never be considered duplicates
    assert!(!ch.is_duplicate("").await);
    assert!(!ch.is_duplicate("").await);
}

#[test]
fn test_config_serde() {
    let toml_str = r#"
app_id = "12345"
app_secret = "secret_abc"
allowed_users = ["user1"]
"#;
    let config: crate::openhuman::config::schema::QQConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.app_id, "12345");
    assert_eq!(config.app_secret, "secret_abc");
    assert_eq!(config.allowed_users, vec!["user1"]);
}

#[test]
fn ensure_https_accepts_https_urls() {
    assert!(ensure_https("https://api.example.com").is_ok());
    assert!(ensure_https("https://api.sgroup.qq.com/v1").is_ok());
}

#[test]
fn ensure_https_rejects_http_and_other_schemes() {
    assert!(ensure_https("http://example.com").is_err());
    assert!(ensure_https("ws://example.com").is_err());
    assert!(ensure_https("ftp://example.com").is_err());
    assert!(ensure_https("").is_err());
    assert!(ensure_https("example.com").is_err());
}

#[test]
fn api_base_and_auth_url_are_https_constants() {
    assert!(QQ_API_BASE.starts_with("https://"));
    assert!(QQ_AUTH_URL.starts_with("https://"));
}

#[test]
fn new_constructor_stores_fields() {
    let ch = QQChannel::new("a".into(), "b".into(), vec!["u1".into()]);
    assert_eq!(ch.app_id, "a");
    assert_eq!(ch.app_secret, "b");
    assert_eq!(ch.allowed_users, vec!["u1".to_string()]);
}
