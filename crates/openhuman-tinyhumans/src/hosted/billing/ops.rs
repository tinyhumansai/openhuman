//! Billing and payment RPC ops — thin adapters over the SDK's typed
//! `payments()` and `coupons()` clients ([`HostedClient`]).
//!
//! # Security
//! Every call authenticates with the core's resolved backend credential — a
//! session JWT as `Authorization: Bearer`, an API key as `x-api-key`.
//! **No server-side authorization is replicated here**: the backend enforces plan
//! ownership, tenant isolation, and payment policy on every request.
//! Callers that lack a valid session or sufficient permissions receive a
//! backend 401/403 error surfaced verbatim as an RPC error string.
//! API keys / JWTs are never written to logs (only redacted status codes + paths).

use reqwest::Method;
use serde_json::{Map, Value};
use tinyhumans_sdk::api::payments::{
    CoinbaseInterval, CoinbasePlan, CreateCoinbaseChargeRequest, CreditTopUpRequest,
    PaymentGateway, PurchaseStripePlanRequest, UpdateAutoRechargeCardRequest,
};
use tinyhumans_sdk::api::types::{BillingPlan, CodeRequest};

use openhuman_core::config::Config;
use openhuman_core::rpc::RpcOutcome;

use crate::hosted::client::HostedClient;

/// Parse a wire string into one of the SDK's closed request enums, naming the
/// field and the offending value on failure. The plan enums are
/// `SCREAMING_SNAKE_CASE` on the wire, so a lower-case plan (`"pro"`) is
/// accepted as its upper-case form; the lower-case `interval`/`gateway` enums
/// are matched as given.
fn parse_enum<T: serde::de::DeserializeOwned>(field: &str, raw: &str) -> Result<T, String> {
    serde_json::from_value(Value::String(raw.to_string()))
        .or_else(|_| serde_json::from_value(Value::String(raw.to_ascii_uppercase())))
        .map_err(|_| format!("unsupported {field}: {raw}"))
}

pub async fn get_current_plan(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "GET /payments/stripe/currentPlan",
        client.sdk().payments().get_current_plan().await,
    )?;
    Ok(RpcOutcome::single_log(
        data,
        "current plan fetched from backend",
    ))
}

pub async fn get_summary(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "GET /payments/summary",
        client.sdk().payments().get_summary().await,
    )?;
    Ok(RpcOutcome::single_log(data, "billing summary fetched"))
}

pub async fn get_balance(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "GET /payments/credits/balance",
        client.sdk().payments().get_credit_balance().await,
    )?;
    Ok(RpcOutcome::single_log(data, "credit balance fetched"))
}

pub async fn get_transactions(
    config: &Config,
    limit: Option<u64>,
    offset: Option<u64>,
) -> Result<RpcOutcome<Value>, String> {
    let limit = limit.unwrap_or(20);
    let offset = offset.unwrap_or(0);
    let client = HostedClient::from_config(config)?;
    let query = [
        ("limit", Some(limit.to_string())),
        ("offset", Some(offset.to_string())),
    ];
    let data = client.finish_value(
        "GET /payments/credits/transactions",
        client
            .sdk()
            .payments()
            .list_credit_transactions(&query)
            .await,
    )?;
    Ok(RpcOutcome::single_log(data, "credit transactions fetched"))
}

pub async fn get_auto_recharge(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "GET /payments/credits/auto-recharge",
        client.sdk().payments().get_auto_recharge().await,
    )?;
    Ok(RpcOutcome::single_log(
        data,
        "auto recharge settings fetched",
    ))
}

/// `PATCH /payments/credits/auto-recharge`.
///
/// Forwarded verbatim on the SDK's raw primitive (route policy still applies)
/// rather than through the typed `update_auto_recharge`: the SDK's
/// `AutoRechargeRequest` has no `weeklyLimitUsd`, which the settings UI sends,
/// so decoding into it would silently drop the weekly cap.
pub async fn update_auto_recharge(
    config: &Config,
    payload: Value,
) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let data = client.finish(
        "PATCH /payments/credits/auto-recharge",
        client
            .sdk()
            .raw()
            .send(
                Method::PATCH,
                "/payments/credits/auto-recharge",
                &[],
                Some(&payload),
                true,
            )
            .await,
    )?;
    Ok(RpcOutcome::single_log(
        data,
        "auto recharge settings updated",
    ))
}

pub async fn get_cards(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "GET /payments/credits/auto-recharge/cards",
        client.sdk().payments().list_auto_recharge_cards().await,
    )?;
    Ok(RpcOutcome::single_log(data, "saved cards fetched"))
}

pub async fn create_setup_intent(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "POST /payments/credits/auto-recharge/cards/setup-intent",
        client
            .sdk()
            .payments()
            .create_auto_recharge_card_setup_intent()
            .await,
    )?;
    Ok(RpcOutcome::single_log(data, "setup intent created"))
}

