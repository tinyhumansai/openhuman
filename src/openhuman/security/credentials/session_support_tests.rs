use super::*;
use crate::openhuman::security::credentials::profiles::{AuthProfile, AuthProfileKind, TokenSet};
use chrono::Utc;
use serde_json::json;
use std::collections::BTreeMap;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

// ── profile_name_or_default ────────────────────────────────────

#[test]
fn profile_name_or_default_returns_default_for_none_and_empty() {
    assert_eq!(profile_name_or_default(None), DEFAULT_AUTH_PROFILE_NAME);
    assert_eq!(profile_name_or_default(Some("")), DEFAULT_AUTH_PROFILE_NAME);
    assert_eq!(
        profile_name_or_default(Some("   ")),
        DEFAULT_AUTH_PROFILE_NAME
    );
}

#[test]
fn profile_name_or_default_returns_value_when_present() {
    assert_eq!(profile_name_or_default(Some("work")), "work");
    assert_eq!(profile_name_or_default(Some("  work  ")), "work");
}

#[test]
fn is_local_session_token_requires_local_signature_marker() {
    assert!(is_local_session_token("header.payload.local"));
    assert!(is_local_session_token("  header.payload.local  "));
    assert!(!is_local_session_token("header.payload.remote"));
    assert!(!is_local_session_token("header.payload.local.extra"));
    assert!(!is_local_session_token("not-a-jwt"));
}

#[test]
fn slugify_local_session_host_normalizes_machine_names() {
    assert_eq!(
        slugify_local_session_host("My MacBook Pro"),
        "my-macbook-pro"
    );
    assert_eq!(slugify_local_session_host("DESKTOP_123"), "desktop-123");
    assert_eq!(slugify_local_session_host("   "), "device");
    assert_eq!(slugify_local_session_host("---"), "device");
}

// ── parse_fields_value ─────────────────────────────────────────

#[test]
fn parse_fields_value_returns_empty_for_none() {
    let map = parse_fields_value(None).unwrap();
    assert!(map.is_empty());
}

#[test]
fn parse_fields_value_rejects_non_object() {
    let err = parse_fields_value(Some(json!("not an object"))).unwrap_err();
    assert!(err.contains("fields must be a JSON object"));
    assert!(parse_fields_value(Some(json!([1, 2]))).is_err());
    assert!(parse_fields_value(Some(json!(5))).is_err());
}

#[test]
fn parse_fields_value_rejects_empty_keys() {
    let err = parse_fields_value(Some(json!({"": "v"}))).unwrap_err();
    assert!(err.contains("empty keys"));
    let err = parse_fields_value(Some(json!({"   ": "v"}))).unwrap_err();
    assert!(err.contains("empty keys"));
}

#[test]
fn parse_fields_value_renders_scalar_values_as_strings() {
    let out = parse_fields_value(Some(json!({
        "s": "hello",
        "n": 42,
        "b": true,
        "nil": null,
        "obj": { "nested": 1 }
    })))
    .unwrap();
    assert_eq!(out.get("s"), Some(&"hello".to_string()));
    assert_eq!(out.get("n"), Some(&"42".to_string()));
    assert_eq!(out.get("b"), Some(&"true".to_string()));
    assert_eq!(out.get("nil"), Some(&String::new()));
    assert!(out.get("obj").unwrap().contains("nested"));
}

// ── profile_kind_label ─────────────────────────────────────────

#[test]
fn profile_kind_label_is_lowercase_string_form() {
    assert_eq!(profile_kind_label(AuthProfileKind::OAuth), "oauth");
    assert_eq!(profile_kind_label(AuthProfileKind::Token), "token");
}

// ── summarize_auth_profile ─────────────────────────────────────

fn profile_fixture(kind: AuthProfileKind, token: Option<&str>) -> AuthProfile {
    let now = Utc::now();
    AuthProfile {
        id: "p:default".into(),
        provider: "p".into(),
        profile_name: "default".into(),
        kind,
        account_id: Some("acct".into()),
        workspace_id: Some("ws".into()),
        token_set: match kind {
            AuthProfileKind::OAuth => Some(TokenSet {
                access_token: "at".into(),
                refresh_token: None,
                id_token: None,
                expires_at: None,
                token_type: None,
                scope: None,
            }),
            AuthProfileKind::Token => None,
        },
        token: token.map(str::to_string),
        metadata: BTreeMap::from([
            ("user_id".to_string(), "u1".to_string()),
            ("email".to_string(), "a@b.c".to_string()),
        ]),
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn summarize_auth_profile_oauth_has_token_set_only() {
    let p = profile_fixture(AuthProfileKind::OAuth, None);
    let summary = summarize_auth_profile(&p);
    assert_eq!(summary.kind, "oauth");
    assert!(!summary.has_token);
    assert!(summary.has_token_set);
    assert_eq!(summary.account_id.as_deref(), Some("acct"));
    assert_eq!(summary.workspace_id.as_deref(), Some("ws"));
    // Metadata keys sorted
    assert_eq!(summary.metadata_keys, vec!["email", "user_id"]);
}

#[test]
fn summarize_auth_profile_token_has_token_only() {
    let p = profile_fixture(AuthProfileKind::Token, Some("raw-token"));
    let summary = summarize_auth_profile(&p);
    assert_eq!(summary.kind, "token");
    assert!(summary.has_token);
    assert!(!summary.has_token_set);
}

#[test]
fn summarize_auth_profile_treats_whitespace_token_as_missing() {
    let p = profile_fixture(AuthProfileKind::Token, Some("   "));
    let summary = summarize_auth_profile(&p);
    assert!(!summary.has_token);
}

// ── session_user_value ─────────────────────────────────────────

#[test]
fn session_user_value_returns_none_without_user_json() {
    let p = profile_fixture(AuthProfileKind::Token, Some("t"));
    assert!(session_user_value(&p).is_none());
}

#[test]
fn session_user_value_parses_stored_user_json_string() {
    let mut p = profile_fixture(AuthProfileKind::Token, Some("t"));
    p.metadata.insert(
        "user_json".into(),
        r#"{"id":"u1","name":"Alice"}"#.to_string(),
    );
    let v = session_user_value(&p).expect("user_json should parse");
    assert_eq!(v["id"], "u1");
    assert_eq!(v["name"], "Alice");
}

#[test]
fn session_user_value_returns_none_for_invalid_user_json() {
    let mut p = profile_fixture(AuthProfileKind::Token, Some("t"));
    p.metadata
        .insert("user_json".into(), "not valid json".to_string());
    assert!(session_user_value(&p).is_none());
}

// ── build_session_state / get_session_token ────────────────────

#[test]
fn build_session_state_returns_unauthenticated_when_store_is_empty() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let state = build_session_state(&config).expect("state");
    assert!(!state.is_authenticated);
    assert!(state.user_id.is_none());
    assert!(state.user.is_none());
    assert!(state.profile_id.is_none());
}

#[test]
fn get_session_token_returns_none_when_store_is_empty() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(get_session_token(&config).unwrap().is_none());
}

