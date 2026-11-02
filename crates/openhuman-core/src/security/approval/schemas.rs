//! Controller schemas + handlers for the `approval` namespace.
//!
//! Wires `approval_list_pending` and `approval_decide` into the
//! global registry consumed by `crates/openhuman-core/src/core/all.rs`.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::{
    BoundRegisteredController, ControllerEntry, ControllerFuture, RegisteredController,
};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::rpc as approval_rpc;
use super::types::ApprovalDecision;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list_pending"),
        schemas("list_recent_decisions"),
        schemas("list_tool_trust"),
        schemas("revoke_tool_trust"),
        schemas("decide"),
        schemas("get_gate_state"),
        schemas("preauthorize_flow"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list_pending"),
            handler: handle_list_pending,
        },
        RegisteredController {
            schema: schemas("list_recent_decisions"),
            handler: handle_list_recent_decisions,
        },
        RegisteredController {
            schema: schemas("list_tool_trust"),
            handler: handle_list_tool_trust,
        },
        RegisteredController {
            schema: schemas("revoke_tool_trust"),
            handler: handle_revoke_tool_trust,
        },
        RegisteredController {
            schema: schemas("decide"),
            handler: handle_decide,
        },
        RegisteredController {
            schema: schemas("get_gate_state"),
            handler: handle_get_gate_state,
        },
        RegisteredController {
            schema: schemas("preauthorize_flow"),
            handler: handle_preauthorize_flow,
        },
    ]
}

pub(crate) fn all_approval_controller_entries() -> Vec<ControllerEntry> {
    vec![
        ControllerEntry::bound(list_pending_controller()),
        ControllerEntry::bound(list_recent_decisions_controller()),
        ControllerEntry::bound(list_tool_trust_controller()),
        ControllerEntry::bound(revoke_tool_trust_controller()),
        ControllerEntry::bound(decide_controller()),
        ControllerEntry::bound(get_gate_state_controller()),
        ControllerEntry::bound(preauthorize_flow_controller()),
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list_pending" => ControllerSchema {
            namespace: "approval",
            function: "list_pending",
            description:
                "List pending approval requests awaiting a user decision in the current session.",
            inputs: vec![FieldSchema {
                name: "agent_id",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Only the requests this embedded agent parked. Omit to list every request.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "pending",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("PendingApproval"))),
                comment: "Pending approval rows.",
                required: true,
            }],
        },
        "list_recent_decisions" => ControllerSchema {
            namespace: "approval",
            function: "list_recent_decisions",
            description: "List recently decided approval rows for durable audit and diagnostics.",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Maximum decided rows to return (1-500, default 50).",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "decisions",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("ApprovalAuditEntry"))),
                comment: "Recently decided approval rows.",
                required: true,
            }],
        },
        "list_tool_trust" => ControllerSchema {
            namespace: "approval",
            function: "list_tool_trust",
            description: "List live durable whole-tool approval grants in the current tenant scope.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "grants",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("ToolTrustEntry"))),
                comment: "Live whole-tool grants with opaque revocation IDs.",
                required: true,
            }],
        },
        "revoke_tool_trust" => ControllerSchema {
            namespace: "approval",
            function: "revoke_tool_trust",
            description: "Revoke a live whole-tool approval grant by its listed opaque ID.",
            inputs: vec![FieldSchema {
                name: "request_id",
                ty: TypeSchema::String,
                comment: "Opaque ID from approval_list_tool_trust.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "revoked",
                ty: TypeSchema::Bool,
                comment: "Whether the grant was still present and has now been revoked.",
                required: true,
            }],
        },
        "get_gate_state" => ControllerSchema {
            namespace: "approval",
            function: "get_gate_state",
            description:
                "Read the host-aware approval-gate boot state so the UI can render the right banner on first paint.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "state",
                ty: TypeSchema::Ref("ApprovalGateBootState"),
                comment: "Snapshot of the boot decision: installed / disabled-by-env / override-ignored / host tag.",
                required: true,
            }],
        },
        "decide" => ControllerSchema {
            namespace: "approval",
            function: "decide",
            description:
                "Apply a decision to a pending approval (approve_once / approve_always_for_tool / approve_always_for_flow / deny).",
            inputs: vec![
                FieldSchema {
                    name: "request_id",
                    ty: TypeSchema::String,
                    comment: "Identifier of the pending approval to decide.",
                    required: true,
                },
                FieldSchema {
                    name: "decision",
                    ty: TypeSchema::String,
                    comment:
                        "One of \"approve_once\", \"approve_always_for_tool\", \
                         \"approve_always_for_flow\" (only meaningful when the row's \
                         source_context names a flow), or \"deny\".",
                    required: true,
                },
                FieldSchema {
                    name: "agent_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Decide as this embedded agent; refused when the request belongs to \
                              another agent.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "decided",
                ty: TypeSchema::Ref("PendingApproval"),
                comment: "The pending row after the decision was applied.",
                required: true,
            }],
        },
        "preauthorize_flow" => ControllerSchema {
            namespace: "approval",
            function: "preauthorize_flow",
            description:
                "Batch-grant flow-scoped tool trust at save+enable time (consolidated \
                 pre-authorization card). Idempotent; already-trusted tools are reported, \
                 not re-granted. Succeeds with gate_installed=false when the approval gate \
                 is disabled.",
            inputs: vec![
                FieldSchema {
                    name: "flow_id",
                    ty: TypeSchema::String,
                    comment: "Flow to grant trust for.",
                    required: true,
                },
                FieldSchema {
                    name: "tool_names",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment:
                        "Gate trust keys to grant (e.g. \"flows_http_request\", a Composio \
                         slug, or a native tool name). Blanks and duplicates are skipped.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Ref("FlowPreauthorizationResult"),
                comment: "Which grants were newly written vs already present.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "approval",
            function: "unknown",
            description: "Unknown approval function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Schema not defined for the requested function.",
                required: true,
            }],
        },
    }
}

