use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::types::{
    ActionRequestDecisionRpcParams, AlertActionRpcParams, GetActionRequestRpcParams,
    ListActionRequestsRpcParams, ListAlertsRpcParams, TraceAlertRpcParams,
};

pub fn all_internal_controllers() -> Vec<RegisteredController> {
    let mut controllers = vec![
        RegisteredController {
            schema: youpet_schemas("list_alerts"),
            handler: handle_list_alerts,
        },
        RegisteredController {
            schema: youpet_schemas("ack_alert"),
            handler: handle_ack_alert,
        },
        RegisteredController {
            schema: youpet_schemas("resolve_alert"),
            handler: handle_resolve_alert,
        },
        RegisteredController {
            schema: youpet_schemas("trace_alert"),
            handler: handle_trace_alert,
        },
        RegisteredController {
            schema: youpet_schemas("list_action_requests"),
            handler: handle_list_action_requests,
        },
        RegisteredController {
            schema: youpet_schemas("get_action_request"),
            handler: handle_get_action_request,
        },
        RegisteredController {
            schema: youpet_schemas("approve_action_request"),
            handler: handle_approve_action_request,
        },
        RegisteredController {
            schema: youpet_schemas("reject_action_request"),
            handler: handle_reject_action_request,
        },
    ];
    controllers.extend(crate::openhuman::youpet::registry::all_internal_controllers());
    controllers
}

pub fn youpet_schemas(function: &str) -> ControllerSchema {
    match function {
        "list_alerts" => ControllerSchema {
            namespace: "youpet",
            function: "list_alerts",
            description: "List YouPet Core workbench alerts through the Rust core.",
            inputs: vec![
                optional_string(
                    "status",
                    "Optional alert status filter. Omitted -> open alerts only; null or empty string -> all states.",
                ),
                optional_string("severity", "Optional alert severity filter."),
            ],
            outputs: vec![json_output("alerts", "Core workbench alert rows.")],
        },
        "ack_alert" => ControllerSchema {
            namespace: "youpet",
            function: "ack_alert",
            description: "Acknowledge a YouPet Core workbench alert.",
            inputs: alert_action_inputs("note"),
            outputs: vec![json_output("alert", "Updated Core alert.")],
        },
        "resolve_alert" => ControllerSchema {
            namespace: "youpet",
            function: "resolve_alert",
            description: "Resolve a YouPet Core workbench alert.",
            inputs: alert_action_inputs("resolution"),
            outputs: vec![json_output("alert", "Updated Core alert.")],
        },
        "trace_alert" => ControllerSchema {
            namespace: "youpet",
            function: "trace_alert",
            description: "Load a read-only YouPet Core workbench alert trace.",
            inputs: vec![required_string("alertId", "Alert id.")],
            outputs: vec![json_output("trace", "Core workbench alert trace.")],
        },
        "list_action_requests" => ControllerSchema {
            namespace: "youpet",
            function: "list_action_requests",
            description: "List YouPet Core ActionRequests for the operator inbox.",
            inputs: vec![
                optional_string(
                    "tenantId",
                    "Tenant UUID. Omitted -> youpet.tenant_id / YOUPET_TENANT_ID config.",
                ),
                optional_string("approvalState", "Optional approval_state filter."),
                optional_string("executionState", "Optional execution_state filter."),
                optional_i64("limit", "Optional list limit (1-200)."),
            ],
            outputs: vec![json_output(
                "items",
                "Core ActionRequest lifecycle envelopes.",
            )],
        },
        "get_action_request" => ControllerSchema {
            namespace: "youpet",
            function: "get_action_request",
            description: "Get one YouPet Core ActionRequest by id.",
            inputs: vec![required_string(
                "actionRequestId",
                "ActionRequest id (UUID).",
            )],
            outputs: vec![json_output(
                "item",
                "Core ActionRequest lifecycle envelope.",
            )],
        },
        "approve_action_request" => ControllerSchema {
            namespace: "youpet",
            function: "approve_action_request",
            description: "Approve a pending Core ActionRequest as the configured operator user.",
            inputs: action_request_decision_inputs(),
            outputs: vec![json_output("item", "Updated Core ActionRequest envelope.")],
        },
        "reject_action_request" => ControllerSchema {
            namespace: "youpet",
            function: "reject_action_request",
            description: "Reject a pending Core ActionRequest as the configured operator user.",
            inputs: action_request_decision_inputs(),
            outputs: vec![json_output("item", "Updated Core ActionRequest envelope.")],
        },
        _ => ControllerSchema {
            namespace: "youpet",
            function: "unknown",
            description: "Unknown YouPet controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_list_alerts(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<ListAlertsRpcParams>(params)?;
        to_json(crate::openhuman::youpet::list_alerts(&config, payload).await?)
    })
}

fn handle_ack_alert(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<AlertActionRpcParams>(params)?;
        to_json(crate::openhuman::youpet::ack_alert(&config, payload).await?)
    })
}

fn handle_resolve_alert(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<AlertActionRpcParams>(params)?;
        to_json(crate::openhuman::youpet::resolve_alert(&config, payload).await?)
    })
}

fn handle_trace_alert(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<TraceAlertRpcParams>(params)?;
        to_json(crate::openhuman::youpet::get_alert_trace(&config, payload).await?)
    })
}

fn handle_list_action_requests(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<ListActionRequestsRpcParams>(params)?;
        to_json(crate::openhuman::youpet::list_action_requests(&config, payload).await?)
    })
}

fn handle_get_action_request(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<GetActionRequestRpcParams>(params)?;
        to_json(crate::openhuman::youpet::get_action_request(&config, payload).await?)
    })
}

fn handle_approve_action_request(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<ActionRequestDecisionRpcParams>(params)?;
        to_json(crate::openhuman::youpet::approve_action_request(&config, payload).await?)
    })
}

fn handle_reject_action_request(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<ActionRequestDecisionRpcParams>(params)?;
        to_json(crate::openhuman::youpet::reject_action_request(&config, payload).await?)
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn alert_action_inputs(optional_text_name: &'static str) -> Vec<FieldSchema> {
    vec![
        required_string("alertId", "Alert id."),
        optional_string(optional_text_name, "Optional action text."),
        optional_string(
            "idempotencyKey",
            "Optional caller-supplied idempotency key. Omitted or blank -> fresh UUID per attempt, so retries are not replay/dedupe safe; supply a stable key for retry-safe semantics.",
        ),
    ]
}

fn action_request_decision_inputs() -> Vec<FieldSchema> {
    vec![
        required_string("actionRequestId", "ActionRequest id (UUID)."),
        required_string("reason", "Non-empty operator decision reason."),
        required_i64(
            "expectedRowVersion",
            "Current Core row_version for optimistic concurrency.",
        ),
        required_string(
            "idempotencyKey",
            "Stable per-intent Idempotency-Key. Required so retries cannot mint a new UUID.",
        ),
    ]
}

fn optional_i64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
        comment,
        required: false,
    }
}

fn required_i64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::I64,
        comment,
        required: true,
    }
}

#[cfg(test)]
#[path = "youpet_schemas_tests.rs"]
mod tests;