pub async fn update_card(
    config: &Config,
    payment_method_id: &str,
    payload: Value,
) -> Result<RpcOutcome<Value>, String> {
    let payment_method_id = payment_method_id.trim();
    if payment_method_id.is_empty() {
        return Err("paymentMethodId is required".to_string());
    }
    let fields = match payload {
        Value::Object(map) => map,
        Value::Null => Map::new(),
        _ => return Err("card update payload must be an object".to_string()),
    };
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "PATCH /payments/credits/auto-recharge/cards/{paymentMethodId}",
        client
            .sdk()
            .payments()
            .update_auto_recharge_card(payment_method_id, &UpdateAutoRechargeCardRequest { fields })
            .await,
    )?;
    Ok(RpcOutcome::single_log(data, "saved card updated"))
}

pub async fn delete_card(
    config: &Config,
    payment_method_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let payment_method_id = payment_method_id.trim();
    if payment_method_id.is_empty() {
        return Err("paymentMethodId is required".to_string());
    }
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "DELETE /payments/credits/auto-recharge/cards/{paymentMethodId}",
        client
            .sdk()
            .payments()
            .delete_auto_recharge_card(payment_method_id)
            .await,
    )?;
    Ok(RpcOutcome::single_log(data, "saved card deleted"))
}

/// `POST /payments/stripe/purchasePlan`. `plan` is one of the SDK's
/// [`BillingPlan`] values (`BASIC_MONTHLY`, `BASIC_YEARLY`, `PRO_MONTHLY`,
/// `PRO_YEARLY` — the frontend's `PlanIdentifier`).
pub async fn purchase_plan(config: &Config, plan: &str) -> Result<RpcOutcome<Value>, String> {
    let plan = plan.trim();
    if plan.is_empty() {
        return Err("plan is required".to_string());
    }
    let request = PurchaseStripePlanRequest {
        plan: parse_enum::<BillingPlan>("plan", plan)?,
        coupon_code: None,
    };
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "POST /payments/stripe/purchasePlan",
        client.sdk().payments().purchase_stripe_plan(&request).await,
    )?;
    Ok(RpcOutcome::single_log(
        data,
        "plan purchase session created",
    ))
}

pub async fn create_portal_session(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "POST /payments/stripe/portal",
        client.sdk().payments().create_stripe_portal_session().await,
    )?;
    Ok(RpcOutcome::single_log(
        data,
        "customer portal session created",
    ))
}

fn default_gateway() -> String {
    "stripe".to_string()
}

fn normalize_gateway(gateway: Option<String>) -> Result<String, String> {
    let gateway = gateway
        .as_deref()
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(default_gateway);

    if !matches!(gateway.as_str(), "stripe" | "coinbase") {
        return Err("gateway must be one of: stripe, coinbase".to_string());
    }

    Ok(gateway)
}

pub async fn top_up_credits(
    config: &Config,
    amount_usd: f64,
    gateway: Option<String>,
) -> Result<RpcOutcome<Value>, String> {
    if !amount_usd.is_finite() || amount_usd <= 0.0 {
        return Err("amountUsd must be a finite number greater than 0".to_string());
    }

    let gateway = normalize_gateway(gateway)?;
    let request = CreditTopUpRequest {
        amount_usd,
        gateway: Some(parse_enum::<PaymentGateway>("gateway", &gateway)?),
    };
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "POST /payments/credits/top-up",
        client.sdk().payments().create_credit_top_up(&request).await,
    )?;
    Ok(RpcOutcome::single_log(data, "credit top-up initiated"))
}

/// Create a Coinbase Commerce charge (the "payment link" for crypto / annual billing).
/// Maps to `POST /payments/coinbase/charge` — matches `billingApi.createCoinbaseCharge`.
pub async fn create_coinbase_charge(
    config: &Config,
    plan: &str,
    interval: Option<String>,
) -> Result<RpcOutcome<Value>, String> {
    let plan = plan.trim();
    if plan.is_empty() {
        return Err("plan is required".to_string());
    }

    let interval_str = interval
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("annual");

    let request = CreateCoinbaseChargeRequest {
        plan: parse_enum::<CoinbasePlan>("plan", plan)?,
        interval: Some(parse_enum::<CoinbaseInterval>("interval", interval_str)?),
        metadata: Map::new(),
    };
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "POST /payments/coinbase/charge",
        client
            .sdk()
            .payments()
            .create_coinbase_charge(&request)
            .await,
    )?;
    Ok(RpcOutcome::single_log(
        data,
        "Coinbase payment link created",
    ))
}

// ── Coupon operations ──────────────────────────────────────────────────────

/// Redeem a coupon code to add credits to the user's account.
/// Maps to `POST /coupons/redeem`.
pub async fn redeem_coupon(config: &Config, code: &str) -> Result<RpcOutcome<Value>, String> {
    let code = code.trim();
    if code.is_empty() {
        return Err("code is required".to_string());
    }
    let request = CodeRequest {
        code: code.to_string(),
    };
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "POST /coupons/redeem",
        client.sdk().coupons().redeem_coupon(&request).await,
    )?;
    Ok(RpcOutcome::single_log(data, "coupon redeemed"))
}

/// List coupons redeemed by the current user.
/// Maps to `GET /coupons/me`.
pub async fn get_user_coupons(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let client = HostedClient::from_config(config)?;
    let data = client.finish_value(
        "GET /coupons/me",
        client.sdk().coupons().list_my_coupons().await,
    )?;
    Ok(RpcOutcome::single_log(data, "user coupons fetched"))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
