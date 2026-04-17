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

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TransactionsParams {
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    offset: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonValueParams {
    payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardParams {
    payment_method_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateCardParams {
    payment_method_id: String,
    payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RedeemCouponParams {
    code: String,
}

pub fn all_billing_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        billing_schemas("billing_get_current_plan"),
        billing_schemas("billing_get_balance"),
        billing_schemas("billing_purchase_plan"),
        billing_schemas("billing_create_portal_session"),
        billing_schemas("billing_top_up"),
        billing_schemas("billing_create_coinbase_charge"),
        billing_schemas("billing_get_transactions"),
        billing_schemas("billing_get_auto_recharge"),
        billing_schemas("billing_update_auto_recharge"),
        billing_schemas("billing_get_cards"),
        billing_schemas("billing_create_setup_intent"),
        billing_schemas("billing_update_card"),
        billing_schemas("billing_delete_card"),
        billing_schemas("billing_redeem_coupon"),
        billing_schemas("billing_get_coupons"),
    ]
}

pub fn all_billing_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: billing_schemas("billing_get_current_plan"),
            handler: handle_billing_get_current_plan,
        },
        RegisteredController {
            schema: billing_schemas("billing_get_balance"),
            handler: handle_billing_get_balance,
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
        RegisteredController {
            schema: billing_schemas("billing_get_transactions"),
            handler: handle_billing_get_transactions,
        },
        RegisteredController {
            schema: billing_schemas("billing_get_auto_recharge"),
            handler: handle_billing_get_auto_recharge,
        },
        RegisteredController {
            schema: billing_schemas("billing_update_auto_recharge"),
            handler: handle_billing_update_auto_recharge,
        },
        RegisteredController {
            schema: billing_schemas("billing_get_cards"),
            handler: handle_billing_get_cards,
        },
        RegisteredController {
            schema: billing_schemas("billing_create_setup_intent"),
            handler: handle_billing_create_setup_intent,
        },
        RegisteredController {
            schema: billing_schemas("billing_update_card"),
            handler: handle_billing_update_card,
        },
        RegisteredController {
            schema: billing_schemas("billing_delete_card"),
            handler: handle_billing_delete_card,
        },
        RegisteredController {
            schema: billing_schemas("billing_redeem_coupon"),
            handler: handle_billing_redeem_coupon,
        },
        RegisteredController {
            schema: billing_schemas("billing_get_coupons"),
            handler: handle_billing_get_coupons,
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
        "billing_get_balance" => ControllerSchema {
            namespace: "billing",
            function: "get_balance",
            description: "Fetch the current user's credit balance.",
            inputs: vec![],
            outputs: vec![json_output(
                "balance",
                "Credit balance payload from backend /payments/credits/balance.",
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
            outputs: vec![
                output_field(
                    "checkoutUrl",
                    TypeSchema::Option(Box::new(TypeSchema::String)),
                    "Stripe Checkout URL returned by /payments/stripe/purchasePlan.",
                ),
                output_field(
                    "sessionId",
                    TypeSchema::String,
                    "Stripe Checkout session ID returned by /payments/stripe/purchasePlan.",
                ),
            ],
        },
        "billing_create_portal_session" => ControllerSchema {
            namespace: "billing",
            function: "create_portal_session",
            description: "Create Stripe customer portal session.",
            inputs: vec![],
            outputs: vec![output_field(
                "portalUrl",
                TypeSchema::String,
                "Stripe customer portal URL returned by /payments/stripe/portal.",
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
            outputs: vec![
                output_field(
                    "url",
                    TypeSchema::String,
                    "Hosted payment URL returned by /payments/credits/top-up.",
                ),
                output_field(
                    "gatewayTransactionId",
                    TypeSchema::String,
                    "Gateway transaction identifier returned by /payments/credits/top-up.",
                ),
                output_field(
                    "amountUsd",
                    TypeSchema::F64,
                    "Top-up amount in USD returned by /payments/credits/top-up.",
                ),
                output_field(
                    "gateway",
                    TypeSchema::String,
                    "Payment gateway used for the top-up.",
                ),
            ],
        },
        "billing_create_coinbase_charge" => ControllerSchema {
            namespace: "billing",
            function: "create_coinbase_charge",
            description: "Create a Coinbase Commerce payment link for crypto / annual billing.",
            inputs: vec![
                required_string("plan", "Plan tier (e.g. pro, enterprise)."),
                optional_string("interval", "Billing interval; defaults to 'annual'."),
            ],
            outputs: vec![
                output_field(
                    "gatewayTransactionId",
                    TypeSchema::String,
                    "Coinbase Commerce charge identifier returned by /payments/coinbase/charge.",
                ),
                output_field(
                    "hostedUrl",
                    TypeSchema::String,
                    "Hosted Coinbase Commerce payment URL returned by /payments/coinbase/charge.",
                ),
                output_field(
                    "status",
                    TypeSchema::String,
                    "Coinbase charge status returned by /payments/coinbase/charge.",
                ),
                output_field(
                    "expiresAt",
                    TypeSchema::String,
                    "Charge expiration timestamp returned by /payments/coinbase/charge.",
                ),
            ],
        },
        "billing_get_transactions" => ControllerSchema {
            namespace: "billing",
            function: "get_transactions",
            description: "Fetch paginated credit transaction history.",
            inputs: vec![
                optional_u64("limit", "Optional page size."),
                optional_u64("offset", "Optional pagination offset."),
            ],
            outputs: vec![json_output(
                "transactions",
                "Credit transaction page payload.",
            )],
        },
        "billing_get_auto_recharge" => ControllerSchema {
            namespace: "billing",
            function: "get_auto_recharge",
            description: "Fetch Stripe auto-recharge settings.",
            inputs: vec![],
            outputs: vec![json_output("settings", "Auto-recharge settings payload.")],
        },
        "billing_update_auto_recharge" => ControllerSchema {
            namespace: "billing",
            function: "update_auto_recharge",
            description: "Update Stripe auto-recharge settings.",
            inputs: vec![FieldSchema {
                name: "payload",
                ty: TypeSchema::Json,
                comment: "PATCH payload for /payments/credits/auto-recharge.",
                required: true,
            }],
            outputs: vec![json_output(
                "settings",
                "Updated auto-recharge settings payload.",
            )],
        },
        "billing_get_cards" => ControllerSchema {
            namespace: "billing",
            function: "get_cards",
            description: "List saved Stripe cards for auto-recharge.",
            inputs: vec![],
            outputs: vec![json_output("cards", "Saved cards payload.")],
        },
        "billing_create_setup_intent" => ControllerSchema {
            namespace: "billing",
            function: "create_setup_intent",
            description: "Create a Stripe SetupIntent for adding a card.",
            inputs: vec![],
            outputs: vec![json_output("result", "Stripe SetupIntent payload.")],
        },
        "billing_update_card" => ControllerSchema {
            namespace: "billing",
            function: "update_card",
            description: "Update a saved card for auto-recharge.",
            inputs: vec![
                required_string("paymentMethodId", "Stripe payment method id."),
                FieldSchema {
                    name: "payload",
                    ty: TypeSchema::Json,
                    comment: "PATCH payload for card update.",
                    required: true,
                },
            ],
            outputs: vec![json_output("cards", "Updated saved cards payload.")],
        },
        "billing_delete_card" => ControllerSchema {
            namespace: "billing",
            function: "delete_card",
            description: "Delete a saved card for auto-recharge.",
            inputs: vec![required_string(
                "paymentMethodId",
                "Stripe payment method id.",
            )],
            outputs: vec![json_output("cards", "Updated saved cards payload.")],
        },
        "billing_redeem_coupon" => ControllerSchema {
            namespace: "billing",
            function: "redeem_coupon",
            description: "Redeem a coupon code to add credits to the account.",
            inputs: vec![required_string("code", "Coupon code to redeem.")],
            outputs: vec![json_output(
                "result",
                "Coupon redemption result from /coupons/redeem.",
            )],
        },
        "billing_get_coupons" => ControllerSchema {
            namespace: "billing",
            function: "get_coupons",
            description: "List coupons redeemed by the current user.",
            inputs: vec![],
            outputs: vec![json_output(
                "coupons",
                "User's redeemed coupons from /coupons/me.",
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

fn handle_billing_get_balance(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::billing::get_balance(&config).await?)
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

fn handle_billing_get_transactions(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = if params.is_empty() {
            TransactionsParams::default()
        } else {
            deserialize_params::<TransactionsParams>(params)?
        };
        to_json(
            crate::openhuman::billing::get_transactions(&config, payload.limit, payload.offset)
                .await?,
        )
    })
}

fn handle_billing_get_auto_recharge(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::billing::get_auto_recharge(&config).await?)
    })
}

fn handle_billing_update_auto_recharge(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<JsonValueParams>(params)?;
        to_json(crate::openhuman::billing::update_auto_recharge(&config, payload.payload).await?)
    })
}

fn handle_billing_get_cards(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::billing::get_cards(&config).await?)
    })
}

