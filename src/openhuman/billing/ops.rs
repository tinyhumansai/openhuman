//! Billing and payment RPC ops — thin adapters that call the hosted API.
//!
//! # Security
//! All methods require a valid app-session JWT stored via `auth_store_session`.
//! The JWT is sent as `Authorization: Bearer …` to the backend.
//! **No server-side authorization is replicated here**: the backend enforces plan
//! ownership, tenant isolation, and payment policy on every request.
//! Callers that lack a valid session or sufficient permissions receive a
//! backend 401/403 error surfaced verbatim as an RPC error string.
//! API keys / JWTs are never written to logs (only redacted status codes + paths).

use reqwest::Method;
use serde::Serialize;
use serde_json::{json, Value};

use crate::api::config::effective_api_url;
use crate::api::jwt::get_session_token;
use crate::api::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

fn require_token(config: &Config) -> Result<String, String> {
    get_session_token(config)?
        .and_then(|v| {
            let t = v.trim().to_string();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        })
        .ok_or_else(|| "no backend session token; run auth_store_session first".to_string())
}

async fn get_authed_value(
    config: &Config,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let token = require_token(config)?;
    let api_url = effective_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    client
        .authed_json(&token, method, path, body)
        .await
        .map_err(|e| e.to_string())
}

pub async fn get_current_plan(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(config, Method::GET, "/payments/stripe/currentPlan", None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "current plan fetched from backend",
    ))
}

pub async fn get_balance(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(config, Method::GET, "/payments/credits/balance", None).await?;
    Ok(RpcOutcome::single_log(data, "credit balance fetched"))
}

pub async fn get_transactions(
    config: &Config,
    limit: Option<u64>,
    offset: Option<u64>,
) -> Result<RpcOutcome<Value>, String> {
    let limit = limit.unwrap_or(20);
    let offset = offset.unwrap_or(0);
    let path = format!("/payments/credits/transactions?limit={limit}&offset={offset}");
    let data = get_authed_value(config, Method::GET, &path, None).await?;
    Ok(RpcOutcome::single_log(data, "credit transactions fetched"))
}

pub async fn get_auto_recharge(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data =
        get_authed_value(config, Method::GET, "/payments/credits/auto-recharge", None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "auto recharge settings fetched",
    ))
}

pub async fn update_auto_recharge(
    config: &Config,
    payload: Value,
) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(
        config,
        Method::PATCH,
        "/payments/credits/auto-recharge",
        Some(payload),
    )
    .await?;
    Ok(RpcOutcome::single_log(
        data,
        "auto recharge settings updated",
    ))
}

pub async fn get_cards(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(
        config,
        Method::GET,
        "/payments/credits/auto-recharge/cards",
        None,
    )
    .await?;
    Ok(RpcOutcome::single_log(data, "saved cards fetched"))
}

pub async fn create_setup_intent(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(
        config,
        Method::POST,
        "/payments/credits/auto-recharge/cards/setup-intent",
        None,
    )
    .await?;
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
    let path = format!(
        "/payments/credits/auto-recharge/cards/{}",
        urlencoding::encode(payment_method_id)
    );
    let data = get_authed_value(config, Method::PATCH, &path, Some(payload)).await?;
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
    let path = format!(
        "/payments/credits/auto-recharge/cards/{}",
        urlencoding::encode(payment_method_id)
    );
    let data = get_authed_value(config, Method::DELETE, &path, None).await?;
    Ok(RpcOutcome::single_log(data, "saved card deleted"))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PurchasePlanBody<'a> {
    plan: &'a str,
}

pub async fn purchase_plan(config: &Config, plan: &str) -> Result<RpcOutcome<Value>, String> {
    let plan = plan.trim();
    if plan.is_empty() {
        return Err("plan is required".to_string());
    }

    let body = json!(PurchasePlanBody { plan });
    let data = get_authed_value(
        config,
        Method::POST,
        "/payments/stripe/purchasePlan",
        Some(body),
    )
    .await?;

    Ok(RpcOutcome::single_log(
        data,
        "plan purchase session created",
    ))
}

pub async fn create_portal_session(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(config, Method::POST, "/payments/stripe/portal", None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "customer portal session created",
    ))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TopUpBody {
    amount_usd: f64,
    #[serde(default = "default_gateway")]
    gateway: String,
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
    let body = TopUpBody {
        amount_usd,
        gateway,
    };

    let data = get_authed_value(
        config,
        Method::POST,
        "/payments/credits/top-up",
        Some(json!(body)),
    )
    .await?;

    Ok(RpcOutcome::single_log(data, "credit top-up initiated"))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoinbaseChargeBody<'a> {
    plan: &'a str,
    interval: &'a str,
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

    let body = json!(CoinbaseChargeBody {
        plan,
        interval: interval_str,
    });

    let data = get_authed_value(
        config,
        Method::POST,
        "/payments/coinbase/charge",
        Some(body),
    )
    .await?;

    Ok(RpcOutcome::single_log(
        data,
        "Coinbase payment link created",
    ))
}

#[cfg(test)]
mod tests {
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
    }
}