/// Regression for CodeRabbit feedback on PR #2085: a profile whose
/// token is whitespace-only must come back as `None`, matching the
/// `is_authenticated` view (which trims + filters empty).
#[test]
fn session_token_from_profile_normalises_blank_tokens_to_none() {
    let p_blank = profile_fixture(AuthProfileKind::Token, Some("   "));
    assert!(session_token_from_profile(Some(&p_blank)).is_none());

    let p_empty = profile_fixture(AuthProfileKind::Token, Some(""));
    assert!(session_token_from_profile(Some(&p_empty)).is_none());

    let p_none = profile_fixture(AuthProfileKind::Token, None);
    assert!(session_token_from_profile(Some(&p_none)).is_none());

    let p_real = profile_fixture(AuthProfileKind::Token, Some("  tok  "));
    // Trim leaks into the returned value — this matches the
    // `is_authenticated` semantic that "  tok  " is a real token.
    assert_eq!(
        session_token_from_profile(Some(&p_real)).as_deref(),
        Some("tok")
    );
}

#[test]
fn get_session_token_returns_stored_token_when_present() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let service = AuthService::from_config(&config);
    service
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "raw-session-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store token");
    assert_eq!(
        get_session_token(&config).unwrap().as_deref(),
        Some("raw-session-token")
    );
    let state = build_session_state(&config).unwrap();
    assert!(state.is_authenticated);
    assert!(state.profile_id.is_some());
}

// ── classify_session_token (local expiry precheck, #3297) ──────────

fn token_profile_with_expiry(token: Option<&str>, expires_at: Option<&str>) -> AuthProfile {
    let mut p = profile_fixture(AuthProfileKind::Token, token);
    match expires_at {
        Some(rfc3339) => {
            p.metadata
                .insert(SESSION_EXPIRES_AT_META.to_string(), rfc3339.to_string());
        }
        None => {
            p.metadata.remove(SESSION_EXPIRES_AT_META);
        }
    }
    p
}

#[test]
fn classify_absent_when_no_profile_or_empty_token() {
    let now = Utc::now();
    assert_eq!(classify_session_token(None, now), SessionTokenCheck::Absent);
    let p = token_profile_with_expiry(Some("   "), None);
    assert_eq!(
        classify_session_token(Some(&p), now),
        SessionTokenCheck::Absent
    );
}

#[test]
fn classify_live_when_no_recorded_expiry() {
    // exp-less / local sessions fall through to presence-only (401 net covers revocation).
    let now = Utc::now();
    let p = token_profile_with_expiry(Some("jwt-token"), None);
    assert_eq!(
        classify_session_token(Some(&p), now),
        SessionTokenCheck::Live("jwt-token".to_string())
    );
}

#[test]
fn classify_live_when_expiry_in_future() {
    let now = Utc::now();
    let future = (now + chrono::Duration::hours(1)).to_rfc3339();
    let p = token_profile_with_expiry(Some("jwt-token"), Some(&future));
    assert_eq!(
        classify_session_token(Some(&p), now),
        SessionTokenCheck::Live("jwt-token".to_string())
    );
}

#[test]
fn classify_expired_when_exp_in_past() {
    let now = Utc::now();
    let past = (now - chrono::Duration::hours(1)).to_rfc3339();
    let p = token_profile_with_expiry(Some("jwt-token"), Some(&past));
    assert_eq!(
        classify_session_token(Some(&p), now),
        SessionTokenCheck::Expired
    );
}

#[test]
fn classify_expired_within_skew_window() {
    // exp is technically in the future but inside the 30s skew → treat as expired
    // so an in-flight request can't race the boundary into a 401.
    let now = Utc::now();
    let soon = (now + chrono::Duration::seconds(10)).to_rfc3339();
    let p = token_profile_with_expiry(Some("jwt-token"), Some(&soon));
    assert_eq!(
        classify_session_token(Some(&p), now),
        SessionTokenCheck::Expired
    );
}
