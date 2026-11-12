//! `openhuman.x402_fetch`: one HTTP request that may pay an HTTP 402 challenge
//! within a cap the caller states.
//!
//! For a local, non-model caller with no agent turn. The request goes through
//! the same URL guard as `http_request` (https only, no local or private
//! destinations, connections pinned to the vetted addresses, no redirects).
//! On a 402 the challenge goes to TinyWallet through
//! [`x402_service::prepare_capped_payment`], whose one-shot host permit lets
//! the authorization callback approve only this payment, only within
//! `maxAmountAtomic` and the per-request budget. The paid retry's outcome is
//! always reported back so the module ledger commits or keeps the reservation.
//!
//! Logging names the host, status and payment amount only: never the URL path
//! or query, request headers, bodies, or wallet material.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tinyagents_harness::url_guard::ValidatedUrl;

use crate::core::runtime::DispatchCtx;
use crate::modules::x402_service::{self, X402RpcPermitOutcome};

/// Prefix of every error that means the payment was refused by policy (the
/// caller's cap or the host's authorization), as opposed to a failure.
pub(super) const REFUSED_PREFIX: &str = "X402_REFUSED:";

/// Largest response body returned to the caller, in bytes.
pub(super) const MAX_BODY_BYTES: usize = 256 * 1024;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PROXY_SERVICE: &str = "rpc.x402_fetch";
const USER_AGENT: &str = concat!("OpenHuman/", env!("CARGO_PKG_VERSION"));
/// Headers that carry an x402 payment; only the module may produce them.
const PAYMENT_HEADERS: [&str; 2] = ["PAYMENT-SIGNATURE", "X-PAYMENT"];

#[derive(Debug)]
pub(super) struct FetchParams {
    pub(super) url: String,
    pub(super) method: reqwest::Method,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Option<String>,
    pub(super) max_amount_atomic: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFetchParams {
    url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: Option<Map<String, Value>>,
    #[serde(default)]
    body: Option<String>,
    max_amount_atomic: u64,
}

/// Validate the caller's request before anything leaves the device.
pub(super) fn parse_params(params: Map<String, Value>) -> Result<FetchParams, String> {
    let raw: RawFetchParams = serde_json::from_value(Value::Object(params))
        .map_err(|error| format!("x402_fetch params: {error}"))?;

    let parsed = reqwest::Url::parse(raw.url.trim()).ok();
    if !parsed
        .as_ref()
        .is_some_and(|url| url.scheme() == "https" && url.host_str().is_some())
    {
        return Err("x402_fetch params: url must be an absolute https:// URL".into());
    }

    let method = match raw
        .method
        .as_deref()
        .map(str::to_ascii_uppercase)
        .as_deref()
    {
        None | Some("GET") => reqwest::Method::GET,
        Some("POST") => reqwest::Method::POST,
        Some(_) => return Err("x402_fetch params: method must be GET or POST".into()),
    };
    if raw.body.is_some() && method != reqwest::Method::POST {
        return Err("x402_fetch params: a body is only allowed with POST".into());
    }

    let mut headers = Vec::new();
    for (name, value) in raw.headers.unwrap_or_default() {
        if PAYMENT_HEADERS
            .iter()
            .any(|payment| payment.eq_ignore_ascii_case(&name))
        {
            return Err(format!(
                "x402_fetch params: the {name} header is set by the wallet, not the caller"
            ));
        }
        let Value::String(value) = value else {
            return Err(format!(
                "x402_fetch params: header {name} must have a string value"
            ));
        };
        if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err()
            || reqwest::header::HeaderValue::from_str(&value).is_err()
        {
            return Err(format!("x402_fetch params: header {name} is not valid"));
        }
        headers.push((name, value));
    }

    if raw.max_amount_atomic == 0 {
        return Err("x402_fetch params: maxAmountAtomic must be greater than zero".into());
    }

    Ok(FetchParams {
        url: raw.url.trim().to_owned(),
        method,
        headers,
        body: raw.body,
        max_amount_atomic: raw.max_amount_atomic,
    })
}

/// The `http_request` URL guard in open mode: refuses local and private
/// hosts, including names that resolve to them, and pins the vetted addresses.
pub(super) async fn guard_destination(url: &str) -> Result<ValidatedUrl, String> {
    tinyagents_harness::url_guard::validate_url_with_dns_check(url, &[])
        .await
        .map_err(|error| format!("x402_fetch: destination refused: {error}"))
}

/// The caller-facing error for a failed payment preparation: refusals carry
/// [`REFUSED_PREFIX`] so the caller can tell policy from failure.
pub(super) fn prepare_error(
    outcome: X402RpcPermitOutcome,
    error: impl std::fmt::Display,
) -> String {
    match outcome {
        X402RpcPermitOutcome::Refused(reason) => format!("{REFUSED_PREFIX} {reason}"),
        X402RpcPermitOutcome::Approved | X402RpcPermitOutcome::Unused => {
            format!("x402_fetch: payment failed: {error}")
        }
    }
}

/// Second line behind the host callback: the module's prepared amount must
/// not exceed the caller's cap.
pub(super) fn ensure_within_cap(amount_atomic: u64, max_amount_atomic: u64) -> Result<(), String> {
    if amount_atomic > max_amount_atomic {
        return Err(format!(
            "{REFUSED_PREFIX} x402 payment of {amount_atomic} exceeds maxAmountAtomic {max_amount_atomic}"
        ));
    }
    Ok(())
}

/// Decode at most `limit` bytes as UTF-8 text, reporting whether the body was
/// cut. A cut inside a multi-byte character drops that partial character.
pub(super) fn body_text(mut bytes: Vec<u8>, limit: usize) -> (String, bool) {
    let truncated = bytes.len() > limit;
    bytes.truncate(limit);
    match String::from_utf8(bytes) {
        Ok(text) => (text, truncated),
        Err(error) => {
            let utf8 = error.utf8_error();
            let mut bytes = error.into_bytes();
            if truncated && utf8.error_len().is_none() {
                bytes.truncate(utf8.valid_up_to());
            }
            (String::from_utf8_lossy(&bytes).into_owned(), truncated)
        }
    }
}

/// Public facts of a payment made for the request.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FetchPayment {
    pub(super) payment_id: String,
    pub(super) amount_atomic: u64,
    pub(super) asset: String,
    pub(super) recipient: String,
    pub(super) network: String,
    /// The paid retry succeeded and the module ledger recorded it as settled.
    pub(super) settled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) settlement_response: Option<String>,
}

