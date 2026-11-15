//! The Langfuse usage-data push: the [`TraceSink`] this layer registers with
//! the core's agent-trace exporter.
//!
//! Spans go to the backend's Langfuse OTLP proxy — the endpoint the host
//! resolved for the current backend (`resolved_endpoints.tracing_otlp_url`),
//! authed with the live session bearer (the backend injects the real Langfuse
//! keys server-side). The deployment-environment gate
//! ([`super::langfuse_environment`]) runs before the credential is read, so
//! an unknown origin never receives the bearer.

use std::time::Duration;

use async_trait::async_trait;
use openhuman_embed::__host::config::{build_runtime_proxy_client_with_timeouts, Config};
use openhuman_embed::__host::core::observability::{export_brand, TraceSink};
use openhuman_embed::__host::credentials::backend_credential::{
    direct_backend_credential, BackendCredential,
};
use tinyagents_harness::observability::trace_export::otlp::otlp_requests;
use tinyagents_harness::observability::trace_export::TraceSpan;
use tinyagents_harness::observability::LangfuseClient;

use super::langfuse_environment::{environment_for_base, journal_push_ready, otlp_url, skip_push};

/// The id the sink registers under.
pub(crate) const SINK_ID: &str = "tinyhumans-langfuse";

/// Cap per OTLP request so a slow or hung proxy never stalls run teardown.
const PUSH_TIMEOUT: Duration = Duration::from_secs(10);

/// The backend Langfuse proxy sink.
#[derive(Debug, Default)]
pub(crate) struct LangfuseSink;

#[async_trait]
impl TraceSink for LangfuseSink {
    async fn ready(&self, config: &Config) -> bool {
        journal_push_ready(config).await
    }

    async fn push(&self, config: &Config, spans: &[TraceSpan]) -> Result<(), String> {
        push_spans(config, spans).await
    }
}

pub(crate) fn reject_partial_success(response: &serde_json::Value) -> Result<(), String> {
    if response["partialSuccess"]["errorMessage"]
        .as_str()
        .is_some_and(|message| !message.is_empty())
    {
        Err("Langfuse OTLP proxy reported partial success".to_string())
    } else {
        Ok(())
    }
}

/// Push `spans` to the Langfuse OTLP proxy. A skipped environment or a missing
/// session is `Ok(())`: skipping is the configured outcome, not a fault.
pub(crate) async fn push_spans(config: &Config, spans: &[TraceSpan]) -> Result<(), String> {
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
    let token = match direct_backend_credential(config, "langfuse otlp push").await {
        Some(BackendCredential::Session(token)) => token,
        _ => return Ok(()),
    };
    let client = LangfuseClient::proxy_with_transport(
        url,
        token,
        build_runtime_proxy_client_with_timeouts("telemetry.otlp", 10, 10),
        config.resolved_endpoints.attribution_header_map(),
    )
    .map_err(|_| "Langfuse OTLP proxy client could not be configured".to_string())?;
    for payload in otlp_requests(spans, environment, &export_brand()) {
        let response = tokio::time::timeout(PUSH_TIMEOUT, client.send_otlp(payload))
            .await
            .map_err(|_| "Langfuse OTLP proxy request timed out".to_string())?
            .map_err(|_| "Langfuse OTLP proxy request failed".to_string())?;
        reject_partial_success(&response)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "langfuse_sink_tests.rs"]
mod tests;
