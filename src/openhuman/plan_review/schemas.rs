//! JSON-RPC surface for the plan-review gate: `openhuman.plan_review_decide`
//! resolves a parked interactive turn (approve / reject / revise-with-feedback).

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::gate;
use super::types::PlanReviewResolution;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("decide")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("decide"),
        handler: handle_decide,
    }]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "decide" => ControllerSchema {
            namespace: "plan_review",
            function: "decide",
            description: "Resolve a parked plan review: approve (the turn resumes and executes), \
                          reject (the turn resumes and stops), or revise (the turn resumes, \
                          re-plans from `feedback`, and re-parks).",
            inputs: vec![
                FieldSchema {
                    name: "request_id",
                    ty: TypeSchema::String,
                    comment: "Plan-review request id from the `plan_review_request` event.",
                    required: true,
                },
                FieldSchema {
                    name: "decision",
                    ty: TypeSchema::String,
                    comment: "One of `approve` | `reject` | `revise`.",
                    required: true,
                },
                FieldSchema {
                    name: "feedback",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Free-text revision request — required when `decision` is `revise`.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "resolved",
                ty: TypeSchema::Bool,
                comment:
                    "true if a parked turn was woken; false if the request was unknown/expired.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "plan_review",
            function: "unknown",
            description: "Unknown plan_review controller function.",
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

#[derive(Debug, Deserialize)]
struct DecideParams {
    request_id: String,
    decision: String,
    #[serde(default)]
    feedback: Option<String>,
}

fn handle_decide(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<DecideParams>(params)?;
        let resolution = match p.decision.trim().to_ascii_lowercase().as_str() {
            "approve" => PlanReviewResolution::Approve,
            "reject" => PlanReviewResolution::Reject,
            "revise" => {
                let feedback = p.feedback.unwrap_or_default();
                if feedback.trim().is_empty() {
                    return Err("revise requires non-empty `feedback`".to_string());
                }
                PlanReviewResolution::Revise { feedback }
            }
            other => {
                return Err(format!(
                    "invalid decision '{other}' (expected approve|reject|revise)"
                ))
            }
        };
        tracing::debug!(
            request_id = %p.request_id,
            decision = resolution.as_str(),
            "[rpc][plan_review] decide entry"
        );
        let resolved = gate::global().decide(&p.request_id, resolution);
        Ok(serde_json::json!({ "resolved": resolved }))
    })
}

fn parse<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_lists_match_lengths() {
        assert_eq!(
            all_controller_schemas().len(),
            all_registered_controllers().len()
        );
    }

    #[test]
    fn schema_uses_plan_review_namespace() {
        assert_eq!(schemas("decide").namespace, "plan_review");
    }
}
