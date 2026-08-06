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
    let params: Map<String, Value> = serde_json::from_value(json!({"amountUsd": 10.0})).unwrap();
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
    let params: Map<String, Value> =
        serde_json::from_value(json!({"paymentMethodId": "pm_1", "payload": {"default": true}}))
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
