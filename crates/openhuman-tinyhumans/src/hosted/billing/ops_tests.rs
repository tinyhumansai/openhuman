use super::*;

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
