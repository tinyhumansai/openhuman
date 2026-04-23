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

use crate::api::config::effective_api_url;
use crate::api::jwt::get_session_token;
use crate::api::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

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

async fn get_authed_value(
    config: &Config,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let token = require_token(config)?;
    let api_url = effective_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    client
        .authed_json(&token, method, path, body)
        .await
        .map_err(|e| e.to_string())
}

pub async fn get_usage(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(config, Method::GET, "/teams/me/usage", None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team usage fetched from backend",
    ))
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
mod tests {
    use super::*;

    #[test]
    fn build_api_path_encodes_reserved_characters_in_segments() {
        let path = build_api_path(&["teams", "team/with?reserved", "members", "user#frag"])
            .expect("path should build");

        assert_eq!(path, "/teams/team%2Fwith%3Freserved/members/user%23frag");
    }

    #[test]
    fn build_api_path_empty_segments_list_is_root() {
        let path = build_api_path(&[]).expect("path should build");
        assert_eq!(path, "/");
    }

    #[test]
    fn build_api_path_preserves_segment_order() {
        let path = build_api_path(&["a", "b", "c"]).expect("path should build");
        assert_eq!(path, "/a/b/c");
    }

    #[test]
    fn build_api_path_percent_encodes_spaces_and_unicode() {
        let path = build_api_path(&["teams", "with space", "👥"]).expect("path should build");
        assert!(path.contains("with%20space"));
        // Unicode must be percent-encoded (UTF-8 bytes).
        assert!(!path.contains('👥'));
    }

    #[test]
    fn normalize_id_rejects_empty_with_field_name() {
        let err = normalize_id("", "teamId").unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[test]
    fn normalize_id_rejects_whitespace_only() {
        let err = normalize_id("   \t\n", "userId").unwrap_err();
        assert_eq!(err, "userId is required");
    }

    #[test]
    fn normalize_id_trims_and_keeps_body() {
        assert_eq!(normalize_id("  abc  ", "teamId").unwrap(), "abc");
    }

    #[test]
    fn normalize_id_preserves_internal_whitespace() {
        // Only leading/trailing whitespace is stripped — interior is preserved
        // so we don't silently corrupt caller-provided identifiers.
        assert_eq!(normalize_id("a b", "x").unwrap(), "a b");
    }

    // --- pre-HTTP input validation (no network) -----------------------------

    fn cfg() -> Config {
        Config::default()
    }

    #[tokio::test]
    async fn list_members_rejects_empty_team_id() {
        let err = list_members(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn list_members_rejects_whitespace_team_id() {
        let err = list_members(&cfg(), "   ").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn get_team_rejects_empty_team_id() {
        let err = get_team(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn create_team_rejects_empty_name() {
        let err = create_team(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "name is required");
    }

    #[tokio::test]
    async fn create_team_rejects_whitespace_name() {
        let err = create_team(&cfg(), "   ").await.unwrap_err();
        assert_eq!(err, "name is required");
    }

    #[tokio::test]
    async fn update_team_rejects_empty_team_id() {
        let err = update_team(&cfg(), "", Some("new")).await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn delete_team_rejects_empty_team_id() {
        let err = delete_team(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn switch_team_rejects_empty_team_id() {
        let err = switch_team(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn leave_team_rejects_empty_team_id() {
        let err = leave_team(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn join_team_rejects_empty_code() {
        let err = join_team(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "code is required");
    }

    #[tokio::test]
    async fn join_team_rejects_whitespace_code() {
        let err = join_team(&cfg(), "   ").await.unwrap_err();
        assert_eq!(err, "code is required");
    }

    #[tokio::test]
    async fn create_invite_rejects_empty_team_id() {
        let err = create_invite(&cfg(), "", None, None).await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn remove_member_validates_team_id_before_user_id() {
        // Failing input order must be deterministic: team_id is normalized
        // first, so an empty team_id reports the teamId error regardless of
        // the user_id.
        let err = remove_member(&cfg(), "", "someone").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn remove_member_rejects_empty_user_id_when_team_id_valid() {
        let err = remove_member(&cfg(), "t1", "").await.unwrap_err();
        assert_eq!(err, "userId is required");
    }

    #[tokio::test]
    async fn change_member_role_rejects_missing_role() {
        let err = change_member_role(&cfg(), "t1", "u1", "")
            .await
            .unwrap_err();
        assert_eq!(err, "role is required");
    }

    #[tokio::test]
    async fn change_member_role_validates_team_id_first() {
        let err = change_member_role(&cfg(), "", "u1", "admin")
            .await
            .unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn change_member_role_validates_user_id_before_role() {
        let err = change_member_role(&cfg(), "t1", "", "admin")
            .await
            .unwrap_err();
        assert_eq!(err, "userId is required");
    }

    #[tokio::test]
    async fn list_invites_rejects_empty_team_id() {
        let err = list_invites(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn revoke_invite_rejects_empty_team_id() {
        let err = revoke_invite(&cfg(), "", "inv1").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn revoke_invite_rejects_empty_invite_id() {
        let err = revoke_invite(&cfg(), "t1", "").await.unwrap_err();
        assert_eq!(err, "inviteId is required");
    }
}
