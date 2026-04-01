use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PurchasePlanParams {
    plan: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TopUpParams {
    amount_usd: f64,
    #[serde(default)]
    gateway: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoinbaseChargeParams {
    plan: String,
    #[serde(default)]
    interval: Option<String>,
}

pub fn all_billing_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        billing_schemas("billing_get_current_plan"),
        billing_schemas("billing_purchase_plan"),
        billing_schemas("billing_create_portal_session"),
        billing_schemas("billing_top_up"),
        billing_schemas("billing_create_coinbase_charge"),
    ]
}

pub fn all_billing_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: billing_schemas("billing_get_current_plan"),
            handler: handle_billing_get_current_plan,
        },
        RegisteredController {
            schema: billing_schemas("billing_purchase_plan"),
            handler: handle_billing_purchase_plan,
        },
        RegisteredController {
            schema: billing_schemas("billing_create_portal_session"),
            handler: handle_billing_create_portal_session,
        },
        RegisteredController {
            schema: billing_schemas("billing_top_up"),
            handler: handle_billing_top_up,
        },
        RegisteredController {
            schema: billing_schemas("billing_create_coinbase_charge"),
            handler: handle_billing_create_coinbase_charge,
        },
    ]
}

pub fn billing_schemas(function: &str) -> ControllerSchema {
    match function {
        "billing_get_current_plan" => ControllerSchema {
            namespace: "billing",
            function: "get_current_plan",
            description: "Fetch current subscription plan and entitlements.",
            inputs: vec![],
            outputs: vec![json_output(
                "plan",
                "Current plan payload from backend /payments/stripe/currentPlan.",
            )],
        },
        "billing_purchase_plan" => ControllerSchema {
            namespace: "billing",
            function: "purchase_plan",
            description: "Create Stripe Checkout session for a plan purchase.",
            inputs: vec![required_string(
                "plan",
                "Plan identifier (backend contract).",
            )],
            outputs: vec![json_output(
                "session",
                "Purchase session payload from /payments/stripe/purchasePlan.",
            )],
        },
        "billing_create_portal_session" => ControllerSchema {
            namespace: "billing",
            function: "create_portal_session",
            description: "Create Stripe customer portal session.",
            inputs: vec![],
            outputs: vec![json_output(
                "session",
                "Portal session payload from /payments/stripe/portal.",
            )],
        },
        "billing_top_up" => ControllerSchema {
            namespace: "billing",
            function: "top_up",
            description: "Initiate credit top-up via Stripe/Coinbase.",
            inputs: vec![
                FieldSchema {
                    name: "amountUsd",
                    ty: TypeSchema::F64,
                    comment: "Top-up amount in USD.",
                    required: true,
                },
                optional_string("gateway", "Payment gateway (stripe|coinbase)."),
            ],
            outputs: vec![json_output(
                "topUp",
                "Top-up initiation payload from /payments/credits/top-up.",
            )],
        },
        "billing_create_coinbase_charge" => ControllerSchema {
            namespace: "billing",
            function: "create_coinbase_charge",
            description: "Create a Coinbase Commerce payment link for crypto / annual billing.",
            inputs: vec![
                required_string("plan", "Plan tier (e.g. pro, enterprise)."),
                optional_string("interval", "Billing interval; defaults to 'annual'."),
            ],
            outputs: vec![json_output(
                "charge",
                "Coinbase charge payload from /payments/coinbase/charge.",
            )],
        },
        _ => ControllerSchema {
            namespace: "billing",
            function: "unknown",
            description: "Unknown billing controller.",
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

fn handle_billing_get_current_plan(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::billing::get_current_plan(&config).await?)
    })
}

fn handle_billing_purchase_plan(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<PurchasePlanParams>(params)?;
        to_json(crate::openhuman::billing::purchase_plan(&config, payload.plan.trim()).await?)
    })
}

fn handle_billing_create_portal_session(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::billing::create_portal_session(&config).await?)
    })
}

fn handle_billing_top_up(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<TopUpParams>(params)?;
        to_json(
            crate::openhuman::billing::top_up_credits(&config, payload.amount_usd, payload.gateway)
                .await?,
        )
    })
}

fn handle_billing_create_coinbase_charge(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<CoinbaseChargeParams>(params)?;
        to_json(
            crate::openhuman::billing::create_coinbase_charge(
                &config,
                payload.plan.trim(),
                payload.interval,
            )
            .await?,
        )
    })
}

fn to_json(outcome: RpcOutcome<Value>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
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

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_names_are_stable() {
        let s = billing_schemas("billing_top_up");
        assert_eq!(s.namespace, "billing");
        assert_eq!(s.function, "top_up");
    }

    #[test]
    fn controller_lists_match_lengths() {
        assert_eq!(
            all_billing_controller_schemas().len(),
            all_billing_registered_controllers().len()
        );
    }
}