fn handle_billing_create_setup_intent(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::billing::create_setup_intent(&config).await?)
    })
}

fn handle_billing_update_card(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<UpdateCardParams>(params)?;
        to_json(
            crate::openhuman::billing::update_card(
                &config,
                payload.payment_method_id.trim(),
                payload.payload,
            )
            .await?,
        )
    })
}

fn handle_billing_delete_card(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<CardParams>(params)?;
        to_json(
            crate::openhuman::billing::delete_card(&config, payload.payment_method_id.trim())
                .await?,
        )
    })
}

fn handle_billing_redeem_coupon(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<RedeemCouponParams>(params)?;
        to_json(crate::openhuman::billing::redeem_coupon(&config, payload.code.trim()).await?)
    })
}

fn handle_billing_get_coupons(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::billing::get_user_coupons(&config).await?)
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

fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
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

fn output_field(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn all_billing_controller_schemas_returns_15() {
        let schemas = all_billing_controller_schemas();
        assert_eq!(schemas.len(), 15);
    }

    #[test]
    fn all_billing_registered_controllers_returns_15() {
        let controllers = all_billing_registered_controllers();
        assert_eq!(controllers.len(), 15);
    }

    #[test]
    fn billing_schemas_get_current_plan() {
        let s = billing_schemas("billing_get_current_plan");
        assert_eq!(s.namespace, "billing");
        assert_eq!(s.function, "get_current_plan");
        assert!(s.inputs.is_empty());
        assert!(!s.outputs.is_empty());
    }

    #[test]
    fn billing_schemas_get_balance() {
        let s = billing_schemas("billing_get_balance");
        assert_eq!(s.function, "get_balance");
        assert!(s.inputs.is_empty());
    }

    #[test]
    fn billing_schemas_purchase_plan() {
        let s = billing_schemas("billing_purchase_plan");
        assert_eq!(s.function, "purchase_plan");
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "plan");
        assert!(s.inputs[0].required);
        assert!(s.outputs.len() >= 2);
    }

    #[test]
    fn billing_schemas_create_portal_session() {
        let s = billing_schemas("billing_create_portal_session");
        assert_eq!(s.function, "create_portal_session");
        assert!(s.inputs.is_empty());
    }

    #[test]
    fn billing_schemas_top_up() {
        let s = billing_schemas("billing_top_up");
        assert_eq!(s.function, "top_up");
        assert_eq!(s.inputs.len(), 2);
        assert_eq!(s.inputs[0].name, "amountUsd");
        assert!(s.inputs[0].required);
        assert!(!s.inputs[1].required); // gateway is optional
    }

    #[test]
    fn billing_schemas_create_coinbase_charge() {
        let s = billing_schemas("billing_create_coinbase_charge");
        assert_eq!(s.function, "create_coinbase_charge");
        assert_eq!(s.inputs.len(), 2);
        assert!(s.outputs.len() >= 4);
    }

    #[test]
    fn billing_schemas_get_transactions() {
        let s = billing_schemas("billing_get_transactions");
        assert_eq!(s.function, "get_transactions");
        assert_eq!(s.inputs.len(), 2);
        assert!(!s.inputs[0].required); // limit is optional
        assert!(!s.inputs[1].required); // offset is optional
    }

    #[test]
    fn billing_schemas_get_auto_recharge() {
        let s = billing_schemas("billing_get_auto_recharge");
        assert_eq!(s.function, "get_auto_recharge");
        assert!(s.inputs.is_empty());
    }

    #[test]
    fn billing_schemas_update_auto_recharge() {
        let s = billing_schemas("billing_update_auto_recharge");
        assert_eq!(s.function, "update_auto_recharge");
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "payload");
    }

    #[test]
    fn billing_schemas_get_cards() {
        let s = billing_schemas("billing_get_cards");
        assert_eq!(s.function, "get_cards");
        assert!(s.inputs.is_empty());
    }

    #[test]
    fn billing_schemas_create_setup_intent() {
        let s = billing_schemas("billing_create_setup_intent");
        assert_eq!(s.function, "create_setup_intent");
        assert!(s.inputs.is_empty());
    }

    #[test]
    fn billing_schemas_update_card() {
        let s = billing_schemas("billing_update_card");
        assert_eq!(s.function, "update_card");
        assert_eq!(s.inputs.len(), 2);
    }

    #[test]
    fn billing_schemas_delete_card() {
        let s = billing_schemas("billing_delete_card");
        assert_eq!(s.function, "delete_card");
        assert_eq!(s.inputs.len(), 1);
    }

    #[test]
    fn billing_schemas_redeem_coupon() {
        let s = billing_schemas("billing_redeem_coupon");
        assert_eq!(s.function, "redeem_coupon");
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "code");
    }

    #[test]
    fn billing_schemas_get_coupons() {
        let s = billing_schemas("billing_get_coupons");
        assert_eq!(s.function, "get_coupons");
        assert!(s.inputs.is_empty());
    }

    #[test]
    fn billing_schemas_unknown_function() {
        let s = billing_schemas("billing_nonexistent");
        assert_eq!(s.function, "unknown");
    }

    // Param deserialization tests

    #[test]
    fn deserialize_purchase_plan_params() {
        let params: Map<String, Value> = serde_json::from_value(json!({"plan": "pro"})).unwrap();
        let result = deserialize_params::<PurchasePlanParams>(params);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().plan, "pro");
    }

    #[test]
    fn deserialize_top_up_params() {
        let params: Map<String, Value> =
            serde_json::from_value(json!({"amountUsd": 10.0})).unwrap();
        let result = deserialize_params::<TopUpParams>(params);
        assert!(result.is_ok());
        let p = result.unwrap();
        assert_eq!(p.amount_usd, 10.0);
        assert!(p.gateway.is_none());
    }

    #[test]
    fn deserialize_top_up_params_with_gateway() {
        let params: Map<String, Value> =
            serde_json::from_value(json!({"amountUsd": 5.0, "gateway": "stripe"})).unwrap();
        let result = deserialize_params::<TopUpParams>(params);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().gateway.as_deref(), Some("stripe"));
    }

    #[test]
    fn deserialize_coinbase_charge_params() {
        let params: Map<String, Value> =
            serde_json::from_value(json!({"plan": "enterprise", "interval": "annual"})).unwrap();
        let result = deserialize_params::<CoinbaseChargeParams>(params);
        assert!(result.is_ok());
        let p = result.unwrap();
        assert_eq!(p.plan, "enterprise");
        assert_eq!(p.interval.as_deref(), Some("annual"));
    }

    #[test]
    fn deserialize_transactions_params_defaults() {
        let params: Map<String, Value> = serde_json::from_value(json!({})).unwrap();
        let result = deserialize_params::<TransactionsParams>(params);
        assert!(result.is_ok());
        let p = result.unwrap();
        assert!(p.limit.is_none());
        assert!(p.offset.is_none());
    }

    #[test]
    fn deserialize_transactions_params_with_values() {
        let params: Map<String, Value> =
            serde_json::from_value(json!({"limit": 10, "offset": 5})).unwrap();
        let result = deserialize_params::<TransactionsParams>(params);
        assert!(result.is_ok());
        let p = result.unwrap();
        assert_eq!(p.limit, Some(10));
        assert_eq!(p.offset, Some(5));
    }

    #[test]
    fn deserialize_card_params() {
        let params: Map<String, Value> =
            serde_json::from_value(json!({"paymentMethodId": "pm_123"})).unwrap();
        let result = deserialize_params::<CardParams>(params);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().payment_method_id, "pm_123");
    }

    #[test]
    fn deserialize_update_card_params() {
        let params: Map<String, Value> = serde_json::from_value(
            json!({"paymentMethodId": "pm_1", "payload": {"default": true}}),
        )
        .unwrap();
        let result = deserialize_params::<UpdateCardParams>(params);
        assert!(result.is_ok());
    }

    #[test]
    fn deserialize_redeem_coupon_params() {
        let params: Map<String, Value> = serde_json::from_value(json!({"code": "SAVE50"})).unwrap();
        let result = deserialize_params::<RedeemCouponParams>(params);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().code, "SAVE50");
    }

    #[test]
    fn deserialize_invalid_params_returns_error() {
        let params: Map<String, Value> = serde_json::from_value(json!({})).unwrap();
        let result = deserialize_params::<PurchasePlanParams>(params);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid params"));
    }

    // Helper function tests

    #[test]
    fn required_string_helper() {
        let f = required_string("name", "a comment");
        assert_eq!(f.name, "name");
        assert!(f.required);
        assert!(matches!(f.ty, TypeSchema::String));
    }

    #[test]
    fn optional_string_helper() {
        let f = optional_string("gateway", "desc");
        assert_eq!(f.name, "gateway");
        assert!(!f.required);
    }

    #[test]
    fn optional_u64_helper() {
        let f = optional_u64("limit", "desc");
        assert_eq!(f.name, "limit");
        assert!(!f.required);
    }

    #[test]
    fn json_output_helper() {
        let f = json_output("result", "desc");
        assert_eq!(f.name, "result");
        assert!(f.required);
    }

    #[test]
    fn output_field_helper() {
        let f = output_field("url", TypeSchema::String, "desc");
        assert_eq!(f.name, "url");
        assert!(f.required);
    }

    #[test]
    fn schemas_and_controllers_are_consistent() {
        let schemas = all_billing_controller_schemas();
        let controllers = all_billing_registered_controllers();
        assert_eq!(schemas.len(), controllers.len());
        for (s, c) in schemas.iter().zip(controllers.iter()) {
            assert_eq!(s.namespace, c.schema.namespace);
            assert_eq!(s.function, c.schema.function);
        }
    }
}
