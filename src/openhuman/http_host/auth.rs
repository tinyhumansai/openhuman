use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use rand::RngExt as _;

use crate::openhuman::config;
use crate::openhuman::credentials::session_support;
use crate::openhuman::http_host::types::HostedDirAuth;
use crate::openhuman::http_host::LOG_PREFIX;

pub(crate) fn ensure_authorized(headers: &HeaderMap, auth: &HostedDirAuth) -> Result<(), Response> {
    if !auth.enabled {
        return Ok(());
    }
    let Some(expected_user) = auth.username.as_deref() else {
        return Err(unauthorized_response());
    };
    let Some(expected_pass) = auth.password.as_deref() else {
        return Err(unauthorized_response());
    };
    let Some(header_value) = headers.get(header::AUTHORIZATION) else {
        return Err(unauthorized_response());
    };
    let Ok(auth_value) = header_value.to_str() else {
        return Err(unauthorized_response());
    };
    let Some(encoded) = auth_value.strip_prefix("Basic ") else {
        return Err(unauthorized_response());
    };
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded.as_bytes()) else {
        return Err(unauthorized_response());
    };
    let Ok(rendered) = String::from_utf8(decoded) else {
        return Err(unauthorized_response());
    };
    let Some((username, password)) = rendered.split_once(':') else {
        return Err(unauthorized_response());
    };
    if username == expected_user && password == expected_pass {
        Ok(())
    } else {
        Err(unauthorized_response())
    }
}

fn unauthorized_response() -> Response {
    let mut response = (StatusCode::UNAUTHORIZED, "basic auth required").into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Basic realm=\"OpenHuman Hosted Directory\""),
    );
    response
}

pub(crate) async fn resolve_default_auth_username() -> Option<String> {
    let config = match config::load_config_with_timeout().await {
        Ok(config) => config,
        Err(error) => {
            log::debug!("{LOG_PREFIX} default auth username config load failed: {error}");
            return fallback_env_username();
        }
    };

    match session_support::build_session_state(&config) {
        Ok(state) => state
            .user
            .as_ref()
            .and_then(resolve_default_auth_username_from_user_value)
            .or(state.user_id)
            .and_then(|value| sanitize_basic_auth_username(Some(value))),
        Err(error) => {
            log::debug!("{LOG_PREFIX} session state lookup failed for auth username: {error}");
            fallback_env_username()
        }
    }
}

fn fallback_env_username() -> Option<String> {
    sanitize_basic_auth_username(
        std::env::var("USER")
            .ok()
            .or_else(|| std::env::var("USERNAME").ok()),
    )
}

pub(crate) fn resolve_default_auth_username_from_user_value(
    user: &serde_json::Value,
) -> Option<String> {
    let object = user.as_object()?;
    [
        "username",
        "userName",
        "handle",
        "slug",
        "name",
        "displayName",
        "display_name",
        "email",
        "user_id",
        "userId",
        "id",
    ]
    .iter()
    .find_map(|key| object.get(*key).and_then(|value| value.as_str()))
    .map(str::to_string)
}

pub(crate) fn sanitize_basic_auth_username(value: Option<String>) -> Option<String> {
    let raw = value?;
    let mut out = String::new();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    for ch in trimmed.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' | '@' => out.push(ch),
            ' ' => out.push('-'),
            ':' => {}
            _ => {}
        }
        if out.len() >= 64 {
            break;
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(crate) fn generate_password() -> String {
    let mut bytes = [0u8; 18];
    rand::rng().fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
