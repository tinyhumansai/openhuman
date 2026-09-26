use crate::rpc::RpcOutcome;
use crate::skills::webhooks::{
    WebhookDebugLogListResult, WebhookDebugLogsClearedResult, WebhookDebugRegistrationsResult,
    WebhookRequest, WebhookResponseData,
};
use base64::Engine;
use std::collections::HashMap;

/// Retrieve the global webhook router, returning an error if the socket
/// manager or router is not yet initialised.
fn get_router() -> Result<std::sync::Arc<crate::skills::webhooks::WebhookRouter>, String> {
    crate::platform::socket::global_socket_manager()
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
    let removed = router.unregister(tunnel_uuid, "echo")?;
    let registrations = router.list_all();
    // The wire shape is unchanged (#6091 leaves that a separate contract call):
    // the caller still gets the full registration list and can diff it. What
    // changes is that the log no longer claims a removal that did not happen.
    let log = if removed {
        format!("webhooks.unregister_echo removed tunnel {tunnel_uuid}")
    } else {
        format!(
            "webhooks.unregister_echo: no registration for tunnel {tunnel_uuid}, nothing removed"
        )
    };
    Ok(RpcOutcome::single_log(
        WebhookDebugRegistrationsResult { registrations },
        log,
    ))
}

/// Register an agent-backed webhook tunnel.
///
/// Incoming requests on this tunnel will be routed to the triage
/// pipeline instead of direct skill dispatch.
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
    use crate::agent::triage::TriggerEnvelope;

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

    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        crate::agent::triage::run_triage(&envelope),
    )
    .await
    .map_err(|_| "triage timed out after 60s".to_string())?
    .map_err(|e| format!("triage failed: {e}"))?;

    match outcome {
        crate::agent::triage::TriageOutcome::Decision(run) => {
            // Remote payload: a webhook body is attacker-influenceable, so the
            // dispatch parks rather than running on a trust root (#5634).
            let origin = crate::agent::triage::remote_trigger_origin(&envelope);
            tokio::time::timeout(
                std::time::Duration::from_secs(60),
                crate::agent::turn_origin::with_origin(
                    origin,
                    crate::agent::triage::apply_decision(run.clone(), &envelope),
                ),
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
                    "resolution_path": run.resolution_path.as_str(),
                }),
                format!("webhooks.trigger_agent completed for {source}/{caller_id}"),
            ))
        }
        crate::agent::triage::TriageOutcome::Deferred {
            defer_until_ms,
            reason,
        } => Ok(RpcOutcome::single_log(
            serde_json::json!({
                "decision": "deferred",
                "resolution_path": "deferred",
                "defer_until_ms": defer_until_ms,
                "reason": reason,
            }),
            format!("webhooks.trigger_agent deferred for {source}/{caller_id}"),
        )),
    }
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

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
