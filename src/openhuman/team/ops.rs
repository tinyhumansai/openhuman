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
    let text = resp.text().await.unwrap_or_default();

    debug!("{LOG_PREFIX} {} {} -> {}", method, url, status);

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
    let path = format!("/teams/{}/members", team_id);
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
    let path = format!("/teams/{}/invites", team_id);
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
    let path = format!("/teams/{}/members/{}", team_id, user_id);
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
    let path = format!("/teams/{}/members/{}/role", team_id, user_id);
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
    let path = format!("/teams/{}/invites", team_id);
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
    let path = format!("/teams/{}/invites/{}", team_id, invite_id);
    let data = get_authed_value(config, Method::DELETE, &path, None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team invite revoked via backend",
    ))
}