fn handle_list_pending(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    list_pending_controller().invoke(ctx, params)
}

fn list_pending_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        schemas("list_pending"),
        parse_list_pending,
        run_list_pending,
    )
}

fn run_list_pending(ctx: DispatchCtx, agent_id: Option<String>) -> ControllerFuture {
    Box::pin(async move {
        let outcome = approval_rpc::approval_list_pending(&ctx, agent_id.as_deref())
            .await
            .map_err(|e| e.to_string())?;
        to_json(outcome)
    })
}

fn handle_list_recent_decisions(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    list_recent_decisions_controller().invoke(_ctx, params)
}

fn handle_list_tool_trust(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    list_tool_trust_controller().invoke(ctx, params)
}

fn list_tool_trust_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        schemas("list_tool_trust"),
        parse_list_tool_trust,
        run_list_tool_trust,
    )
}

fn run_list_tool_trust(ctx: DispatchCtx, _parsed: ()) -> ControllerFuture {
    Box::pin(async move {
        let outcome = approval_rpc::approval_list_tool_trust(&ctx)
            .await
            .map_err(|error| error.to_string())?;
        to_json(outcome)
    })
}

fn handle_revoke_tool_trust(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    revoke_tool_trust_controller().invoke(ctx, params)
}

fn revoke_tool_trust_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        schemas("revoke_tool_trust"),
        parse_revoke_tool_trust,
        run_revoke_tool_trust,
    )
}

fn run_revoke_tool_trust(ctx: DispatchCtx, request_id: String) -> ControllerFuture {
    Box::pin(async move {
        let outcome = approval_rpc::approval_revoke_tool_trust(&ctx, &request_id)
            .await
            .map_err(|error| error.to_string())?;
        to_json(outcome)
    })
}

fn list_recent_decisions_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        schemas("list_recent_decisions"),
        parse_list_recent_decisions,
        run_list_recent_decisions,
    )
}

fn run_list_recent_decisions(_ctx: DispatchCtx, limit: Option<usize>) -> ControllerFuture {
    Box::pin(async move {
        let outcome = approval_rpc::approval_list_recent_decisions(limit)
            .await
            .map_err(|e| e.to_string())?;
        to_json(outcome)
    })
}

fn handle_get_gate_state(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    get_gate_state_controller().invoke(_ctx, params)
}

fn get_gate_state_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        schemas("get_gate_state"),
        parse_get_gate_state,
        run_get_gate_state,
    )
}

fn run_get_gate_state(_ctx: DispatchCtx, _parsed: ()) -> ControllerFuture {
    Box::pin(async move {
        let outcome = approval_rpc::approval_get_gate_state()
            .await
            .map_err(|e| e.to_string())?;
        to_json(outcome)
    })
}

