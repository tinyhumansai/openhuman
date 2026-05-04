use crate::api::config::effective_api_url;
use crate::api::jwt::get_session_token;
use crate::api::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::openhuman::webhooks::{
    WebhookDebugLogListResult, WebhookDebugLogsClearedResult, WebhookDebugRegistrationsResult,
    WebhookRequest, WebhookResponseData,
};
use crate::rpc::RpcOutcome;
use base64::Engine;
use reqwest::Method;
use serde_json::Value;
use std::collections::HashMap;

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

/// Retrieve the global webhook router, returning an error if the socket
/// manager or router is not yet initialised.
fn get_router() -> Result<std::sync::Arc<crate::openhuman::webhooks::WebhookRouter>, String> {
    crate::openhuman::socket::global_socket_manager()
        .ok_or_else(|| "socket manager not initialized".to_string())?
        .webhook_router()
        .ok_or_else(|| "webhook router not initialized".to_string())
}

pub async fn list_registrations() -> Result<RpcOutcome<WebhookDebugRegistrationsResult>, String> {
    match get_router() {
        Ok(router) => {
            let registrations = router.list_all();
            let count = registrations.len();
            Ok(RpcOutcome::single_log(
                WebhookDebugRegistrationsResult { registrations },
                format!("webhooks.list_registrations returned {count} registration(s)"),
            ))
        }
        Err(_) => {
            // Router not yet initialized — return empty list (not an error in RPC).
            Ok(RpcOutcome::single_log(
                WebhookDebugRegistrationsResult {
                    registrations: Vec::new(),
                },
                "webhooks.list_registrations returned 0 registration(s) (router not initialized)"
                    .to_string(),
            ))
        }
    }
}

pub async fn list_logs(
    limit: Option<usize>,
) -> Result<RpcOutcome<WebhookDebugLogListResult>, String> {
    match get_router() {
        Ok(router) => {
            let logs = router.list_logs(limit);
            let count = logs.len();
            Ok(RpcOutcome::single_log(
                WebhookDebugLogListResult { logs },
                format!("webhooks.list_logs returned {count} log entrie(s)"),
            ))
        }
        Err(_) => Ok(RpcOutcome::single_log(
            WebhookDebugLogListResult { logs: Vec::new() },
            "webhooks.list_logs returned 0 log entrie(s) (router not initialized)".to_string(),
        )),
    }
}

pub async fn clear_logs() -> Result<RpcOutcome<WebhookDebugLogsClearedResult>, String> {
    match get_router() {
        Ok(router) => {
            let cleared = router.clear_logs();
            Ok(RpcOutcome::single_log(
                WebhookDebugLogsClearedResult { cleared },
                format!("webhooks.clear_logs removed {cleared} log entrie(s)"),
            ))
        }
        Err(_) => Ok(RpcOutcome::single_log(
            WebhookDebugLogsClearedResult { cleared: 0 },
            "webhooks.clear_logs removed 0 log entrie(s) (router not initialized)".to_string(),
        )),
    }
}

pub async fn register_echo(
    tunnel_uuid: &str,
    tunnel_name: Option<String>,
    backend_tunnel_id: Option<String>,
) -> Result<RpcOutcome<WebhookDebugRegistrationsResult>, String> {
    let router = get_router().map_err(|e| format!("webhooks.register_echo failed: {e}"))?;
    router.register_echo(tunnel_uuid, tunnel_name, backend_tunnel_id)?;
    let registrations = router.list_all();
    Ok(RpcOutcome::single_log(
        WebhookDebugRegistrationsResult { registrations },
        format!("webhooks.register_echo registered tunnel {tunnel_uuid}"),
    ))
}

pub async fn unregister_echo(
    tunnel_uuid: &str,
) -> Result<RpcOutcome<WebhookDebugRegistrationsResult>, String> {
    let router = get_router().map_err(|e| format!("webhooks.unregister_echo failed: {e}"))?;
    router.unregister(tunnel_uuid, "echo")?;
    let registrations = router.list_all();
    Ok(RpcOutcome::single_log(
        WebhookDebugRegistrationsResult { registrations },
        format!("webhooks.unregister_echo removed tunnel {tunnel_uuid}"),
    ))
}

/// Register an agent-backed webhook tunnel.
///
/// Incoming requests on this tunnel will be routed to the triage
/// pipeline instead of the (removed) skill runtime.
pub async fn register_agent(
    tunnel_uuid: &str,
    agent_id: Option<String>,
    tunnel_name: Option<String>,
    backend_tunnel_id: Option<String>,
) -> Result<RpcOutcome<WebhookDebugRegistrationsResult>, String> {
    let router = get_router().map_err(|e| format!("webhooks.register_agent failed: {e}"))?;
    router.register_agent(tunnel_uuid, agent_id, tunnel_name, backend_tunnel_id)?;
    let registrations = router.list_all();
    Ok(RpcOutcome::single_log(
        WebhookDebugRegistrationsResult { registrations },
        format!("webhooks.register_agent registered agent tunnel {tunnel_uuid}"),
    ))
}

