//! Host wiring for the network tools that live in `tinytools_std::network`.
//!
//! The tools take their limits' fallbacks, their HTML engine and their 402
//! handler from the host. This file is where OpenHuman supplies them: the
//! `[http_request]` defaults, TinyJuice for page extraction, and (with the
//! `web3` feature) the x402 payment flow.

use std::sync::Arc;

use tinytools_std::network::{
    AsyncHtmlExtractor, HttpLimits, HttpRequestTool, NetGate, WebFetchTool,
};

use crate::config::HttpRequestConfig;
use crate::inference::tokenjuice::focus::{summary_focus_property, SUMMARY_FOCUS_ARG};

/// The fallbacks for a `0` limit, from the `[http_request]` schema defaults, so
/// the tools, the schema and the 5→6 migration share one source.
fn http_limits() -> HttpLimits {
    let defaults = HttpRequestConfig::default();
    HttpLimits {
        max_response_size: defaults.max_response_size,
        timeout_secs: defaults.timeout_secs,
    }
}

/// HTML detection and extraction execute in the lazily loaded TinyJuice module.
#[derive(Debug)]
struct TinyJuiceHtml;

#[async_trait::async_trait]
impl AsyncHtmlExtractor for TinyJuiceHtml {
    async fn looks_like_html(&self, body: &str) -> anyhow::Result<bool> {
        let kind = crate::inference::tokenjuice::detect(
            body.to_string(),
            crate::inference::tokenjuice::types::ContentHint::default(),
        )
        .await
        .map_err(anyhow::Error::msg)?;
        Ok(kind == "html")
    }

    async fn to_markdown(&self, html: &str) -> anyhow::Result<String> {
        crate::inference::tokenjuice::extract_html(html.to_string())
            .await
            .map_err(anyhow::Error::msg)
    }
}

/// `http_request`, with the x402 402-retry wired in when `web3` is compiled.
pub fn http_request_tool(
    gate: Arc<dyn NetGate>,
    allowed_domains: Vec<String>,
    max_response_size: usize,
    timeout_secs: u64,
) -> HttpRequestTool {
    let tool = HttpRequestTool::new(
        gate,
        allowed_domains,
        max_response_size,
        timeout_secs,
        http_limits(),
    );
    #[cfg(feature = "web3")]
    let tool = tool.with_payment_hook(Arc::new(x402::X402PaymentHook));
    tool
}

/// `web_fetch`, converting HTML through TinyJuice and opting into TinyJuice's
/// optional `summary_focus` argument.
pub fn web_fetch_tool(
    gate: Arc<dyn NetGate>,
    allowed_domains: Vec<String>,
    max_bytes: Option<usize>,
    timeout_secs: Option<u64>,
) -> WebFetchTool {
    WebFetchTool::new_async(
        gate,
        allowed_domains,
        max_bytes,
        timeout_secs,
        http_limits(),
        Arc::new(TinyJuiceHtml),
    )
    // Opts into a caller-steered summary; see `tokenjuice::focus`.
    .with_schema_property(SUMMARY_FOCUS_ARG, summary_focus_property())
}

#[cfg(feature = "web3")]
mod x402 {
    //! x402 machine payment for `http_request`'s `402 Payment Required` retry.

    use async_trait::async_trait;
    use base64::engine::Engine as _;
    use tinytools_std::network::{PaymentAttempt, PaymentHook, PaymentOutcome};

    use crate::web3::x402;

    #[derive(Debug)]
    pub(super) struct X402PaymentHook;

    #[async_trait]
    impl PaymentHook for X402PaymentHook {
        async fn pay(
            &self,
            url: &str,
            response_headers: &reqwest::header::HeaderMap,
        ) -> Result<PaymentAttempt, String> {
            let payment_result = x402::handle_402_and_pay(response_headers, url)
                .await
                .map_err(|e| format!("x402 payment failed: {e}"))?;

            // Stamped with the ledger's session and the active chat thread, exactly as
            // the `x402_request` tool stamps its own.
            let record = x402::pending_record(&payment_result);
            let record_id = record.id.clone();
            let _ = x402::store::with_ledger_mut(|l| l.record_payment(record));

            log::debug!(
                "[tool.http_request] retrying with x402 payment header amount={} asset={}",
                payment_result.amount_atomic,
                payment_result.asset
            );

            let url = url.to_string();
            Ok(PaymentAttempt {
                headers: vec![("PAYMENT-SIGNATURE".to_string(), payment_result.header_value)],
                settle: Box::new(move |outcome| settle(&record_id, &url, &outcome)),
            })
        }
    }

    /// Write the retry's result back onto the ledger entry the payment opened.
    fn settle(record_id: &str, url: &str, outcome: &PaymentOutcome) {
        let settled_status = if outcome.success {
            x402::PaymentStatus::Settled
        } else {
            x402::PaymentStatus::Failed
        };
        let tx_sig = outcome
            .payment_response
            .as_deref()
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
                outcome.status
            );
        }
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