fn handle_decide(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    decide_controller().invoke(ctx, params)
}

fn decide_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(schemas("decide"), parse_decide, run_decide)
}

fn run_decide(
    ctx: DispatchCtx,
    (request_id, decision, agent_id): (String, ApprovalDecision, Option<String>),
) -> ControllerFuture {
    Box::pin(async move {
        let outcome =
            approval_rpc::approval_decide(&ctx, request_id.trim(), decision, agent_id.as_deref())
                .await
                .map_err(|e| e.to_string())?;
        to_json(outcome)
    })
}

fn handle_preauthorize_flow(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    preauthorize_flow_controller().invoke(_ctx, params)
}

fn preauthorize_flow_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        schemas("preauthorize_flow"),
        parse_preauthorize_flow,
        run_preauthorize_flow,
    )
}

fn run_preauthorize_flow(
    _ctx: DispatchCtx,
    (flow_id, tool_names): (String, Vec<String>),
) -> ControllerFuture {
    Box::pin(async move {
        let outcome = approval_rpc::approval_preauthorize_flow(flow_id.trim(), tool_names)
            .await
            .map_err(|e| e.to_string())?;
        to_json(outcome)
    })
}

fn parse_list_pending(params: Map<String, Value>) -> Result<Option<String>, String> {
    read_optional_string(&params, "agent_id")
}

fn parse_list_recent_decisions(params: Map<String, Value>) -> Result<Option<usize>, String> {
    Ok(read_optional_u64(&params, "limit")?.map(|value| value as usize))
}

fn parse_list_tool_trust(_params: Map<String, Value>) -> Result<(), String> {
    Ok(())
}

fn parse_revoke_tool_trust(params: Map<String, Value>) -> Result<String, String> {
    read_required_string(&params, "request_id")
}

fn parse_get_gate_state(_params: Map<String, Value>) -> Result<(), String> {
    Ok(())
}

fn parse_decide(
    params: Map<String, Value>,
) -> Result<(String, ApprovalDecision, Option<String>), String> {
    let request_id = read_required_string(&params, "request_id")?;
    let decision_str = read_required_string(&params, "decision")?;
    let decision = ApprovalDecision::from_str(decision_str.trim()).ok_or_else(|| {
        format!(
            "invalid 'decision': expected \
             approve_once|approve_always_for_tool|approve_always_for_flow|deny, got '{decision_str}'"
        )
    })?;
    let agent_id = read_optional_string(&params, "agent_id")?;
    Ok((request_id, decision, agent_id))
}

fn parse_preauthorize_flow(params: Map<String, Value>) -> Result<(String, Vec<String>), String> {
    let flow_id = read_required_string(&params, "flow_id")?;
    let tool_names = match params.get("tool_names") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(s.clone()),
                other => Err(format!(
                    "invalid 'tool_names' entry: expected string, got {}",
                    type_name(other)
                )),
            })
            .collect::<Result<Vec<String>, String>>()?,
        Some(other) => {
            return Err(format!(
                "invalid 'tool_names': expected array of strings, got {}",
                type_name(other)
            ))
        }
        None => return Err("missing required param 'tool_names'".to_string()),
    };
    Ok((flow_id, tool_names))
}

fn read_optional_u64(params: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match params.get(key) {
        Some(Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("invalid '{key}': expected unsigned integer")),
        Some(Value::Null) | None => Ok(None),
        Some(other) => Err(format!(
            "invalid '{key}': expected unsigned integer, got {}",
            type_name(other)
        )),
    }
}

fn read_optional_string(params: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match params.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
        Some(Value::String(_)) | Some(Value::Null) | None => Ok(None),
        Some(other) => Err(format!(
            "invalid '{key}': expected string, got {}",
            type_name(other)
        )),
    }
}

fn read_required_string(params: &Map<String, Value>, key: &str) -> Result<String, String> {
    match params.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(format!(
            "invalid '{key}': expected string, got {}",
            type_name(other)
        )),
        None => Err(format!("missing required param '{key}'")),
    }
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
pub(crate) fn approval_parameter_contracts() -> Vec<crate::core::all::ControllerParamContract> {
    tests::parameter_contracts()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
