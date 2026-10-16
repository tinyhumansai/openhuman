//! Bounded OTLP/HTTP JSON export through the authenticated backend proxy.
//!
//! The span-to-OTLP conversion is upstream
//! (`tinyagents_harness::observability::trace_export::otlp`); this module only
//! resolves the backend URL and credential and posts the requests.

use serde_json::Value;
use tinyagents_harness::observability::trace_export::otlp::otlp_requests;
use tinyagents_harness::observability::trace_export::TraceSpan;

use super::langfuse::{environment_for_base, otlp_url, skip_push};
use crate::config::Config;
use crate::security::credentials::jwt::bearer_authorization_value;
use crate::security::credentials::session_support::direct_backend_credential;

/// Service key the push is proxied under (`ProxyConfig::services`).
pub(super) const OTLP_SERVICE_KEY: &str = "telemetry.otlp";

/// The push's HTTP client: the runtime proxy and platform TLS roots apply, and
/// the underlying client is cached per service key, so every push reuses one
/// connection pool.
pub(super) fn otlp_client() -> reqwest::Client {
    crate::config::build_runtime_proxy_client_with_timeouts(OTLP_SERVICE_KEY, 10, 10)
}

pub(super) async fn push_spans(config: &Config, spans: &[TraceSpan]) -> Result<(), String> {
    if spans.is_empty() {
        return Ok(());
    }
    let url = otlp_url(config);
    let environment = environment_for_base(config, &url);
    if skip_push(environment) {
        return Ok(());
    }
    if !url.starts_with("http") {
        return Err("Langfuse backend proxy URL is unavailable".to_string());
    }
    // No TinyHumans connection, or no usable credential (signed out, offline
    // local session): a configured state, so skip quietly rather than failing
    // every turn's push.
    let token = match direct_backend_credential(config, "langfuse otlp push") {
        Some(crate::security::credentials::session_support::BackendCredential::Session(token)) => {
            token
        }
        _ => return Ok(()),
    };
    // Backend traffic: stamp the host's attribution headers (product
    // identity, versions) resolved onto the config.
    let attribution = config.resolved_endpoints.attribution_header_map();
    let client = otlp_client();
    for payload in otlp_requests(spans, environment, &super::export_brand()) {
        let response = client
            .post(&url)
            .headers(attribution.clone())
            .header(
                reqwest::header::AUTHORIZATION,
                bearer_authorization_value(&token),
            )
            .timeout(std::time::Duration::from_secs(10))
            .json(&payload)
            .send()
            .await
            .map_err(|err| format!("Langfuse OTLP transport failed: {err}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("Langfuse OTLP proxy returned {status}"));
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if body["partialSuccess"]["rejectedSpans"]
            .as_u64()
            .unwrap_or(0)
            > 0
            || body["partialSuccess"]["errorMessage"]
                .as_str()
                .is_some_and(|message| !message.is_empty())
        {
            return Err("Langfuse OTLP proxy partially rejected spans".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "otlp_tests.rs"]
mod tests;
