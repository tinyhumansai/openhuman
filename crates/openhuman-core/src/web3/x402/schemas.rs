//! Controller schemas + handlers for the `x402` namespace.
//!
//! Wires `x402_get_summary`, `x402_list_payments`, and `x402_update_budget`
//! into the global registry consumed by `crates/openhuman-core/src/core/all.rs`.

use crate::core::runtime::DispatchCtx;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

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

fn handle_get_summary(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx
            .load_config()
            .await
            .map_err(|error| format!("x402 get_summary: {error}"))?;
        let summary = crate::modules::x402_service::summary(&ctx, &config)
            .await
            .map_err(|error| format!("x402 get_summary: {error}"))?;
        Ok(summary_value(summary))
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListPaymentsParams {
    #[serde(default)]
    limit: Option<u64>,
}

fn handle_list_payments(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: ListPaymentsParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("x402 list_payments params: {e}"))?;

        let config = ctx
            .load_config()
            .await
            .map_err(|error| format!("x402 list_payments: {error}"))?;
        let limit = p.limit.unwrap_or(50).min(500);
        let response = crate::modules::x402_service::list_payments(&ctx, &config, limit)
            .await
            .map_err(|error| format!("x402 list_payments: {error}"))?;
        Ok(payments_value(response.payments))
    })
}

fn payments_value(payments: Vec<tinywallet_bus::wire::X402PaymentRecord>) -> Value {
    json!({ "payments": payments })
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

fn handle_update_budget(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: UpdateBudgetParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("x402 update_budget params: {e}"))?;

        let config = ctx
            .load_config()
            .await
            .map_err(|error| format!("x402 update_budget: {error}"))?;
        let summary = crate::modules::x402_service::update_budget(
            &ctx,
            &config,
            p.per_request_max,
            p.daily_max,
            p.monthly_max,
        )
        .await
        .map_err(|error| format!("x402 update_budget: {error}"))?;

        Ok(budget_value(
            summary.per_request_max_atomic,
            summary.daily_max_atomic,
            summary.monthly_max_atomic,
        ))
    })
}

fn summary_value(summary: tinywallet_bus::wire::X402DetailedSpendingSummary) -> Value {
    json!({
        "summary": {
            "sessionTotalAtomic": summary.session_total_atomic,
            "dailyTotalAtomic": summary.daily_total_atomic,
            "monthlyTotalAtomic": summary.monthly_total_atomic,
            "sessionCount": summary.session_count,
            "dailyCount": summary.daily_count,
            "monthlyCount": summary.monthly_count,
        },
        "budget": {
            "perRequestMaxAtomic": summary.per_request_max_atomic,
            "dailyMaxAtomic": summary.daily_max_atomic,
            "monthlyMaxAtomic": summary.monthly_max_atomic,
        },
    })
}

fn budget_value(
    per_request_max_atomic: u64,
    daily_max_atomic: u64,
    monthly_max_atomic: u64,
) -> Value {
    json!({
        "budget": {
            "perRequestMaxAtomic": per_request_max_atomic,
            "dailyMaxAtomic": daily_max_atomic,
            "monthlyMaxAtomic": monthly_max_atomic,
        }
    })
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
