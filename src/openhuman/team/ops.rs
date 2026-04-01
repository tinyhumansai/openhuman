//! Team management RPC ops — thin adapters that call the hosted API.
//!
//! # Security
//! All methods require a valid app-session JWT stored via `auth_store_session`.
//! The JWT is sent as `Authorization: Bearer …` to the backend.
//! **No server-side authorization is replicated here**: the backend enforces team
//! ownership, role permissions, and tenant isolation on every request.
//! Callers without the required role (e.g. non-owner trying to remove a member)
//! receive a backend 401/403 surfaced verbatim as an RPC error string.
//! API keys / JWTs are never written to logs.

use log::debug;
use reqwest::{header::AUTHORIZATION, Client, Method, Url};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

use crate::api::config::effective_api_url;
use crate::api::jwt::{bearer_authorization_value, get_session_token};
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[team]";

fn build_client() -> Result<Client, String> {
    Client::builder()
        .use_rustls_tls()
        .http1_only()
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))
}

fn resolve_base(config: &Config) -> Result<Url, String> {
    let base = effective_api_url(&config.api_url);
    Url::parse(base.trim()).map_err(|e| format!("invalid api_url '{}': {e}", base))
}

fn require_token(config: &Config) -> Result<String, String> {
    get_session_token(config)?
        .and_then(|v| {
            let t = v.trim().to_string();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        })
        .ok_or_else(|| "no backend session token; run auth_store_session first".to_string())
}

fn normalize_id(input: &str, field: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} is required"));
    }
    Ok(trimmed.to_string())
}

fn build_api_path(segments: &[&str]) -> Result<String, String> {
    let mut url = Url::parse("https://openhuman.invalid")
        .map_err(|e| format!("failed to initialize URL path builder: {e}"))?;
    {
        let mut path_segments = url
            .path_segments_mut()
            .map_err(|_| "failed to initialize URL path builder".to_string())?;
        path_segments.clear();
        for segment in segments {
            path_segments.push(segment);
        }
    }
    Ok(url.path().to_string())
}

fn is_identifier_segment(segment: &str) -> bool {
    let trimmed = segment.trim();
    if trimmed.is_empty() {
        return false;
    }

    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '%' | '.');
    let has_digit = trimmed.chars().any(|c| c.is_ascii_digit());
    let is_uuid_like = trimmed.len() >= 8
        && trimmed.chars().all(allowed)
        && trimmed.contains('-')
        && trimmed.chars().any(|c| c.is_ascii_hexdigit());

    (has_digit && trimmed.chars().all(allowed)) || is_uuid_like
}

fn redact_route_template(url: &Url) -> String {
    let Some(segments) = url.path_segments() else {
        return url.path().to_string();
    };

    let segments = segments.collect::<Vec<_>>();
    let redacted = segments
        .iter()
        .enumerate()
        .map(|(idx, segment)| {
            match (
                idx,
                segments.first().copied(),
                segments.get(2).copied(),
                *segment,
            ) {
                (1, Some("teams"), _, _) => "{team_id}".to_string(),
                (3, Some("teams"), Some("members"), _) => "{user_id}".to_string(),
                (3, Some("teams"), Some("invites"), _) => "{invite_id}".to_string(),
                (_, _, _, value)
                    if is_identifier_segment(value)
                        && !matches!(value, "teams" | "members" | "invites" | "role") =>
                {
                    "{id}".to_string()
                }
                (_, _, _, value) => value.to_string(),
            }
        })
        .collect::<Vec<_>>();

    format!("/{}", redacted.join("/"))
}

