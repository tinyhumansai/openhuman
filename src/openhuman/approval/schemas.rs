//! Controller schemas + handlers for the `approval` namespace.
//!
//! Wires `approval_list_pending` and `approval_decide` into the
//! global registry consumed by `src/core/all.rs`.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

use super::rpc as approval_rpc;
use super::types::ApprovalDecision;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("list_pending"), schemas("decide")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list_pending"),
            handler: handle_list_pending,
        },
        RegisteredController {
            schema: schemas("decide"),
            handler: handle_decide,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list_pending" => ControllerSchema {
            namespace: "approval",
            function: "list_pending",
            description:
                "List pending approval requests awaiting a user decision in the current session.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "pending",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("PendingApproval"))),
                comment: "Pending approval rows.",
                required: true,
            }],
        },
        "decide" => ControllerSchema {
            namespace: "approval",
            function: "decide",
            description:
                "Apply a decision to a pending approval (approve_once / approve_always_for_tool / deny).",
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
                        "One of \"approve_once\", \"approve_always_for_tool\", or \"deny\".",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "decided",
                ty: TypeSchema::Ref("PendingApproval"),
                comment: "The pending row after the decision was applied.",
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

fn handle_list_pending(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let outcome = approval_rpc::approval_list_pending()
            .await
            .map_err(|e| e.to_string())?;
        to_json(outcome)
    })
}

fn handle_decide(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let request_id = read_required_string(&params, "request_id")?;
        let decision_str = read_required_string(&params, "decision")?;
        let decision = ApprovalDecision::from_str(decision_str.trim()).ok_or_else(|| {
            format!(
                "invalid 'decision': expected approve_once|approve_always_for_tool|deny, got '{decision_str}'"
            )
        })?;
        let outcome = approval_rpc::approval_decide(request_id.trim(), decision)
            .await
            .map_err(|e| e.to_string())?;
        to_json(outcome)
    })
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

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schemas_list_pending_has_no_inputs() {
        let s = schemas("list_pending");
        assert_eq!(s.namespace, "approval");
        assert_eq!(s.function, "list_pending");
        assert!(s.inputs.is_empty());
    }

    #[test]
    fn schemas_decide_requires_request_id_and_decision() {
        let s = schemas("decide");
        let names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
        assert!(names.contains(&"request_id"));
        assert!(names.contains(&"decision"));
        assert!(s.inputs.iter().all(|f| f.required));
    }

    #[test]
    fn schemas_unknown_returns_placeholder() {
        let s = schemas("nope");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.outputs[0].name, "error");
    }

    #[test]
    fn all_registered_controllers_has_handler_per_schema() {
        let controllers = all_registered_controllers();
        assert_eq!(controllers.len(), 2);
        let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
        assert_eq!(names, vec!["list_pending", "decide"]);
    }

    #[test]
    fn read_required_string_returns_value_for_present_key() {
        let mut params = Map::new();
        params.insert("request_id".into(), json!("abc"));
        let got = read_required_string(&params, "request_id").unwrap();
        assert_eq!(got, "abc");
    }

    #[test]
    fn read_required_string_rejects_wrong_type() {
        let mut params = Map::new();
        params.insert("decision".into(), json!(42));
        let err = read_required_string(&params, "decision").unwrap_err();
        assert!(err.contains("expected string"));
    }

    #[test]
    fn read_required_string_missing_key_errors() {
        let err = read_required_string(&Map::new(), "request_id").unwrap_err();
        assert!(err.contains("missing required"));
    }
}