pub(super) fn response_value(
    status: u16,
    content_type: Option<String>,
    body: String,
    truncated: bool,
    payment: Option<FetchPayment>,
) -> Value {
    let mut value = json!({
        "status": status,
        "body": body,
        "truncated": truncated,
        "payment": payment,
    });
    if let (Some(content_type), Some(object)) = (content_type, value.as_object_mut()) {
        object.insert("contentType".into(), Value::String(content_type));
    }
    value
}

async fn send(
    target: &ValidatedUrl,
    params: &FetchParams,
    payment_header: Option<&str>,
) -> Result<reqwest::Response, String> {
    let builder = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none());
    let builder = crate::config::apply_runtime_proxy_to_builder(builder, PROXY_SERVICE)
        .resolve_to_addrs(&target.host, &target.addrs);
    let client = builder
        .build()
        .map_err(|error| format!("x402_fetch: HTTP client unavailable: {error}"))?;
    let mut request = client.request(params.method.clone(), &target.url);
    for (name, value) in &params.headers {
        request = request.header(name, value);
    }
    if let Some(payment_header) = payment_header {
        request = request.header("PAYMENT-SIGNATURE", payment_header);
    }
    if let Some(body) = &params.body {
        request = request.body(body.clone());
    }
    request
        .send()
        .await
        .map_err(|error| format!("x402_fetch: request failed: {}", error.without_url()))
}

fn header(response: &reqwest::Response, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        response
            .headers()
            .get(*name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    })
}

async fn response_body(mut response: reqwest::Response) -> Result<(String, bool), String> {
    let mut bytes = Vec::new();
    while bytes.len() <= MAX_BODY_BYTES {
        match response.chunk().await {
            Ok(Some(chunk)) => bytes.extend_from_slice(&chunk),
            Ok(None) => break,
            Err(error) => {
                return Err(format!(
                    "x402_fetch: reading the response failed: {}",
                    error.without_url()
                ))
            }
        }
    }
    Ok(body_text(bytes, MAX_BODY_BYTES))
}

