//! Team management RPC ops — thin adapters over the SDK's typed `teams()`
//! client.
//!
//! # Security
//! Every call authenticates with the core's resolved backend credential
//! ([`HostedClient`]). **No server-side authorization is replicated here**:
//! the backend enforces team ownership, role permissions and tenant isolation
//! on every request. Callers without the required role receive the backend's
//! error surfaced as an RPC error string. Credentials are never logged.
//!
//! Input validation always runs before the credential is resolved, so a bad
//! argument reports the argument, never "not signed in".

use reqwest::Method;
use serde_json::{json, Value};
use tinyhumans_sdk::api::types::{CodeRequest, CreateTeamInviteRequest};

use openhuman_core::api::config::effective_backend_api_url;
use openhuman_core::api::BackendOAuthClient;
use openhuman_core::config::Config;
use openhuman_core::integrations::client::budget_gate;
use openhuman_core::rpc::RpcOutcome;

use crate::hosted::client::HostedClient;

fn normalize_id(input: &str, field: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} is required"));
    }
    Ok(trimmed.to_string())
}

fn clamp_u32(value: Option<u64>, field: &str) -> Result<Option<u32>, String> {
    value
        .map(|v| u32::try_from(v).map_err(|_| format!("{field} is out of range")))
        .transpose()
}

/// `GET /teams/me/usage` — the usage document behind the frontend's usage
/// poll. Wrapped in the core's shared failure backoff
/// ([`budget_gate::usage_with_failure_backoff`]) so a persistent backend
/// fault collapses to about one probe a minute across this RPC *and* the
/// managed-tool budget gate (GH #4153).
///
/// The credential is resolved first: a user with no TinyHumans account gets
/// the core's `BACKEND_UNAVAILABLE:` sentinel without a request and without
/// opening a backoff streak (Sentry 36649).
pub async fn get_usage(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let backend_key = effective_backend_api_url(&config.api_url);
    budget_gate::usage_with_failure_backoff(&backend_key, || async {
        client.finish_value(
            "GET /teams/me/usage",
            client.sdk().teams().get_my_usage().await,
        )
    })
    .await
}

pub async fn list_members(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "GET /teams/{teamId}/members",
        client.sdk().teams().list_members(&team_id).await,
    )?;
    Ok(RpcOutcome::single_log(
        data,
        "team members fetched from backend",
    ))
}

pub async fn list_teams(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value("GET /teams", client.sdk().teams().list_teams().await)?;
    Ok(RpcOutcome::single_log(data, "teams fetched from backend"))
}

pub async fn get_team(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "GET /teams/{teamId}",
        client.sdk().teams().get_team(&team_id).await,
    )?;
    Ok(RpcOutcome::single_log(data, "team fetched from backend"))
}

/// `POST /teams` through the core's `BackendOAuthClient::authed_json`.
///
/// Deliberately **not** on the SDK: the SDK has no typed method for this route
/// and its generated public-route registry has no entry for it (teams were
/// folded into users on the backend). Until the route is added to the SDK
/// upstream, or the RPC is retired, it stays on the pre-SDK path unchanged.
pub async fn create_team(config: &Config, name: &str) -> Result<RpcOutcome<Value>, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("name is required".to_string());
    }
    let data = legacy_authed_value(
        config,
        Method::POST,
        "/teams",
        Some(json!({ "name": trimmed })),
    )
    .await?;
    Ok(RpcOutcome::single_log(data, "team created via backend"))
}

/// The pre-SDK request path for the two team routes the SDK does not carry
/// ([`create_team`], [`delete_team`]). Credential first, so a user without a
/// TinyHumans account still gets the sentinel without a request.
async fn legacy_authed_value(
    config: &Config,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let credential =
        openhuman_core::security::credentials::session_support::resolve_backend_credential(config)?;
    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| format!("{e:#}"))?;
    client
        .authed_json(credential, method, path, body)
        .await
        .map_err(openhuman_core::api::flatten_authed_error)
}

pub async fn update_team(
    config: &Config,
    team_id: &str,
    name: Option<&str>,
) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let mut body = serde_json::Map::new();
    if let Some(name) = name.map(str::trim).filter(|value| !value.is_empty()) {
        body.insert("name".to_string(), Value::String(name.to_string()));
    }
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "PUT /teams/{teamId}",
        client
            .sdk()
            .teams()
            .update_team(&team_id, &Value::Object(body))
            .await,
    )?;
    Ok(RpcOutcome::single_log(data, "team updated via backend"))
}

/// `DELETE /teams/{teamId}` on the pre-SDK path — see [`create_team`].
pub async fn delete_team(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = format!("/teams/{}", tinyhumans_sdk::enc(&team_id));
    let data = legacy_authed_value(config, Method::DELETE, &path, None).await?;
    Ok(RpcOutcome::single_log(data, "team deleted via backend"))
}

pub async fn switch_team(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "POST /teams/{teamId}/switch",
        client.sdk().teams().switch_team(&team_id).await,
    )?;
    Ok(RpcOutcome::single_log(
        data,
        "active team switched via backend",
    ))
}

pub async fn leave_team(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "POST /teams/{teamId}/leave",
        client.sdk().teams().leave_team(&team_id).await,
    )?;
    Ok(RpcOutcome::single_log(data, "team left via backend"))
}

pub async fn join_team(config: &Config, code: &str) -> Result<RpcOutcome<Value>, String> {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return Err("code is required".to_string());
    }
    let client = HostedClient::from_config(config)?;
    let request = CodeRequest {
        code: trimmed.to_string(),
    };
    let data = client.finish_value(
        "POST /teams/join",
        client.sdk().teams().join_team(&request).await,
    )?;
    Ok(RpcOutcome::single_log(data, "team joined via backend"))
}

pub async fn create_invite(
    config: &Config,
    team_id: &str,
    max_uses: Option<u64>,
    expires_in_days: Option<u64>,
) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let request = CreateTeamInviteRequest {
        max_uses: clamp_u32(max_uses, "maxUses")?,
        expires_in_days: clamp_u32(expires_in_days, "expiresInDays")?,
    };
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "POST /teams/{teamId}/invites",
        client.sdk().teams().create_invite(&team_id, &request).await,
    )?;
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
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "DELETE /teams/{teamId}/members/{userId}",
        client.sdk().teams().remove_member(&team_id, &user_id).await,
    )?;
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
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "PUT /teams/{teamId}/members/{userId}/role",
        client
            .sdk()
            .teams()
            .update_member_role(&team_id, &user_id, &json!({ "role": role }))
            .await,
    )?;
    Ok(RpcOutcome::single_log(
        data,
        "team member role updated via backend",
    ))
}

/// List all active invites for a team (`GET /teams/:teamId/invites`).
pub async fn list_invites(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "GET /teams/{teamId}/invites",
        client.sdk().teams().list_invites(&team_id).await,
    )?;
    Ok(RpcOutcome::single_log(
        data,
        "team invites listed from backend",
    ))
}

/// Revoke an invite (`DELETE /teams/:teamId/invites/:inviteId`).
pub async fn revoke_invite(
    config: &Config,
    team_id: &str,
    invite_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let invite_id = normalize_id(invite_id, "inviteId")?;
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "DELETE /teams/{teamId}/invites/{inviteId}",
        client
            .sdk()
            .teams()
            .revoke_invite(&team_id, &invite_id)
            .await,
    )?;
    Ok(RpcOutcome::single_log(
        data,
        "team invite revoked via backend",
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
