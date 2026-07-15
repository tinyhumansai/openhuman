use super::url_guard::{normalize_allowed_domains, validate_url_with_dns_check};
use crate::openhuman::config::HttpRequestConfig;
use crate::openhuman::security::{CommandClass, GateDecision, SecurityPolicy};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use base64::engine::Engine as _;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Build the egress descriptor for an outbound HTTP request (privacy epic S2,
/// #4436). The host is the destination; a request body carries tool arguments
/// and any custom headers (e.g. an `Authorization` token) are metadata that
/// also leaves the device — so a header-only call is not under-reported as
/// URL-only (codex P2, PR #4812). Pure over its inputs so it is unit-testable
/// off the network path.
fn network_egress_descriptor(
    url: &str,
    has_body: bool,
    has_headers: bool,
) -> crate::openhuman::security::egress::EgressDescriptor {
    use crate::openhuman::security::egress::{DataKind, EgressDescriptor};
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    let mut desc = EgressDescriptor::network_fetch(host);
    if has_body {
        desc = desc.with_data_kind(DataKind::ToolArguments);
    }
    if has_headers {
        desc = desc.with_data_kind(DataKind::Metadata);
    }
    desc
}

/// HTTP request tool for API interactions.
/// Supports GET, POST, PUT, DELETE methods with configurable security.
pub struct HttpRequestTool {
    security: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
    max_response_size: usize,
    timeout_secs: u64,
}

impl HttpRequestTool {
    pub fn new(
        security: Arc<SecurityPolicy>,
        allowed_domains: Vec<String>,
        max_response_size: usize,
        timeout_secs: u64,
    ) -> Self {
        // Treat `0` as "use default": a 0-byte cap or 0-second timeout is never
        // a meaningful limit, only a footgun (see migration 5→6). Pull the
        // fallbacks from `HttpRequestConfig::default()` so the tool, the schema
        // default, and the migration share one source and can't drift. A `0`
        // here means a stale/invalid config slipped past the migration, so
        // surface it with a stable, grep-friendly, non-sensitive log line.
        let defaults = HttpRequestConfig::default();
        let max_response_size = if max_response_size == 0 {
            log::warn!(
                "[tool.http_request] coercing invalid limit field=max_response_size \
                 from=0 to={} (stale/invalid config — see migration 5→6)",
                defaults.max_response_size
            );
            defaults.max_response_size
        } else {
            max_response_size
        };
        let timeout_secs = if timeout_secs == 0 {
            log::warn!(
                "[tool.http_request] coercing invalid limit field=timeout_secs \
                 from=0 to={} (stale/invalid config — see migration 5→6)",
                defaults.timeout_secs
            );
            defaults.timeout_secs
        } else {
            timeout_secs
        };
        Self {
            security,
            allowed_domains: normalize_allowed_domains(allowed_domains),
            max_response_size,
            timeout_secs,
        }
    }

    async fn validate_url(&self, raw_url: &str) -> anyhow::Result<String> {
        validate_url_with_dns_check(raw_url, &self.allowed_domains).await
    }

    fn validate_method(&self, method: &str) -> anyhow::Result<reqwest::Method> {
        match method.to_uppercase().as_str() {
            "GET" => Ok(reqwest::Method::GET),
            "POST" => Ok(reqwest::Method::POST),
            "PUT" => Ok(reqwest::Method::PUT),
            "DELETE" => Ok(reqwest::Method::DELETE),
            "PATCH" => Ok(reqwest::Method::PATCH),
            "HEAD" => Ok(reqwest::Method::HEAD),
            "OPTIONS" => Ok(reqwest::Method::OPTIONS),
            _ => anyhow::bail!("Unsupported HTTP method: {method}. Supported: GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS"),
        }
    }

    fn parse_headers(&self, headers: &serde_json::Value) -> Vec<(String, String)> {
        let mut result = Vec::new();
        if let Some(obj) = headers.as_object() {
            for (key, value) in obj {
                if let Some(str_val) = value.as_str() {
                    result.push((key.clone(), str_val.to_string()));
                }
            }
        }
        result
    }

    fn redact_headers_for_display(headers: &[(String, String)]) -> Vec<(String, String)> {
        headers
            .iter()
            .map(|(key, value)| {
                let lower = key.to_lowercase();
                let is_sensitive = lower.contains("authorization")
                    || lower.contains("api-key")
                    || lower.contains("apikey")
                    || lower.contains("token")
                    || lower.contains("secret");
                if is_sensitive {
                    (key.clone(), "***REDACTED***".into())
                } else {
                    (key.clone(), value.clone())
                }
            })
            .collect()
    }

    async fn execute_request(
        &self,
        url: &str,
        method: reqwest::Method,
        headers: Vec<(String, String)>,
        body: Option<&str>,
    ) -> anyhow::Result<reqwest::Response> {
        let builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none());
        let builder =
            crate::openhuman::config::apply_runtime_proxy_to_builder(builder, "tool.http_request");
        let client = builder.build()?;

        let mut request = client.request(method, url);

        for (key, value) in headers {
            request = request.header(&key, &value);
        }

        if let Some(body_str) = body {
            request = request.body(body_str.to_string());
        }

        Ok(request.send().await?)
    }

    async fn handle_x402_payment(
        &self,
        _initial_response: reqwest::Response,
        url: &str,
        method: reqwest::Method,
        headers: Vec<(String, String)>,
        body: Option<&str>,
    ) -> Result<reqwest::Response, String> {
        use crate::openhuman::x402;

        log::debug!("[tool.http_request] 402 received with PAYMENT-REQUIRED, attempting x402 payment for {url}");

        let initial_headers = _initial_response.headers().clone();
        let payment_result = x402::handle_402_and_pay(&initial_headers, url)
            .await
            .map_err(|e| format!("x402 payment failed: {e}"))?;

        let record = x402::PaymentRecord {
            id: uuid::Uuid::new_v4().to_string(),
            url: payment_result.url.clone(),
            asset: payment_result.asset.clone(),
            amount_atomic: payment_result.amount_atomic,
            amount_display: format!(
                "{:.6} USDC",
                payment_result.amount_atomic as f64 / 1_000_000.0
            ),
            recipient: payment_result.recipient.clone(),
            network: payment_result.network.clone(),
            tx_signature: None,
            status: x402::PaymentStatus::Pending,
            timestamp: chrono::Utc::now(),
            session_id: String::new(),
        };

        let record_id = record.id.clone();
        let _ = x402::store::with_ledger_mut(|l| l.record_payment(record));

        log::debug!(
            "[tool.http_request] retrying with x402 payment header amount={} asset={}",
            payment_result.amount_atomic,
            payment_result.asset
        );

        let mut retry_headers = headers;
        retry_headers.push(("PAYMENT-SIGNATURE".to_string(), payment_result.header_value));

        let response = self
            .execute_request(url, method, retry_headers, body)
            .await
            .map_err(|e| format!("x402 retry request failed: {e}"))?;

        let settled_status = if response.status().is_success() {
            x402::PaymentStatus::Settled
        } else {
            x402::PaymentStatus::Failed
        };
        let tx_sig = response
            .headers()
            .get("PAYMENT-RESPONSE")
            .and_then(|v| v.to_str().ok())
            .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
            .and_then(|bytes| serde_json::from_slice::<x402::SettlementResponse>(&bytes).ok())
            .and_then(|r| {
                if r.success && !r.transaction.is_empty() {
                    Some(r.transaction)
                } else {
                    None
                }
            });

        let _ = x402::store::with_ledger_mut(|l| {
            if let Some(rec) = l
                .recent_payments(100)
                .into_iter()
                .find(|r| r.id == record_id)
            {
                let mut updated = rec;
                updated.status = settled_status;
                updated.tx_signature = tx_sig.clone();
                l.record_payment(updated);
            }
        });

        if settled_status == x402::PaymentStatus::Settled {
            log::debug!(
                "[tool.http_request] x402 payment settled for {url} tx={:?}",
                tx_sig
            );
        } else {
            log::warn!(
                "[tool.http_request] x402 payment retry returned status {}",
                response.status()
            );
        }

        Ok(response)
    }

    async fn format_response(&self, response: reqwest::Response) -> anyhow::Result<ToolResult> {
        let status = response.status();
        let status_code = status.as_u16();

        let response_headers = response.headers().iter();
        let headers_text = response_headers
            .map(|(k, _)| {
                let is_sensitive = k.as_str().to_lowercase().contains("set-cookie");
                if is_sensitive {
                    format!("{}: ***REDACTED***", k.as_str())
                } else {
                    format!("{}: {:?}", k.as_str(), k.as_str())
                }
            })
            .collect::<Vec<_>>()
            .join(", ");

        let response_text = match response.text().await {
            Ok(text) => self.truncate_response(&text),
            Err(e) => format!("[Failed to read response body: {e}]"),
        };

        let output = format!(
            "Status: {} {}\nResponse Headers: {}\n\nResponse Body:\n{}",
            status_code,
            status.canonical_reason().unwrap_or("Unknown"),
            headers_text,
            response_text
        );

        if status.is_success() {
            Ok(ToolResult::success(output))
        } else {
            Ok(ToolResult::error(format!("HTTP {}", status_code)))
        }
    }

    fn truncate_response(&self, text: &str) -> String {
        if text.len() > self.max_response_size {
            let mut truncated = text
                .chars()
                .take(self.max_response_size)
                .collect::<String>();
            truncated.push_str("\n\n... [Response truncated due to size limit] ...");
            truncated
        } else {
            text.to_string()
        }
    }
}

