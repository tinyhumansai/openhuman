use reqwest::Method;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::types::{
    ActionRequestDecisionRpcParams, ActionRequestLifecycleEnvelope, ActionRequestListResponse,
    AlertActionRpcParams, CoreWorkbenchAlert, CoreWorkbenchAlertsResponse,
    GetActionRequestRpcParams, ListActionRequestsRpcParams, ListAlertsRpcParams,
    TraceAlertRpcParams, WorkbenchAlertTrace,
};
use super::{config_error, structured_error, YouPetTransport};

pub async fn list_alerts(
    config: &Config,
    params: ListAlertsRpcParams,
) -> Result<RpcOutcome<Vec<CoreWorkbenchAlert>>, String> {
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let mut request = transport.get("/api/v1/workbench/alerts")?;
    if let Some(status) = params.status.as_query_param() {
        request = request.query(&[("status", status)]);
    }
    if let Some(severity) = params.severity {
        request = request.query(&[("severity", severity.as_str())]);
    }
    let response: CoreWorkbenchAlertsResponse = transport.send(request).await?;
    Ok(RpcOutcome::single_log(
        response.items,
        "[youpet] listed Core workbench alerts",
    ))
}

pub async fn ack_alert(
    config: &Config,
    params: AlertActionRpcParams,
) -> Result<RpcOutcome<CoreWorkbenchAlert>, String> {
    let actor_user_id = required_operator_user_id(config)?;
    let mut body = Map::new();
    body.insert("actor_user_id".to_string(), json!(actor_user_id));
    if let Some(note) = params.note {
        body.insert("note".to_string(), json!(note));
    }
    let alert = send_alert_action(
        config,
        &params.alert_id,
        "ack",
        Value::Object(body),
        params.idempotency_key,
    )
    .await?;
    Ok(RpcOutcome::single_log(
        alert,
        "[youpet] acknowledged Core alert",
    ))
}

pub async fn resolve_alert(
    config: &Config,
    params: AlertActionRpcParams,
) -> Result<RpcOutcome<CoreWorkbenchAlert>, String> {
    let actor_user_id = required_operator_user_id(config)?;
    let mut body = Map::new();
    body.insert("actor_user_id".to_string(), json!(actor_user_id));
    if let Some(resolution) = params.resolution {
        body.insert("resolution".to_string(), json!(resolution));
    }
    let alert = send_alert_action(
        config,
        &params.alert_id,
        "resolve",
        Value::Object(body),
        params.idempotency_key,
    )
    .await?;
    Ok(RpcOutcome::single_log(
        alert,
        "[youpet] resolved Core alert",
    ))
}

pub async fn get_alert_trace(
    config: &Config,
    params: TraceAlertRpcParams,
) -> Result<RpcOutcome<WorkbenchAlertTrace>, String> {
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let path = format!(
        "/api/v1/workbench/alerts/{}/trace",
        urlencoding::encode(&params.alert_id)
    );
    let trace: WorkbenchAlertTrace = transport.send(transport.get(&path)?).await?;
    Ok(RpcOutcome::single_log(
        trace,
        "[youpet] loaded Core workbench alert trace",
    ))
}

pub async fn list_action_requests(
    config: &Config,
    params: ListActionRequestsRpcParams,
) -> Result<RpcOutcome<Vec<ActionRequestLifecycleEnvelope>>, String> {
    let tenant_id = resolve_tenant_id(config, params.tenant_id.as_deref())?;
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let mut request = transport
        .get("/api/v1/action-requests")?
        .query(&[("tenant_id", tenant_id.as_str())]);
    if let Some(state) = params
        .approval_state
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request = request.query(&[("approval_state", state)]);
    }
    if let Some(state) = params
        .execution_state
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request = request.query(&[("execution_state", state)]);
    }
    if let Some(limit) = params.limit {
        request = request.query(&[("limit", limit.to_string())]);
    }
    let response: ActionRequestListResponse = transport.send(request).await?;
    Ok(RpcOutcome::single_log(
        response.items,
        "[youpet] listed Core action requests",
    ))
}

