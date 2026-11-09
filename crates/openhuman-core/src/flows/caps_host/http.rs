//! The `HttpClient` capability for `http_request` nodes.
//!
//! Allowlist and DNS-rebind protection come from the underlying
//! `HttpRequestTool`, so this adapter inherits them. What it adds is credential
//! resolution: a `http_cred:<name>` connection_ref is resolved against the
//! encrypted credentials store and injected server-side, **after** the approval
//! gate has computed its redacted summary — so the secret never reaches the
//! approval UI, the graph, the node output, or the logs.

#![allow(unused_imports)]

use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::config::{Config, HttpRequestConfig};
use crate::flows::http_credentials::{HttpCredential, HttpCredentialsStore};
use crate::modules::tinyflows as module;
use crate::security::policy::CommandClass;
use crate::security::SecurityPolicy;
use crate::tools::http_request_tool;
use tinytools::Tool as _;

use super::{
    close_audit, enforce_current_node_tier_gate, enforce_node_tier_gate, gate_call_for_tier,
};
use super::{EngineError, Result};

/// [`HttpClient`] adapter over `HttpRequestTool`
/// (`tinyagents_harness_tools::net::network::HttpRequestTool`, built by `tools/impl/network/host.rs`). Allowlist + DNS-rebind
/// guard live inside `execute`, so this adapter gets them for free.
///
/// **B2:** also routes through the OpenHuman `ApprovalGate` before dispatch
/// (same rationale/shape as [`OpenHumanTools::invoke`] — closes the Codex P1
/// finding that flow HTTP nodes bypassed the Network approval gate).
///
/// **Phase 2 — `http_cred:<name>` resolution:** a `"http_cred:<name>"`
/// `connection_ref` is now resolved against the credentials domain's
/// [`HttpCredentialsStore`] (encrypted-at-rest bearer/basic/header templates).
/// The resolved auth header is injected **server-side** into the outbound
/// request — after the approval gate has already computed its redacted audit
/// summary — so the secret is never surfaced to the approval UI, the flow
/// engine/graph, the node's output, or the logs (only the header *name* and
/// scheme are logged; the value is redacted). A `connection_ref` that names an
/// **unknown** credential fails the request closed (`EngineError::Capability`)
/// rather than silently sending it unauthenticated.
pub struct OpenHumanHttp {
    /// The context the run acts under, handed over by whoever started it.
    pub ctx: crate::core::runtime::DispatchCtx,
    /// The run's config snapshot, used to reload its anchored autonomy tier.
    pub config: Arc<Config>,
    pub security: Arc<SecurityPolicy>,
    pub http_config: HttpRequestConfig,
    pub http_creds: Arc<HttpCredentialsStore>,
    /// The run's approval context (flow run correlation), handed to the gate
    /// with every acting dispatch.
    pub approval: crate::security::approval::ApprovalTurnContext,
}

/// Resolve the exact name returned by the module's credential parser. Trim
/// only at the host's encrypted-store lookup boundary to preserve the prior
/// credential identifier policy.
pub(crate) async fn resolve_http_credential(
    store: &HttpCredentialsStore,
    credential_name: Option<&str>,
) -> Result<Option<HttpCredential>> {
    let Some(raw_name) = credential_name else {
        return Ok(None);
    };
    let name = raw_name.trim();
    if name.is_empty() {
        return Err(EngineError::Capability(
            "http_request connection_ref has an empty http_cred name".to_owned(),
        ));
    }
    match store.get(name).await {
        Ok(Some(cred)) => {
            tracing::debug!(
                target: "flows",
                cred = %name,
                scheme = cred.scheme.as_str(),
                "[flows] http_request: resolved http_cred (secret redacted)"
            );
            Ok(Some(cred))
        }
        Ok(None) => {
            tracing::warn!(
                target: "flows",
                cred = %name,
                "[flows] http_request: connection_ref names an unknown http_cred — failing the \
                 request closed rather than sending it unauthenticated"
            );
            Err(EngineError::Capability(format!(
                "http_request connection_ref names an unknown http_cred: {name}"
            )))
        }
        Err(e) => {
            tracing::error!(
                target: "flows",
                cred = %name,
                error = %e,
                "[flows] http_request: failed to resolve http_cred from the store"
            );
            Err(EngineError::Capability(format!(
                "failed to resolve http_cred '{name}': {e}"
            )))
        }
    }
}

/// Refuse a request that would carry a stored credential over plain HTTP to a
/// remote host. Only `https` and loopback `http` are allowed; an unparsable or
/// missing URL fails closed.
pub(crate) fn refuse_credential_over_plain_http(request: &Value) -> Result<()> {
    let url = request
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let https = url::Url::parse(url).is_ok_and(|parsed| parsed.scheme() == "https");
    if https || crate::integrations::composio::client::is_loopback_http_url(url) {
        return Ok(());
    }
    tracing::warn!(
        target: "flows",
        "[flows] http_request: refusing to send a stored credential over plain http to a \
         non-loopback host"
    );
    Err(EngineError::Capability(
        "http_request refuses to send a stored credential over plain http to a non-loopback \
         host; use https"
            .to_owned(),
    ))
}

