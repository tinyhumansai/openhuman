use super::*;
use serde_json::json;

#[test]
fn normalize_gateway_defaults_to_stripe() {
    assert_eq!(normalize_gateway(None).unwrap(), "stripe");
    assert_eq!(
        normalize_gateway(Some("   ".to_string())).unwrap(),
        "stripe"
    );
}

#[test]
fn normalize_gateway_accepts_supported_values_case_insensitively() {
    assert_eq!(
        normalize_gateway(Some(" Stripe ".to_string())).unwrap(),
        "stripe"
    );
    assert_eq!(
        normalize_gateway(Some("COINBASE".to_string())).unwrap(),
        "coinbase"
    );
}

#[test]
fn normalize_gateway_rejects_unknown_values() {
    assert_eq!(
        normalize_gateway(Some("paypal".to_string())),
        Err("gateway must be one of: stripe, coinbase".to_string())
    );
    assert_eq!(
        normalize_gateway(Some("".to_string())),
        Ok("stripe".to_string()),
        "empty string falls through to default, matching None"
    );
}

// --- pre-HTTP input validation (no network) ---------------------------
//
// These tests only exercise the argument checks that run *before* any
// HTTP call. They must not depend on the backend, stored session token,
// or filesystem state — only on input shape.

fn cfg() -> Config {
    Config::default()
}

#[tokio::test]
async fn purchase_plan_rejects_empty_plan() {
    let err = purchase_plan(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "plan is required");
}

#[tokio::test]
async fn purchase_plan_rejects_whitespace_only_plan() {
    // Whitespace must be trimmed and then rejected.
    let err = purchase_plan(&cfg(), "   \t\n").await.unwrap_err();
    assert_eq!(err, "plan is required");
}

#[tokio::test]
async fn create_coinbase_charge_rejects_empty_plan() {
    let err = create_coinbase_charge(&cfg(), "", None).await.unwrap_err();
    assert_eq!(err, "plan is required");
}

#[tokio::test]
async fn create_coinbase_charge_rejects_whitespace_plan() {
    let err = create_coinbase_charge(&cfg(), "   ", Some("monthly".into()))
        .await
        .unwrap_err();
    assert_eq!(err, "plan is required");
}

#[tokio::test]
async fn update_card_rejects_empty_payment_method_id() {
    let err = update_card(&cfg(), "", json!({})).await.unwrap_err();
    assert_eq!(err, "paymentMethodId is required");
}

#[tokio::test]
async fn update_card_rejects_whitespace_payment_method_id() {
    let err = update_card(&cfg(), "  \t", json!({})).await.unwrap_err();
    assert_eq!(err, "paymentMethodId is required");
}

#[tokio::test]
async fn delete_card_rejects_empty_payment_method_id() {
    let err = delete_card(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "paymentMethodId is required");
}

#[tokio::test]
async fn redeem_coupon_rejects_empty_code() {
    let err = redeem_coupon(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "code is required");
}

#[tokio::test]
async fn redeem_coupon_rejects_whitespace_code() {
    let err = redeem_coupon(&cfg(), "   ").await.unwrap_err();
    assert_eq!(err, "code is required");
}

#[tokio::test]
async fn top_up_rejects_zero_amount() {
    let err = top_up_credits(&cfg(), 0.0, None).await.unwrap_err();
    assert!(err.contains("amountUsd must be a finite number greater than 0"));
}

#[tokio::test]
async fn top_up_rejects_negative_amount() {
    let err = top_up_credits(&cfg(), -1.0, None).await.unwrap_err();
    assert!(err.contains("amountUsd must be a finite number greater than 0"));
}

#[tokio::test]
async fn top_up_rejects_nan_amount() {
    let err = top_up_credits(&cfg(), f64::NAN, None).await.unwrap_err();
    assert!(err.contains("amountUsd must be a finite number greater than 0"));
}

#[tokio::test]
async fn top_up_rejects_infinity_amount() {
    let err = top_up_credits(&cfg(), f64::INFINITY, None)
        .await
        .unwrap_err();
    assert!(err.contains("amountUsd must be a finite number greater than 0"));
    let err = top_up_credits(&cfg(), f64::NEG_INFINITY, None)
        .await
        .unwrap_err();
    assert!(err.contains("amountUsd must be a finite number greater than 0"));
}

