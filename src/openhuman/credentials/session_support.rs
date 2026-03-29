//! Session/auth helpers used by RPC and [`crate::core_server::helpers`].

use crate::openhuman::config::Config;

use super::profiles::{AuthProfileKind, TokenSet};
use super::responses::{AuthProfileSummary, AuthStateResponse};
use super::AuthService;

use super::{APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};

pub fn profile_name_or_default(value: Option<&str>) -> &str {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_AUTH_PROFILE_NAME)
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
    let auth_service = AuthService::from_config(config);
    let profile = auth_service
        .get_profile(APP_SESSION_PROVIDER, None)
        .map_err(|e| e.to_string())?;

    let Some(profile) = profile else {
        return Ok(AuthStateResponse {
            is_authenticated: false,
            user_id: None,
            user: None,
            profile_id: None,
        });
    };

    let is_authenticated = profile
        .token
        .as_ref()
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false);

    Ok(AuthStateResponse {
        is_authenticated,
        user_id: profile.metadata.get("user_id").cloned(),
        user: session_user_value(&profile),
        profile_id: Some(profile.id),
    })
}

pub fn get_session_token(config: &Config) -> Result<Option<String>, String> {
    let auth_service = AuthService::from_config(config);
    let profile = auth_service
        .get_profile(APP_SESSION_PROVIDER, None)
        .map_err(|e| e.to_string())?;
    Ok(profile.and_then(|entry| entry.token))
}
