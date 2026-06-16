//! Agent tool for making x402-paid HTTP requests.
//!
//! Unlike the general `http_request` tool (which silently handles 402s as a
//! fallback), this tool is purpose-built for x402 endpoints: it always expects
//! a payment challenge, surfaces pricing to the agent, and records the payment
//! in the ledger.

use async_trait::async_trait;
use base64::engine::{general_purpose::STANDARD as B64, Engine as _};
use log::debug;
use serde_json::json;
use std::time::Duration;

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};

use super::store;
use super::types::*;

const LOG_PREFIX: &str = "[tool.x402_request]";
const DEFAULT_TIMEOUT_SECS: u64 = 30;

pub struct X402RequestTool;

impl X402RequestTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for X402RequestTool {
    fn name(&self) -> &str {
        "x402_request"
    }

    fn description(&self) -> &str {
        "Make an HTTP request to an x402-payable API endpoint. Automatically handles the \
         HTTP 402 payment challenge by signing a payment (EVM EIP-3009 on Base/Ethereum, or \
         Solana SPL transfer) with the wallet and retrying with the payment proof. \
         Returns the API response after payment. Use this for x402-enabled APIs like twit.sh. \
         The wallet must be set up with USDC on the target chain."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL of the x402-payable API endpoint (e.g. https://x402.twit.sh/tweets/by/id?id=1110302988)"
                },
                "method": {
                    "type": "string",
                    "description": "HTTP method (default: GET)",
                    "default": "GET",
                    "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"]
                },
                "headers": {
                    "type": "object",
                    "description": "Optional HTTP headers as key-value pairs",
                    "default": {}
                },
                "body": {
                    "type": "string",
                    "description": "Optional request body (for POST/PUT/PATCH)"
                }
            },
            "required": ["url"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        let url = match args.get("url").and_then(|v| v.as_str()) {
            Some(u) => u.to_string(),
            None => return Ok(ToolResult::error("Missing required 'url' parameter")),
        };

        let method_str = args.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
        let method: reqwest::Method = match method_str.parse() {
            Ok(m) => m,
            Err(_) => {
                return Ok(ToolResult::error(format!(
                    "Unsupported HTTP method: {method_str}"
                )))
            }
        };

        let headers = parse_header_args(args.get("headers"));
        let body = args.get("body").and_then(|v| v.as_str()).map(String::from);

        debug!("{LOG_PREFIX} requesting {method} {url}");

        // Step 1: Initial request to get the 402 challenge
        let client = match build_client() {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Failed to build HTTP client: {e}"
                )))
            }
        };

        let initial_response =
            match send_request(&client, &method, &url, &headers, body.as_deref()).await {
                Ok(r) => r,
                Err(e) => return Ok(ToolResult::error(format!("Initial request failed: {e}"))),
            };

        // If the response is not 402, return it directly
        if initial_response.status() != reqwest::StatusCode::PAYMENT_REQUIRED {
            let status = initial_response.status().as_u16();
            debug!("{LOG_PREFIX} got {status} (not 402), returning directly");
            return format_response(initial_response, &url).await;
        }

        // Step 2: Parse the 402 challenge
        let initial_headers = initial_response.headers().clone();
        if initial_headers.get(HEADER_PAYMENT_REQUIRED).is_none()
            && initial_headers.get(HEADER_PAYMENT_REQUIRED_V1).is_none()
        {
            return Ok(ToolResult::error(
                "Server returned 402 but without a PAYMENT-REQUIRED header — not an x402 endpoint",
            ));
        }

        debug!("{LOG_PREFIX} got 402 with PAYMENT-REQUIRED header, processing payment");

        // Step 3: Build and sign the payment
        let payment_result = match super::handle_402_and_pay(&initial_headers, &url).await {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolResult::error(format!("x402 payment failed: {e}")));
            }
        };

        let amount_display = format!(
            "{:.6} USDC",
            payment_result.amount_atomic as f64 / 1_000_000.0
        );

        debug!(
            "{LOG_PREFIX} payment built: {} to {} on {} for {}",
            amount_display, payment_result.recipient, payment_result.network, url
        );

        // Record the pending payment
        let record = store::PaymentRecord {
            id: uuid::Uuid::new_v4().to_string(),
            url: payment_result.url.clone(),
            asset: payment_result.asset.clone(),
            amount_atomic: payment_result.amount_atomic,
            amount_display: amount_display.clone(),
            recipient: payment_result.recipient.clone(),
            network: payment_result.network.clone(),
            tx_signature: None,
            status: store::PaymentStatus::Pending,
            timestamp: chrono::Utc::now(),
            session_id: String::new(),
        };
        let record_id = record.id.clone();
        let _ = store::with_ledger_mut(|l| l.record_payment(record));

        // Step 4: Retry with the payment signature
        let mut retry_headers = headers.clone();
        retry_headers.push((
            HEADER_PAYMENT_SIGNATURE.to_string(),
            payment_result.header_value,
        ));

        let paid_response =
            match send_request(&client, &method, &url, &retry_headers, body.as_deref()).await {
                Ok(r) => r,
                Err(e) => {
                    let _ = store::with_ledger_mut(|l| {
                        let mut updated = store::PaymentRecord {
                            id: record_id,
                            url: url.clone(),
                            asset: payment_result.asset.clone(),
                            amount_atomic: payment_result.amount_atomic,
                            amount_display: amount_display.clone(),
                            recipient: payment_result.recipient.clone(),
                            network: payment_result.network.clone(),
                            tx_signature: None,
                            status: store::PaymentStatus::Failed,
                            timestamp: chrono::Utc::now(),
                            session_id: String::new(),
                        };
                        l.record_payment(updated);
                    });
                    return Ok(ToolResult::error(format!("x402 retry request failed: {e}")));
                }
            };

        // Step 5: Parse settlement response and update ledger
        let settled_status = if paid_response.status().is_success() {
            store::PaymentStatus::Settled
        } else {
            store::PaymentStatus::Failed
        };

        let tx_sig = paid_response
            .headers()
            .get(HEADER_PAYMENT_RESPONSE)
            .and_then(|v| v.to_str().ok())
            .and_then(|b64| B64.decode(b64).ok())
            .and_then(|bytes| serde_json::from_slice::<SettlementResponse>(&bytes).ok())
            .and_then(|r| {
                if r.success && !r.transaction.is_empty() {
                    Some(r.transaction)
                } else {
                    None
                }
            });

        let _ = store::with_ledger_mut(|l| {
            let updated = store::PaymentRecord {
                id: record_id.clone(),
                url: url.clone(),
                asset: payment_result.asset.clone(),
                amount_atomic: payment_result.amount_atomic,
                amount_display: amount_display.clone(),
                recipient: payment_result.recipient.clone(),
                network: payment_result.network.clone(),
                tx_signature: tx_sig.clone(),
                status: settled_status,
                timestamp: chrono::Utc::now(),
                session_id: String::new(),
            };
            l.record_payment(updated);
        });

        if settled_status == store::PaymentStatus::Settled {
            debug!(
                "{LOG_PREFIX} payment settled for {url} tx={:?} amount={}",
                tx_sig, amount_display
            );
        } else {
            log::warn!(
                "{LOG_PREFIX} payment failed for {url} status={}",
                paid_response.status()
            );
        }

        // Step 6: Format and return the response with payment metadata
        format_response_with_payment(
            paid_response,
            &url,
            &amount_display,
            &payment_result.network,
            tx_sig.as_deref(),
        )
        .await
    }
}

