//! Referral program — authenticated calls to the hosted API (`/referral/*`)
//! through the SDK's typed `referral()` client.
//!
//! The desktop WebView `fetch` to the backend can fail with a generic "Load
//! failed" (CORS / TLS / WebKit), so these run in-process like billing.

use serde_json::Value;
use tinyhumans_sdk::api::types::ClaimReferralRequest;

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::core::Outcome;

use crate::hosted::client::HostedClient;

pub async fn get_stats(config: &Config) -> Result<Outcome<Value>, String> {
    let client = HostedClient::from_config(config).await?;
    let data = client.finish_value(
        "GET /referral/stats",
        client.sdk().referral().get_referral_stats().await,
    )?;
    Ok(Outcome::single_log(
        data,
        "referral stats fetched from backend GET /referral/stats",
    ))
}

pub async fn claim_referral(
    config: &Config,
    code: &str,
    device_fingerprint: Option<&str>,
) -> Result<Outcome<Value>, String> {
    let client = HostedClient::from_config(config).await?;
    let request = ClaimReferralRequest {
        code: code.trim().to_string(),
        device_fingerprint: device_fingerprint
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    };
    let data = client.finish_value(
        "POST /referral/claim",
        client.sdk().referral().claim_referral(&request).await,
    )?;
    Ok(Outcome::single_log(
        data,
        "referral claim accepted by backend POST /referral/claim",
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