#[tokio::test]
async fn top_up_rejects_invalid_gateway_after_amount_passes() {
    // Amount validation passes → gateway validation kicks in and rejects.
    let err = top_up_credits(&cfg(), 10.0, Some("paypal".into()))
        .await
        .unwrap_err();
    assert_eq!(err, "gateway must be one of: stripe, coinbase");
}

// --- SDK-backed calls against a mock backend -----------------------------

use crate::hosted::test_support;
use tempfile::TempDir;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn parse_enum_names_field_and_value() {
    assert_eq!(
        parse_enum::<BillingPlan>("plan", "PRO_MONTHLY").unwrap(),
        BillingPlan::ProMonthly
    );
    assert_eq!(
        parse_enum::<BillingPlan>("plan", "GOLD").unwrap_err(),
        "unsupported plan: GOLD"
    );
}

#[tokio::test]
async fn purchase_plan_rejects_unknown_plan_before_any_request() {
    let err = purchase_plan(&cfg(), "GOLD").await.unwrap_err();
    assert_eq!(err, "unsupported plan: GOLD");
}

#[tokio::test]
async fn billing_calls_send_schema_current_requests() {
    let server = MockServer::start().await;
    let ok =
        || ResponseTemplate::new(200).set_body_json(json!({"success": true, "data": {"ok": true}}));
    Mock::given(method("GET"))
        .and(path("/payments/credits/transactions"))
        .and(query_param("limit", "20"))
        .and(query_param("offset", "0"))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/payments/credits/top-up"))
        .and(body_json(json!({"amountUsd": 5.0, "gateway": "coinbase"})))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/payments/stripe/purchasePlan"))
        .and(body_json(json!({"plan": "BASIC_YEARLY"})))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/payments/coinbase/charge"))
        .and(body_json(json!({"plan": "PRO", "interval": "annual"})))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/payments/credits/auto-recharge"))
        .and(body_json(json!({"enabled": true, "weeklyLimitUsd": 50})))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/payments/credits/auto-recharge/cards/pm_1"))
        .and(body_json(json!({"isDefault": true})))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/coupons/redeem"))
        .and(body_json(json!({"code": "FREE"})))
        .respond_with(ok())
        .expect(1)
        .mount(&server)
        .await;
    for (verb, route) in [
        ("GET", "/payments/stripe/currentPlan"),
        ("GET", "/payments/summary"),
        ("GET", "/payments/credits/balance"),
        ("GET", "/payments/credits/auto-recharge"),
        ("GET", "/payments/credits/auto-recharge/cards"),
        ("POST", "/payments/credits/auto-recharge/cards/setup-intent"),
        ("DELETE", "/payments/credits/auto-recharge/cards/pm_1"),
        ("POST", "/payments/stripe/portal"),
        ("GET", "/coupons/me"),
    ] {
        Mock::given(method(verb))
            .and(path(route))
            .respond_with(ok())
            .expect(1)
            .mount(&server)
            .await;
    }

    let tmp = TempDir::new().unwrap();
    let config = test_support::signed_in(&tmp, &server.uri());
    get_transactions(&config, None, None).await.unwrap();
    top_up_credits(&config, 5.0, Some(" Coinbase ".into()))
        .await
        .unwrap();
    purchase_plan(&config, "BASIC_YEARLY").await.unwrap();
    create_coinbase_charge(&config, "PRO", None).await.unwrap();
    update_auto_recharge(&config, json!({"enabled": true, "weeklyLimitUsd": 50}))
        .await
        .unwrap();
    update_card(&config, "pm_1", json!({"isDefault": true}))
        .await
        .unwrap();
    redeem_coupon(&config, " FREE ").await.unwrap();
    let out = get_current_plan(&config).await.unwrap();
    assert_eq!(out.value, json!({"ok": true}));
    get_summary(&config).await.unwrap();
    get_balance(&config).await.unwrap();
    get_auto_recharge(&config).await.unwrap();
    get_cards(&config).await.unwrap();
    create_setup_intent(&config).await.unwrap();
    delete_card(&config, "pm_1").await.unwrap();
    create_portal_session(&config).await.unwrap();
    get_user_coupons(&config).await.unwrap();
}

#[tokio::test]
async fn update_card_rejects_non_object_payload() {
    let tmp = TempDir::new().unwrap();
    let config = test_support::signed_in(&tmp, "http://127.0.0.1:9");
    let err = update_card(&config, "pm_1", json!([1])).await.unwrap_err();
    assert_eq!(err, "card update payload must be an object");
}
