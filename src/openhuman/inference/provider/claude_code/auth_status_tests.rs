use super::*;

#[test]
fn parses_claude_ai_subscription() {
    let raw = r#"{
        "loggedIn": true,
        "authMethod": "claude.ai",
        "apiProvider": "firstParty",
        "email": "user@example.com",
        "subscriptionType": "max"
    }"#;
    match parse_auth_status_json(raw) {
        AuthSource::Subscription {
            account_email,
            subscription_type,
            expires_at,
        } => {
            assert_eq!(account_email.as_deref(), Some("user@example.com"));
            assert_eq!(subscription_type.as_deref(), Some("max"));
            assert!(expires_at.is_none());
        }
        other => panic!("expected Subscription, got {other:?}"),
    }
}

#[test]
fn subscription_tolerates_missing_email_and_type() {
    let raw = r#"{ "loggedIn": true, "authMethod": "claude.ai" }"#;
    match parse_auth_status_json(raw) {
        AuthSource::Subscription {
            account_email,
            subscription_type,
            ..
        } => {
            assert!(account_email.is_none());
            assert!(subscription_type.is_none());
        }
        other => panic!("expected Subscription, got {other:?}"),
    }
}

#[test]
fn logged_in_via_api_key_method_maps_to_api_key_env() {
    let raw = r#"{ "loggedIn": true, "authMethod": "console", "apiProvider": "console" }"#;
    assert_eq!(parse_auth_status_json(raw), AuthSource::ApiKeyEnv);
}

#[test]
fn logged_in_without_auth_method_is_unknown_not_api_key() {
    // Schema drift: `loggedIn: true` but no `authMethod`. Must NOT be
    // reported as the definite `api_key_env` signed-in state — fall to
    // `unknown` so the UI shows "couldn't determine" instead.
    let raw = r#"{ "loggedIn": true }"#;
    match parse_auth_status_json(raw) {
        AuthSource::Unknown { reason } => assert!(reason.is_some()),
        other => panic!("expected Unknown, got {other:?}"),
    }
}

#[test]
fn logged_out_maps_to_none() {
    let raw = r#"{ "loggedIn": false }"#;
    assert_eq!(parse_auth_status_json(raw), AuthSource::None);
}

#[test]
fn missing_logged_in_field_is_unknown() {
    let raw = r#"{ "authMethod": "claude.ai", "email": "user@example.com" }"#;
    match parse_auth_status_json(raw) {
        AuthSource::Unknown { reason } => assert!(reason.is_some()),
        other => panic!("expected Unknown, got {other:?}"),
    }
}

#[test]
fn malformed_json_is_unknown_not_signed_out() {
    match parse_auth_status_json("not json at all") {
        AuthSource::Unknown { reason } => assert!(reason.is_some()),
        other => panic!("expected Unknown, got {other:?}"),
    }
}

#[test]
fn api_key_env_short_circuits_probe() {
    let _env = super::super::ENV_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var("ANTHROPIC_API_KEY").ok();
    std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test");

    let s = probe();
    assert_eq!(s.source, AuthSource::ApiKeyEnv);

    match prev {
        Some(v) => std::env::set_var("ANTHROPIC_API_KEY", v),
        None => std::env::remove_var("ANTHROPIC_API_KEY"),
    }
}
