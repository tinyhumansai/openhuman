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

use reqwest::{Method, Url};
use serde::Serialize;
use serde_json::{json, Value};

use openhuman_core::api::config::effective_backend_api_url;
use openhuman_core::api::BackendOAuthClient;
use openhuman_core::config::Config;
use openhuman_core::rpc::RpcOutcome;

/// Canonical authed-session guard. Delegates to `require_live_session_token`,
/// which rejects an expired token locally (publishing `SessionExpired`) instead
/// of firing a doomed backend 401 — see #3297 / `session_support`.
fn require_token(
    config: &Config,
) -> Result<openhuman_core::security::credentials::session_support::BackendCredential, String> {
    openhuman_core::security::credentials::session_support::resolve_backend_credential(config)
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

async fn get_authed_value(
    config: &Config,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let token = require_token(config)?;
    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| format!("{e:#}"))?;
    // `flatten_authed_error` maps the typed `BackendApiError::Unauthorized`
    // (expected session-lapse 401) onto the `SESSION_EXPIRED` sentinel so the
    // JSON-RPC layer classifies it as session expiry and skips Sentry (#3297,
    // TAURI-RUST-8WY on `/teams/me/usage`); every other error keeps its full
    // `{e:#}` anyhow chain. `authed_json` wraps the underlying reqwest error
    // with `.context(format!("backend request {} {}", …))`
    // (`api/rest.rs::authed_json`), so `{e:#}` (not `e.to_string()`) is required
    // to surface the cause (connect timeout, DNS failure, TLS handshake, non-2xx
    // status, …) before the JSON-RPC layer reports it to Sentry.
    // OPENHUMAN-TAURI-AD is the canonical instance: 2 events on `0.53.35` from a
    // Russia user, all with the truncated label and elapsed_ms=49 — far too
    // short for a real timeout, so the underlying cause is the only signal worth
    // surfacing. Same failure mode the `report_error` doc-string in
    // `core/observability.rs` calls out (TAURI-B2).
    client
        .authed_json(&token, method, path, body)
        .await
        .map_err(openhuman_core::api::flatten_authed_error)
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

pub async fn list_teams(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(config, Method::GET, "/teams", None).await?;
    Ok(RpcOutcome::single_log(data, "teams fetched from backend"))
}

pub async fn get_team(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id])?;
    let data = get_authed_value(config, Method::GET, &path, None).await?;
    Ok(RpcOutcome::single_log(data, "team fetched from backend"))
}

#[derive(Debug, Serialize)]
struct TeamNameBody<'a> {
    name: &'a str,
}

pub async fn create_team(config: &Config, name: &str) -> Result<RpcOutcome<Value>, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("name is required".to_string());
    }
    let data = get_authed_value(
        config,
        Method::POST,
        "/teams",
        Some(json!(TeamNameBody { name: trimmed })),
    )
    .await?;
    Ok(RpcOutcome::single_log(data, "team created via backend"))
}

pub async fn update_team(
    config: &Config,
    team_id: &str,
    name: Option<&str>,
) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id])?;
    let mut body = serde_json::Map::new();
    if let Some(name) = name.map(str::trim).filter(|value| !value.is_empty()) {
        body.insert("name".to_string(), Value::String(name.to_string()));
    }
    let data = get_authed_value(config, Method::PUT, &path, Some(Value::Object(body))).await?;
    Ok(RpcOutcome::single_log(data, "team updated via backend"))
}

pub async fn delete_team(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id])?;
    let data = get_authed_value(config, Method::DELETE, &path, None).await?;
    Ok(RpcOutcome::single_log(data, "team deleted via backend"))
}

pub async fn switch_team(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id, "switch"])?;
    let data = get_authed_value(config, Method::POST, &path, Some(json!({}))).await?;
    Ok(RpcOutcome::single_log(
        data,
        "active team switched via backend",
    ))
}

pub async fn leave_team(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id, "leave"])?;
    let data = get_authed_value(config, Method::POST, &path, Some(json!({}))).await?;
    Ok(RpcOutcome::single_log(data, "team left via backend"))
}

pub async fn join_team(config: &Config, code: &str) -> Result<RpcOutcome<Value>, String> {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return Err("code is required".to_string());
    }
    let data = get_authed_value(
        config,
        Method::POST,
        "/teams/join",
        Some(json!({ "code": trimmed })),
    )
    .await?;
    Ok(RpcOutcome::single_log(data, "team joined via backend"))
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
#[path = "ops_tests.rs"]
mod tests;