async fn into_value(
    response: reqwest::Response,
    payment: Option<FetchPayment>,
) -> Result<Value, String> {
    let status = response.status().as_u16();
    let content_type = header(&response, &["content-type"]);
    let (body, truncated) = response_body(response).await?;
    Ok(response_value(
        status,
        content_type,
        body,
        truncated,
        payment,
    ))
}

/// Whether the module ledger holds `payment_id` as settled. The host does not
/// parse settlement payloads; the module's verdict is the answer.
async fn ledger_settled(
    ctx: &DispatchCtx,
    config: &crate::config::Config,
    payment_id: &str,
) -> bool {
    match x402_service::list_payments(ctx, config, 50).await {
        Ok(response) => response.payments.iter().any(|record| {
            record.id == payment_id
                && record.status == tinywallet_bus::wire::X402PaymentStatus::Settled
        }),
        Err(error) => {
            log::warn!("[x402_fetch] settlement status unavailable: {error}");
            false
        }
    }
}

/// Run one `x402_fetch` request.
pub(super) async fn fetch(ctx: DispatchCtx, params: Map<String, Value>) -> Result<Value, String> {
    let params = parse_params(params)?;
    let target = guard_destination(&params.url).await?;
    crate::security::egress::enforce_egress(
        &crate::security::egress::EgressDescriptor::network_fetch(&target.host),
    )
    .await
    .map_err(|error| format!("x402_fetch: {error}"))?;

    log::debug!(
        "[x402_fetch] {} host={} cap={}",
        params.method,
        target.host,
        params.max_amount_atomic
    );
    let response = send(&target, &params, None).await?;
    if response.status() != reqwest::StatusCode::PAYMENT_REQUIRED {
        return into_value(response, None).await;
    }
    let Some(challenge) = header(&response, &["PAYMENT-REQUIRED", "X-PAYMENT-REQUIRED"]) else {
        // A 402 without an x402 challenge passes through unpaid, as in `http_request`.
        return into_value(response, None).await;
    };

    let config = ctx
        .load_config()
        .await
        .map_err(|error| format!("x402_fetch: {error}"))?;
    if !config.components.wallet {
        return Err(format!(
            "{}:wallet",
            crate::core::runtime::components::COMPONENT_DISABLED
        ));
    }

    let (prepared, outcome) = x402_service::prepare_capped_payment(
        &ctx,
        &config,
        target.url.clone(),
        challenge,
        params.max_amount_atomic,
    )
    .await;
    let (service, payment) = prepared.map_err(|error| prepare_error(outcome, error))?;

    let record = |status: u16, settlement: Option<String>| {
        let config = &config;
        let service = service.clone();
        let payment_id = payment.payment_id.clone();
        async move {
            match x402_service::record_payment(config, service, payment_id, status, settlement)
                .await
            {
                Ok(()) => true,
                Err(error) => {
                    log::warn!("[x402_fetch] x402 settlement recording failed: {error}");
                    false
                }
            }
        }
    };

    if let Err(error) = ensure_within_cap(payment.amount_atomic, params.max_amount_atomic) {
        // The proof is never sent; report the attempt so the module keeps its
        // own record of the reservation.
        record(reqwest::StatusCode::PAYMENT_REQUIRED.as_u16(), None).await;
        return Err(error);
    }

    log::debug!(
        "[x402_fetch] retrying with x402 payment host={} amount={} asset={}",
        target.host,
        payment.amount_atomic,
        payment.asset
    );
    let retry = match send(&target, &params, Some(&payment.payment_header)).await {
        Ok(retry) => retry,
        Err(error) => {
            // No response: the module cannot tell whether the proof settled.
            record(0, None).await;
            return Err(error);
        }
    };
    let status = retry.status();
    let settlement_response = header(&retry, &["PAYMENT-RESPONSE"]);
    let recorded = record(status.as_u16(), settlement_response.clone()).await;
    let settled = recorded
        && status.is_success()
        && settlement_response.is_some()
        && ledger_settled(&ctx, &config, &payment.payment_id).await;
    log::debug!(
        "[x402_fetch] paid retry host={} status={} settled={settled}",
        target.host,
        status.as_u16()
    );

    into_value(
        retry,
        Some(FetchPayment {
            payment_id: payment.payment_id,
            amount_atomic: payment.amount_atomic,
            asset: payment.asset,
            recipient: payment.recipient,
            network: payment.network,
            settled,
            settlement_response,
        }),
    )
    .await
}

#[cfg(test)]
#[path = "fetch_tests.rs"]
mod tests;
