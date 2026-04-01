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

use log::debug;
use reqwest::{header::AUTHORIZATION, Client, Method, Url};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

use crate::api::config::effective_api_url;
use crate::api::jwt::{bearer_authorization_value, get_session_token};
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[billing]";

fn build_client() -> Result<Client, String> {
    Client::builder()
        .use_rustls_tls()
        .http1_only()
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))
}

fn resolve_base(config: &Config) -> Result<Url, String> {
    let base = effective_api_url(&config.api_url);
    Url::parse(base.trim()).map_err(|e| format!("invalid api_url '{}': {e}", base))
}

async fn authed_request(
    client: &Client,
    base: &Url,
    token: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let url = base
        .join(path.trim_start_matches('/'))
        .map_err(|e| format!("build URL failed: {e}"))?;

    let mut req = client
        .request(method.clone(), url.clone())
        .header(AUTHORIZATION, bearer_authorization_value(token));

    if let Some(b) = body {
        req = req.json(&b);
    }

    let resp = req.send().await.map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();

    debug!("{LOG_PREFIX} {} {} -> {}", method, url, status);

    let raw: Value = serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text.clone()));
    if !status.is_success() {
        let msg = raw
            .as_object()
            .and_then(|o| {
                o.get("message")
                    .or_else(|| o.get("error"))
                    .or_else(|| o.get("detail"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or(&text);
        return Err(format!(
            "backend responded with {} for {}: {}",
            status.as_u16(),
            url.path(),
            msg
        ));
    }

    unwrap_api_envelope(raw)
}

fn unwrap_api_envelope(raw: Value) -> Result<Value, String> {
    if let Some(obj) = raw.as_object() {
        if let Some(success) = obj.get("success").and_then(|v| v.as_bool()) {
            if !success {
                let msg = obj
                    .get("message")
                    .or_else(|| obj.get("error"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("request unsuccessful");
                return Err(msg.to_string());
            }
        }
        if let Some(data) = obj.get("data") {
            return Ok(data.clone());
        }
    }
    Ok(raw)
}

fn require_token(config: &Config) -> Result<String, String> {
    get_session_token(config)?
        .and_then(|v| {
            let t = v.trim().to_string();
            if t.is_empty() { None } else { Some(t) }
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
    let client = build_client()?;
    let base = resolve_base(config)?;
    authed_request(&client, &base, &token, method, path, body).await
}

pub async fn get_current_plan(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(
        config,
        Method::GET,
        "/payments/stripe/currentPlan",
        None,
    )
    .await?;
    Ok(RpcOutcome::single_log(
        data,
        "current plan fetched from backend",
    ))
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
    let data = get_authed_value(
        config,
        Method::POST,
        "/payments/stripe/portal",
        None,
    )
    .await?;
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

pub async fn top_up_credits(
    config: &Config,
    amount_usd: f64,
    gateway: Option<String>,
) -> Result<RpcOutcome<Value>, String> {
    if amount_usd <= 0.0 {
        return Err("amountUsd must be greater than 0".to_string());
    }

    let body = TopUpBody {
        amount_usd,
        gateway: gateway
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty())
            .unwrap_or_else(default_gateway),
    };

    let data = get_authed_value(
        config,
        Method::POST,
        "/payments/credits/top-up",
        Some(json!(body)),
    )
    .await?;

    Ok(RpcOutcome::single_log(
        data,
        "credit top-up initiated",
    ))
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
