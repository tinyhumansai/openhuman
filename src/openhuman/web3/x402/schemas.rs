//! Controller schemas + handlers for the `x402` namespace.
//!
//! Wires `x402_get_summary`, `x402_list_payments`, and `x402_update_budget`
//! into the global registry consumed by `src/core/all.rs`.

use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::store;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("get_summary"),
        schemas("list_payments"),
        schemas("update_budget"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("get_summary"),
            handler: handle_get_summary,
        },
        RegisteredController {
            schema: schemas("list_payments"),
            handler: handle_list_payments,
        },
        RegisteredController {
            schema: schemas("update_budget"),
            handler: handle_update_budget,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "get_summary" => ControllerSchema {
            namespace: "x402",
            function: "get_summary",
            description: "Get x402 payment spending summary (session, daily, monthly totals and budget limits).",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "summary",
                    ty: TypeSchema::Ref("SpendingSummary"),
                    comment: "Spending totals for session, day, and month.",
                    required: true,
                },
                FieldSchema {
                    name: "budget",
                    ty: TypeSchema::Ref("SpendingBudget"),
                    comment: "Current budget limits.",
                    required: true,
                },
            ],
        },
        "list_payments" => ControllerSchema {
            namespace: "x402",
            function: "list_payments",
            description: "List recent x402 payment records.",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Max records to return (1-500, default 50).",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "payments",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("PaymentRecord"))),
                comment: "Recent payment records, newest first.",
                required: true,
            }],
        },
        "update_budget" => ControllerSchema {
            namespace: "x402",
            function: "update_budget",
            description: "Update x402 spending budget limits (atomic USDC units: 1 USDC = 1_000_000).",
            inputs: vec![
                FieldSchema {
                    name: "per_request_max",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max atomic USDC per single request.",
                    required: false,
                },
                FieldSchema {
                    name: "daily_max",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max atomic USDC per day.",
                    required: false,
                },
                FieldSchema {
                    name: "monthly_max",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max atomic USDC per month.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "budget",
                ty: TypeSchema::Ref("SpendingBudget"),
                comment: "Updated budget.",
                required: true,
            }],
        },
        other => panic!("unknown x402 schema function: {other}"),
    }
}

fn handle_get_summary(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let (summary, budget) = store::with_ledger(|l| (l.summary(), l.budget().clone()))
            .map_err(|e| format!("x402 get_summary: {e}"))?;

        Ok(json!({
            "summary": summary,
            "budget": budget,
        }))
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListPaymentsParams {
    #[serde(default)]
    limit: Option<u64>,
}

fn handle_list_payments(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: ListPaymentsParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("x402 list_payments params: {e}"))?;

        let limit = p.limit.unwrap_or(50).min(500) as usize;
        let payments = store::with_ledger(|l| l.recent_payments(limit))
            .map_err(|e| format!("x402 list_payments: {e}"))?;

        Ok(json!({ "payments": payments }))
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateBudgetParams {
    #[serde(default)]
    per_request_max: Option<u64>,
    #[serde(default)]
    daily_max: Option<u64>,
    #[serde(default)]
    monthly_max: Option<u64>,
}

fn handle_update_budget(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: UpdateBudgetParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("x402 update_budget params: {e}"))?;

        let budget = store::with_ledger_mut(|l| {
            let mut b = l.budget().clone();
            if let Some(v) = p.per_request_max {
                b.per_request_max_atomic = v;
            }
            if let Some(v) = p.daily_max {
                b.daily_max_atomic = v;
            }
            if let Some(v) = p.monthly_max {
                b.monthly_max_atomic = v;
            }
            l.update_budget(b.clone());
            b
        })
        .map_err(|e| format!("x402 update_budget: {e}"))?;

        Ok(json!({ "budget": budget }))
    })
}
