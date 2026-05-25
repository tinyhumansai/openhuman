//! Session/auth helpers used by RPC and [`crate::core_server::helpers`].

use crate::openhuman::config::Config;

use super::profiles::{AuthProfile, AuthProfileKind, TokenSet};
use super::responses::{AuthProfileSummary, AuthStateResponse};
use super::AuthService;

use super::{APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};

pub const LOCAL_SESSION_USER_ID: &str = "local";

pub fn local_session_user_id() -> String {
    let host = hostname::get()
        .ok()
        .and_then(|value| value.into_string().ok())
        .unwrap_or_default();
    let slug = slugify_local_session_host(&host);
    format!("local-{slug}")
}

fn slugify_local_session_host(host: &str) -> String {
    let mut slug = String::with_capacity(host.len());
    let mut last_was_sep = false;

    for ch in host.trim().chars() {
        let normalized = ch.to_ascii_lowercase();
        if normalized.is_ascii_alphanumeric() {
            slug.push(normalized);
            last_was_sep = false;
        } else if !slug.is_empty() && !last_was_sep {
            slug.push('-');
            last_was_sep = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "device".to_string()
    } else {
        slug
    }
}

pub fn profile_name_or_default(value: Option<&str>) -> &str {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_AUTH_PROFILE_NAME)
}

pub fn is_local_session_token(token: &str) -> bool {
    let trimmed = token.trim();
    let mut parts = trimmed.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(_), Some(_), Some("local"), None)
    )
}

pub fn parse_fields_value(
    input: Option<serde_json::Value>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let Some(value) = input else {
        return Ok(std::collections::HashMap::new());
    };

    let Some(map) = value.as_object() else {
        return Err("fields must be a JSON object".to_string());
    };

    let mut out = std::collections::HashMap::new();
    for (key, raw) in map {
        if key.trim().is_empty() {
            return Err("fields cannot contain empty keys".to_string());
        }
        let rendered = match raw {
            serde_json::Value::Null => String::new(),
            serde_json::Value::String(s) => s.clone(),
            _ => raw.to_string(),
        };
        out.insert(key.clone(), rendered);
    }

    Ok(out)
}

fn profile_kind_label(kind: AuthProfileKind) -> String {
    match kind {
        AuthProfileKind::OAuth => "oauth".to_string(),
        AuthProfileKind::Token => "token".to_string(),
    }
}

pub fn summarize_auth_profile(
    profile: &crate::openhuman::credentials::profiles::AuthProfile,
) -> AuthProfileSummary {
    let mut metadata_keys = profile
        .metadata
        .keys()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>();
    metadata_keys.sort();

    AuthProfileSummary {
        id: profile.id.clone(),
        provider: profile.provider.clone(),
        profile_name: profile.profile_name.clone(),
        kind: profile_kind_label(profile.kind),
        account_id: profile.account_id.clone(),
        workspace_id: profile.workspace_id.clone(),
        metadata_keys,
        updated_at: profile.updated_at.to_rfc3339(),
        has_token: profile.token.as_ref().is_some_and(|v| !v.trim().is_empty()),
        has_token_set: profile
            .token_set
            .as_ref()
            .map(|TokenSet { access_token, .. }| !access_token.trim().is_empty())
            .unwrap_or(false),
    }
}

fn session_user_value(
    profile: &crate::openhuman::credentials::profiles::AuthProfile,
) -> Option<serde_json::Value> {
    profile
        .metadata
        .get("user_json")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
}

pub fn build_session_state(config: &Config) -> Result<AuthStateResponse, String> {
    let profile = load_app_session_profile(config)?;
    Ok(session_state_from_profile(profile.as_ref()))
}

pub fn get_session_token(config: &Config) -> Result<Option<String>, String> {
    let profile = load_app_session_profile(config)?;
    Ok(session_token_from_profile(profile.as_ref()))
}

/// Load the `app-session` profile once. Callers that need both the
/// session-state view (`session_state_from_profile`) AND the raw token
/// (`session_token_from_profile`) should call this once and pass the
/// result to both helpers — every load takes the auth-profile store
/// lock, and on Windows the `app_state_snapshot` hot path used to take
/// it twice per call which materially increased lock contention
/// (Sentry: "Timed out waiting for auth profile lock").
pub fn load_app_session_profile(config: &Config) -> Result<Option<AuthProfile>, String> {
    let auth_service = AuthService::from_config(config);
    auth_service
        .get_profile(APP_SESSION_PROVIDER, None)
        .map_err(|e| e.to_string())
}

pub fn session_state_from_profile(profile: Option<&AuthProfile>) -> AuthStateResponse {
    let Some(profile) = profile else {
        return AuthStateResponse {
            is_authenticated: false,
            user_id: None,
            user: None,
            profile_id: None,
        };
    };

    let is_authenticated = profile
        .token
        .as_ref()
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false);

    AuthStateResponse {
        is_authenticated,
        user_id: profile.metadata.get("user_id").cloned(),
        user: session_user_value(profile),
        profile_id: Some(profile.id.clone()),
    }
}

pub fn session_token_from_profile(profile: Option<&AuthProfile>) -> Option<String> {
    // Mirror the `is_authenticated` check in `session_state_from_profile`
    // (trim + non-empty) so the two views of the same profile never
    // disagree — i.e. we never return `Some("   ")` while reporting
    // `is_authenticated = false`.
    profile
        .and_then(|entry| entry.token.as_deref())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::credentials::profiles::{AuthProfile, AuthProfileKind, TokenSet};
    use chrono::Utc;
    use serde_json::json;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().join("workspace"),
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
}