pub async fn get_action_request(
    config: &Config,
    params: GetActionRequestRpcParams,
) -> Result<RpcOutcome<ActionRequestLifecycleEnvelope>, String> {
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let path = format!(
        "/api/v1/action-requests/{}",
        urlencoding::encode(&params.action_request_id)
    );
    let item: ActionRequestLifecycleEnvelope = transport.send(transport.get(&path)?).await?;
    Ok(RpcOutcome::single_log(
        item,
        "[youpet] loaded Core action request",
    ))
}

pub async fn approve_action_request(
    config: &Config,
    params: ActionRequestDecisionRpcParams,
) -> Result<RpcOutcome<ActionRequestLifecycleEnvelope>, String> {
    let item = send_action_request_decision(config, &params, "approve").await?;
    Ok(RpcOutcome::single_log(
        item,
        "[youpet] approved Core action request",
    ))
}

pub async fn reject_action_request(
    config: &Config,
    params: ActionRequestDecisionRpcParams,
) -> Result<RpcOutcome<ActionRequestLifecycleEnvelope>, String> {
    let item = send_action_request_decision(config, &params, "reject").await?;
    Ok(RpcOutcome::single_log(
        item,
        "[youpet] rejected Core action request",
    ))
}

async fn send_action_request_decision(
    config: &Config,
    params: &ActionRequestDecisionRpcParams,
    action: &str,
) -> Result<ActionRequestLifecycleEnvelope, String> {
    let operator_user_id = required_operator_user_id(config)?;
    let reason = params.reason.trim();
    if reason.is_empty() {
        return Err(structured_error(
            "reason is required for ActionRequest decisions",
            "YouPetRequestInvalid",
            json!({ "field": "reason" }),
            true,
        ));
    }
    if params.expected_row_version < 1 {
        return Err(structured_error(
            "expected_row_version must be >= 1",
            "YouPetRequestInvalid",
            json!({ "field": "expectedRowVersion" }),
            true,
        ));
    }
    let key = params.idempotency_key.trim();
    if key.is_empty() {
        return Err(structured_error(
            "idempotencyKey is required for ActionRequest decisions",
            "YouPetRequestInvalid",
            json!({ "field": "idempotencyKey" }),
            true,
        ));
    }
    // Exact body keys only — never spoof approver_class or decided_at; Core owns those.
    let body = json!({
        "decided_by": {
            "type": "user",
            "id": operator_user_id,
        },
        "reason": reason,
        "expected_row_version": params.expected_row_version,
    });
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let path = format!(
        "/api/v1/action-requests/{}/{}",
        urlencoding::encode(&params.action_request_id),
        action
    );
    let request = transport
        .request(Method::POST, &path)?
        .header("Content-Type", "application/json")
        .header("Idempotency-Key", key)
        .json(&body);
    transport.send(request).await
}

fn resolve_tenant_id(config: &Config, override_id: Option<&str>) -> Result<String, String> {
    if let Some(tenant) = override_id.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(tenant.to_string());
    }
    config
        .youpet
        .tenant_id()
        .map(str::to_string)
        .ok_or_else(|| {
            config_error(
                "youpet.tenant_id is required for ActionRequest list (or pass tenantId)",
                "tenant_id",
            )
        })
}

async fn send_alert_action(
    config: &Config,
    alert_id: &str,
    action: &str,
    body: Value,
    idempotency_key: Option<String>,
) -> Result<CoreWorkbenchAlert, String> {
    let transport = YouPetTransport::new(config, config.youpet.workbench_actor_id());
    let key = idempotency_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let path = format!(
        "/api/v1/alerts/{}/{}",
        urlencoding::encode(alert_id),
        action
    );
    let request = transport
        .request(Method::POST, &path)?
        .header("Content-Type", "application/json")
        .header("Idempotency-Key", key)
        .json(&body);
    transport.send(request).await
}

fn required_operator_user_id(config: &Config) -> Result<&str, String> {
    config.youpet.operator_user_id().ok_or_else(|| {
        config_error(
            "youpet.operator_user_id is required for YouPet Workbench actions",
            "operator_user_id",
        )
    })
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
