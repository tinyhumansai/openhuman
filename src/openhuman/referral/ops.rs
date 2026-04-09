//! Referral program — authenticated calls to the hosted API (`/referral/*`).
//!
//! The desktop WebView `fetch` to the backend can fail with a generic "Load failed"
//! (CORS / TLS / WebKit). These ops reuse the same `reqwest` path as billing.

use reqwest::Method;
use serde_json::{json, Map, Value};

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

pub async fn get_stats(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let token = require_token(config)?;
    let api_url = effective_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let data = client
        .authed_json(&token, Method::GET, "/referral/stats", None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        data,
        "referral stats fetched from backend GET /referral/stats",
    ))
}

pub async fn apply_code(
    config: &Config,
    code: &str,
    device_fingerprint: Option<&str>,
) -> Result<RpcOutcome<Value>, String> {
    let token = require_token(config)?;
    let api_url = effective_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;

    let mut body = Map::new();
    body.insert("code".to_string(), json!(code.trim()));
    if let Some(fp) = device_fingerprint.map(str::trim).filter(|s| !s.is_empty()) {
        body.insert("deviceFingerprint".to_string(), json!(fp));
    }

    let data = client
        .authed_json(
            &token,
            Method::POST,
            "/referral/apply",
            Some(Value::Object(body)),
        )
        .await
        .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::single_log(
        data,
        "referral apply accepted by backend POST /referral/apply",
    ))
}