#[async_trait]
impl Tool for HttpRequestTool {
    fn name(&self) -> &str {
        "http_request"
    }

    fn description(&self) -> &str {
        "Make HTTP requests to external APIs. Supports GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS methods. \
        Security constraints: allowlist-only domains, no local/private hosts, configurable timeout and response size limits."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "HTTP or HTTPS URL to request"
                },
                "method": {
                    "type": "string",
                    "description": "HTTP method (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS)",
                    "default": "GET"
                },
                "headers": {
                    "type": "object",
                    "description": "Optional HTTP headers as key-value pairs (e.g., {\"Authorization\": \"Bearer token\", \"Content-Type\": \"application/json\"})",
                    "default": {}
                },
                "body": {
                    "type": "string",
                    "description": "Optional request body (for POST, PUT, PATCH requests)"
                }
            },
            "required": ["url"]
        })
    }

    /// Rich HTTP semantics (methods, headers, request bodies, and x402 retry)
    /// are the same Network-class risk as `curl`: read-only autonomy is blocked
    /// in `execute`, and supervised/full tiers route through ApprovalGate.
    fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
        self.security.gate_decision(CommandClass::Network) == GateDecision::Prompt
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

        let method_str = args.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
        let headers_val = args.get("headers").cloned().unwrap_or(json!({}));
        let body = args.get("body").and_then(|v| v.as_str());

        if !self.security.can_act() {
            return Ok(ToolResult::error(
                "[policy-blocked] Action blocked: autonomy is read-only",
            ));
        }

        if !self.security.record_action() {
            return Ok(ToolResult::error("Action blocked: rate limit exceeded"));
        }

        // Local-only enforcement (privacy epic S7, #4441): mirror the read-only
        // `can_act()` deny above — under LocalOnly, refuse the outbound request
        // before URL validation / DNS so nothing (not even a DNS lookup for the
        // host) leaves the device. The post-validation `emit_external_transfer`
        // below stays the S2 disclosure point for permitted requests.
        {
            let host = reqwest::Url::parse(url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_else(|| "unknown".to_string());
            if let Some(msg) = crate::openhuman::security::egress::local_only_tool_block(
                &crate::openhuman::security::egress::EgressDescriptor::network_fetch(host),
            ) {
                return Ok(ToolResult::error(msg));
            }
        }

        let url = match self.validate_url(url).await {
            Ok(v) => v,
            Err(e) => return Ok(ToolResult::error(e.to_string())),
        };

        // Egress spine (privacy epic S2, #4436): an agent-driven HTTP request to
        // an allowlisted host leaves the device — disclose the destination and
        // everything that rides with it (body + custom headers) before the
        // round-trip.
        {
            let has_headers = headers_val
                .as_object()
                .map(|h| !h.is_empty())
                .unwrap_or(false);
            crate::openhuman::security::egress::emit_external_transfer(network_egress_descriptor(
                &url,
                body.is_some(),
                has_headers,
            ));
        }

        let method = match self.validate_method(method_str) {
            Ok(m) => m,
            Err(e) => return Ok(ToolResult::error(e.to_string())),
        };

        let request_headers = self.parse_headers(&headers_val);

        let response = match self
            .execute_request(&url, method.clone(), request_headers.clone(), body)
            .await
        {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::error(format!("HTTP request failed: {e}"))),
        };

        // x402: if the server returns 402 with a PAYMENT-REQUIRED header,
        // attempt to pay using the wallet's Solana key and retry.
        let response = if response.status() == reqwest::StatusCode::PAYMENT_REQUIRED
            && (response.headers().get("PAYMENT-REQUIRED").is_some()
                || response.headers().get("X-PAYMENT-REQUIRED").is_some())
        {
            match self
                .handle_x402_payment(response, &url, method, request_headers, body)
                .await
            {
                Ok(paid_response) => paid_response,
                Err(msg) => return Ok(ToolResult::error(msg)),
            }
        } else {
            response
        };

        self.format_response(response).await
    }
}

#[cfg(test)]
#[path = "http_request_tests.rs"]
mod tests;