impl OpenHumanHttp {
    pub(crate) async fn request(&self, mut request: Value, conn: Option<&str>) -> Result<Value> {
        const TOOL_NAME: &str = "flows_http_request";

        // Autonomy-tier gate (Phase 2): an http_request node reaches the network,
        // so it is Network-class. A read-only run `Block`s here and never
        // dispatches; Supervised/Full fall through to the ApprovalGate below.
        // `gate_call_for_tier` is what actually performs the `Prompt` round-trip
        // — it escalates a Supervised `Prompt` decision into a forced approval
        // regardless of the flow's own `require_approval` toggle (Codex P1).
        let security =
            Arc::new(current_tier_policy(&self.ctx, &self.config, &self.security).await?);
        let tier_decision =
            enforce_node_tier_gate(&security, CommandClass::Network, "http_request").await?;

        let portability = module::portability_proxy(&self.config)
            .await
            .map_err(EngineError::Capability)?;
        let inspected: tinyflows_bus::portability::HttpRequestDetails = portability
            .call(
                tinyflows_bus::portability::methods::INSPECT_HTTP_REQUEST,
                &(tinyflows_bus::portability::InspectHttpRequest {
                    request: request.clone(),
                    connection_ref: conn.map(str::to_owned),
                },),
            )
            .await
            .map_err(|error| EngineError::Capability(error.to_string()))?;

        // Check before the approval prompt and before any dispatch, so a request
        // that can never be sent safely is neither prompted for nor attempted.
        if inspected.credential_name.is_some() {
            refuse_credential_over_plain_http(&request)?;
        }

        // The approval gate summarizes/redacts the request BEFORE any credential
        // is injected, so a stored secret never lands in the approval UI or
        // audit trail. Injection happens strictly after this point.
        let summary = inspected.redacted_summary;
        let (outcome, audit_id) = gate_call_for_tier(
            &self.ctx,
            &self.approval,
            tier_decision,
            TOOL_NAME,
            &summary,
            serde_json::to_value(&request).unwrap_or_default(),
        )
        .await;
        if let crate::security::approval::GateOutcome::Deny { reason } = outcome {
            return Err(EngineError::Capability(reason));
        }

        // Resolve `http_cred:<name>` to a stored credential and inject its auth
        // header server-side. An unknown name fails the request closed (see
        // `resolve_http_credential`) — we never send it unauthenticated.
        if let Some(cred) =
            resolve_http_credential(&self.http_creds, inspected.credential_name.as_deref()).await?
        {
            let (header, value) = cred
                .to_header()
                .map_err(|error| EngineError::Capability(error.to_string()))?;
            request = portability
                .call(
                    tinyflows_bus::portability::methods::PREPARE_HTTP_REQUEST,
                    &(tinyflows_bus::portability::PrepareHttpRequest {
                        request,
                        credential: tinyflows_bus::portability::HttpCredentialTemplate::Resolved {
                            header: header.clone(),
                            value,
                        },
                    },),
                )
                .await
                .map_err(|error| EngineError::Capability(error.to_string()))?;
            tracing::info!(
                target: "flows",
                cred = %cred.name,
                scheme = cred.scheme.as_str(),
                header = %header,
                "[flows] http_request: module injected stored credential header (value redacted)"
            );
        }

        let tool = http_request_tool(
            &self.ctx,
            security,
            self.http_config.allowed_domains.clone(),
            self.http_config.max_response_size,
            self.http_config.timeout_secs,
        );

        tracing::debug!(
            target: "flows",
            method = ?request.get("method"),
            url = ?request.get("url"),
            "[flows] http_request: dispatching outbound request"
        );

        // `request` is already `{ method, url, headers?, body? }` — the node's
        // config is the request descriptor; `HttpRequestTool::execute` reads
        // only those keys and ignores the rest (e.g. `connection_ref`,
        // `on_error`), so passing the whole config through is safe.
        let result = tool.execute(request).await;

        let outcome: Result<Value> = match result {
            Ok(result) if result.is_error => {
                // `HttpRequestTool::execute` always returns `Ok`, using
                // `is_error` to signal a failed request (non-2xx, DNS/allowlist
                // rejection, timeout, …) — surface that as a capability error
                // so the engine's `on_error`/`retry` policy can act on it.
                Err(EngineError::Capability(result.text()))
            }
            Ok(result) => Ok(json!({ "text": result.text() })),
            Err(e) => Err(EngineError::Capability(e.to_string())),
        };

        close_audit(audit_id.as_deref(), &outcome);

        outcome
    }
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