async fn authed_request(
    client: &Client,
    base: &Url,
    token: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let url = base
        .join(path.trim_start_matches('/'))
        .map_err(|e| format!("build URL failed: {e}"))?;

    let mut req = client
        .request(method.clone(), url.clone())
        .header(AUTHORIZATION, bearer_authorization_value(token));

    if let Some(b) = body {
        req = req.json(&b);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("failed to read backend response body: {e}"))?;

    debug!(
        "{LOG_PREFIX} {} {} -> {}",
        method,
        redact_route_template(&url),
        status
    );

    let raw: Value = serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text.clone()));
    if !status.is_success() {
        let msg = raw
            .as_object()
            .and_then(|o| {
                o.get("message")
                    .or_else(|| o.get("error"))
                    .or_else(|| o.get("detail"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or(&text);
        return Err(format!(
            "backend responded with {} for {}: {}",
            status.as_u16(),
            url.path(),
            msg
        ));
    }

    unwrap_api_envelope(raw)
}

fn unwrap_api_envelope(raw: Value) -> Result<Value, String> {
    if let Some(obj) = raw.as_object() {
        if let Some(success) = obj.get("success").and_then(|v| v.as_bool()) {
            if !success {
                let msg = obj
                    .get("message")
                    .or_else(|| obj.get("error"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("request unsuccessful");
                return Err(msg.to_string());
            }
        }
        if let Some(data) = obj.get("data") {
            return Ok(data.clone());
        }
    }
    Ok(raw)
}

async fn get_authed_value(
    config: &Config,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let token = require_token(config)?;
    let client = build_client()?;
    let base = resolve_base(config)?;
    authed_request(&client, &base, &token, method, path, body).await
}

pub async fn list_members(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id, "members"])?;
    let data = get_authed_value(config, Method::GET, &path, None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team members fetched from backend",
    ))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InviteBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_uses: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in_days: Option<u64>,
}

pub async fn create_invite(
    config: &Config,
    team_id: &str,
    max_uses: Option<u64>,
    expires_in_days: Option<u64>,
) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id, "invites"])?;
    let body = json!(InviteBody {
        max_uses,
        expires_in_days,
    });
    let data = get_authed_value(config, Method::POST, &path, Some(body)).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team invite created via backend",
    ))
}

pub async fn remove_member(
    config: &Config,
    team_id: &str,
    user_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let user_id = normalize_id(user_id, "userId")?;
    let path = build_api_path(&["teams", &team_id, "members", &user_id])?;
    let data = get_authed_value(config, Method::DELETE, &path, None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team member removed via backend",
    ))
}

pub async fn change_member_role(
    config: &Config,
    team_id: &str,
    user_id: &str,
    role: &str,
) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let user_id = normalize_id(user_id, "userId")?;
    let role = normalize_id(role, "role")?;
    let path = build_api_path(&["teams", &team_id, "members", &user_id, "role"])?;
    let body = json!({ "role": role });
    let data = get_authed_value(config, Method::PUT, &path, Some(body)).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team member role updated via backend",
    ))
}

/// List all active invites for a team.
/// Maps to `GET /teams/:teamId/invites` — matches `teamApi.getInvites`.
pub async fn list_invites(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id, "invites"])?;
    let data = get_authed_value(config, Method::GET, &path, None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team invites listed from backend",
    ))
}

/// Revoke (delete) an existing invite by id.
/// Maps to `DELETE /teams/:teamId/invites/:inviteId` — matches `teamApi.revokeInvite`.
pub async fn revoke_invite(
    config: &Config,
    team_id: &str,
    invite_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let invite_id = normalize_id(invite_id, "inviteId")?;
    let path = build_api_path(&["teams", &team_id, "invites", &invite_id])?;
    let data = get_authed_value(config, Method::DELETE, &path, None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team invite revoked via backend",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_api_path_encodes_reserved_characters_in_segments() {
        let path = build_api_path(&["teams", "team/with?reserved", "members", "user#frag"])
            .expect("path should build");

        assert_eq!(path, "/teams/team%2Fwith%3Freserved/members/user%23frag");
    }

    #[test]
    fn redact_route_template_hides_team_member_and_invite_ids() {
        let members_url =
            Url::parse("https://api.example.test/teams/team-1/members").expect("members url");
        assert_eq!(
            redact_route_template(&members_url),
            "/teams/{team_id}/members"
        );

        let member_role_url = Url::parse(
            "https://api.example.test/teams/69ca3f94bc6e00bbdc551900/members/user-2/role",
        )
        .expect("member role url");
        assert_eq!(
            redact_route_template(&member_role_url),
            "/teams/{team_id}/members/{user_id}/role"
        );

        let invite_url =
            Url::parse("https://api.example.test/teams/team-1/invites/inv-1").expect("invite url");
        assert_eq!(
            redact_route_template(&invite_url),
            "/teams/{team_id}/invites/{invite_id}"
        );
    }
}