fn build_client() -> Result<reqwest::Client, reqwest::Error> {
    let builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5));
    let builder =
        crate::openhuman::config::apply_runtime_proxy_to_builder(builder, "tool.x402_request");
    builder.build()
}

fn parse_header_args(headers_val: Option<&serde_json::Value>) -> Vec<(String, String)> {
    let mut result = Vec::new();
    if let Some(obj) = headers_val.and_then(|v| v.as_object()) {
        for (key, value) in obj {
            if let Some(str_val) = value.as_str() {
                result.push((key.clone(), str_val.to_string()));
            }
        }
    }
    result
}

async fn send_request(
    client: &reqwest::Client,
    method: &reqwest::Method,
    url: &str,
    headers: &[(String, String)],
    body: Option<&str>,
) -> Result<reqwest::Response, reqwest::Error> {
    let mut request = client.request(method.clone(), url);
    for (key, value) in headers {
        request = request.header(key, value);
    }
    if let Some(body_str) = body {
        request = request.body(body_str.to_string());
    }
    request.send().await
}

async fn format_response(response: reqwest::Response, url: &str) -> anyhow::Result<ToolResult> {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let truncated = if body.len() > 50_000 {
        format!("{}…(truncated)", &body[..50_000])
    } else {
        body
    };

    Ok(ToolResult::success(format!(
        "HTTP {status} from {url}\n\n{truncated}"
    )))
}

async fn format_response_with_payment(
    response: reqwest::Response,
    url: &str,
    amount_display: &str,
    network: &str,
    tx_sig: Option<&str>,
) -> anyhow::Result<ToolResult> {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let truncated = if body.len() > 50_000 {
        format!("{}…(truncated)", &body[..50_000])
    } else {
        body
    };

    let chain_label = if network.starts_with("eip155:8453") {
        "Base"
    } else if network.starts_with("eip155:1") {
        "Ethereum"
    } else if network.starts_with("eip155:") {
        "EVM"
    } else if network.starts_with("solana:") {
        "Solana"
    } else {
        network
    };

    let tx_line = tx_sig
        .map(|sig| format!("\nTransaction: {sig}"))
        .unwrap_or_default();

    Ok(ToolResult::success(format!(
        "HTTP {status} from {url}\n\
         x402 payment: {amount_display} on {chain_label}{tx_line}\n\n\
         {truncated}"
    )))
}