/// Trigger the triage/agent pipeline directly via RPC without requiring
/// an incoming webhook request. Useful for testing and manual escalation.
pub async fn trigger_agent(
    source: &str,
    caller_id: &str,
    reason: &str,
    payload: serde_json::Value,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    use crate::openhuman::agent::triage::TriggerEnvelope;

    let envelope = match source {
        "webhook" => TriggerEnvelope::from_webhook(caller_id, "POST", "/trigger", payload),
        "cron" => {
            let output = payload
                .get("output")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(reason);
            TriggerEnvelope::from_cron(caller_id, reason, output)
        }
        "external" => TriggerEnvelope::from_external(caller_id, reason, payload),
        other => {
            return Err(format!(
                "unsupported trigger source `{other}` — supported: webhook, cron, external"
            ))
        }
    };

    let run = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        crate::openhuman::agent::triage::run_triage(&envelope),
    )
    .await
    .map_err(|_| "triage timed out after 60s".to_string())?
    .map_err(|e| format!("triage failed: {e}"))?;

    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        crate::openhuman::agent::triage::apply_decision(run.clone(), &envelope),
    )
    .await
    .map_err(|_| "apply_decision timed out after 60s".to_string())?
    .map_err(|e| format!("apply_decision failed: {e}"))?;

    Ok(RpcOutcome::single_log(
        serde_json::json!({
            "decision": run.decision.action.as_str(),
            "target_agent": run.decision.target_agent,
            "prompt": run.decision.prompt,
            "reason": run.decision.reason,
        }),
        format!("webhooks.trigger_agent completed for {source}/{caller_id}"),
    ))
}

pub fn build_echo_response(request: &WebhookRequest) -> WebhookResponseData {
    let response_body = serde_json::json!({
        "ok": true,
        "echo": {
            "correlationId": request.correlation_id,
            "tunnelId": request.tunnel_id,
            "tunnelUuid": request.tunnel_uuid,
            "tunnelName": request.tunnel_name,
            "method": request.method,
            "path": request.path,
            "query": request.query,
            "headers": request.headers,
            "bodyBase64": request.body,
        }
    });

    let mut headers = HashMap::new();
    headers.insert("content-type".to_string(), "application/json".to_string());
    headers.insert("x-openhuman-webhook-target".to_string(), "echo".to_string());

    WebhookResponseData {
        correlation_id: request.correlation_id.clone(),
        status_code: 200,
        headers,
        body: base64::engine::general_purpose::STANDARD.encode(response_body.to_string()),
    }
}

pub async fn list_tunnels(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(config, Method::GET, "/webhooks/core", None).await?;
    Ok(RpcOutcome::single_log(data, "webhook tunnels fetched"))
}

pub async fn create_tunnel(
    config: &Config,
    name: &str,
    description: Option<String>,
) -> Result<RpcOutcome<Value>, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is required".to_string());
    }
    let mut body_map = serde_json::Map::new();
    body_map.insert(
        "name".to_string(),
        serde_json::Value::String(name.to_string()),
    );
    if let Some(desc) = description {
        let desc = desc.trim().to_string();
        if !desc.is_empty() {
            body_map.insert("description".to_string(), serde_json::Value::String(desc));
        }
    }
    let body = serde_json::Value::Object(body_map);
    let data = get_authed_value(config, Method::POST, "/webhooks/core", Some(body)).await?;
    Ok(RpcOutcome::single_log(data, "webhook tunnel created"))
}

pub async fn get_tunnel(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("id is required".to_string());
    }
    let encoded_id = urlencoding::encode(id);
    let data = get_authed_value(
        config,
        Method::GET,
        &format!("/webhooks/core/{encoded_id}"),
        None,
    )
    .await?;
    Ok(RpcOutcome::single_log(data, "webhook tunnel fetched"))
}

pub async fn update_tunnel(
    config: &Config,
    id: &str,
    payload: Value,
) -> Result<RpcOutcome<Value>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("id is required".to_string());
    }
    let encoded_id = urlencoding::encode(id);
    let data = get_authed_value(
        config,
        Method::PATCH,
        &format!("/webhooks/core/{encoded_id}"),
        Some(payload),
    )
    .await?;
    Ok(RpcOutcome::single_log(data, "webhook tunnel updated"))
}

pub async fn delete_tunnel(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("id is required".to_string());
    }
    let encoded_id = urlencoding::encode(id);
    let data = get_authed_value(
        config,
        Method::DELETE,
        &format!("/webhooks/core/{encoded_id}"),
        None,
    )
    .await?;
    Ok(RpcOutcome::single_log(data, "webhook tunnel deleted"))
}

pub async fn get_bandwidth(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(config, Method::GET, "/webhooks/core/bandwidth", None).await?;
    Ok(RpcOutcome::single_log(data, "webhook bandwidth fetched"))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
