use super::*;
use serde_json::json;

#[test]
fn billing_controller_wire_contracts_match_registration() {
    let schemas = all_billing_controller_schemas();
    let controllers = all_billing_registered_controllers();
    assert_eq!(schemas.len(), 16);
    assert_eq!(controllers.len(), 16);
    for (schema, controller) in schemas.iter().zip(&controllers) {
        assert_eq!(schema.namespace, controller.schema.namespace);
        assert_eq!(schema.function, controller.schema.function);
    }
    for (function, input_count) in [
        ("get_summary", 0),
        ("get_current_plan", 0),
        ("get_balance", 0),
        ("purchase_plan", 1),
        ("create_portal_session", 0),
        ("top_up", 2),
        ("create_coinbase_charge", 2),
        ("get_transactions", 2),
        ("get_auto_recharge", 0),
        ("update_auto_recharge", 1),
        ("get_cards", 0),
        ("create_setup_intent", 0),
        ("update_card", 2),
        ("delete_card", 1),
        ("redeem_coupon", 1),
        ("get_coupons", 0),
    ] {
        let schema = billing_schemas(&format!("billing_{function}"));
        assert_eq!(schema.function, function);
        assert_eq!(schema.namespace, "billing");
        assert_eq!(schema.inputs.len(), input_count, "{function} input count");
    }
    for function in ["get_summary", "get_current_plan"] {
        assert!(!billing_schemas(&format!("billing_{function}"))
            .outputs
            .is_empty());
    }
    let purchase = billing_schemas("billing_purchase_plan");
    assert_eq!(purchase.inputs[0].name, "plan");
    assert!(purchase.inputs[0].required);
    assert!(purchase.outputs.len() >= 2);
    let top_up = billing_schemas("billing_top_up");
    assert_eq!(top_up.inputs[0].name, "amountUsd");
    assert!(top_up.inputs[0].required);
    assert!(!top_up.inputs[1].required);
    assert!(
        billing_schemas("billing_create_coinbase_charge")
            .outputs
            .len()
            >= 4
    );
    assert!(billing_schemas("billing_get_transactions")
        .inputs
        .iter()
        .all(|input| !input.required));
    assert_eq!(
        billing_schemas("billing_update_auto_recharge").inputs[0].name,
        "payload"
    );
    assert_eq!(
        billing_schemas("billing_redeem_coupon").inputs[0].name,
        "code"
    );
}

// Param deserialization tests

#[test]
fn deserialize_purchase_plan_params() {
    let params: Map<String, Value> = serde_json::from_value(json!({"plan": "pro"})).unwrap();
    let result = deserialize_params::<PurchasePlanParams>(params);
    assert_eq!(result.unwrap().plan, "pro");
}

#[test]
fn deserialize_top_up_params() {
    let params: Map<String, Value> = serde_json::from_value(json!({"amountUsd": 10.0})).unwrap();
    let result = deserialize_params::<TopUpParams>(params);
    let p = result.unwrap();
    assert_eq!(p.amount_usd, 10.0);
    assert!(p.gateway.is_none());
}

#[test]
fn deserialize_top_up_params_with_gateway() {
    let params: Map<String, Value> =
        serde_json::from_value(json!({"amountUsd": 5.0, "gateway": "stripe"})).unwrap();
    let result = deserialize_params::<TopUpParams>(params);
    assert_eq!(result.unwrap().gateway.as_deref(), Some("stripe"));
}

#[test]
fn deserialize_coinbase_charge_params() {
    let params: Map<String, Value> =
        serde_json::from_value(json!({"plan": "enterprise", "interval": "annual"})).unwrap();
    let result = deserialize_params::<CoinbaseChargeParams>(params);
    let p = result.unwrap();
    assert_eq!(p.plan, "enterprise");
    assert_eq!(p.interval.as_deref(), Some("annual"));
}

#[test]
fn deserialize_transactions_params_defaults() {
    let params: Map<String, Value> = serde_json::from_value(json!({})).unwrap();
    let result = deserialize_params::<TransactionsParams>(params);
    let p = result.unwrap();
    assert!(p.limit.is_none());
    assert!(p.offset.is_none());
}

#[test]
fn deserialize_transactions_params_with_values() {
    let params: Map<String, Value> =
        serde_json::from_value(json!({"limit": 10, "offset": 5})).unwrap();
    let result = deserialize_params::<TransactionsParams>(params);
    let p = result.unwrap();
    assert_eq!(p.limit, Some(10));
    assert_eq!(p.offset, Some(5));
}

#[test]
fn deserialize_card_params() {
    let params: Map<String, Value> =
        serde_json::from_value(json!({"paymentMethodId": "pm_123"})).unwrap();
    let result = deserialize_params::<CardParams>(params);
    assert_eq!(result.unwrap().payment_method_id, "pm_123");
}

#[test]
fn deserialize_update_card_params() {
    let params: Map<String, Value> =
        serde_json::from_value(json!({"paymentMethodId": "pm_1", "payload": {"default": true}}))
            .unwrap();
    deserialize_params::<UpdateCardParams>(params).expect("valid card update");
}

#[test]
fn deserialize_redeem_coupon_params() {
    let params: Map<String, Value> = serde_json::from_value(json!({"code": "SAVE50"})).unwrap();
    let result = deserialize_params::<RedeemCouponParams>(params);
    assert_eq!(result.unwrap().code, "SAVE50");
}

#[test]
fn deserialize_invalid_params_returns_error() {
    let params: Map<String, Value> = serde_json::from_value(json!({})).unwrap();
    let result = deserialize_params::<PurchasePlanParams>(params);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("invalid params"));
}
