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

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::caps::*;
use tinyflows::error::{EngineError, Result};

use super::*;
use crate::openhuman::config::{Config, HttpRequestConfig};
use crate::openhuman::security::credentials::{HttpCredential, HttpCredentialsStore};
use crate::openhuman::security::{CommandClass, SecurityPolicy};
use crate::openhuman::tools::traits::Tool as _;
use crate::openhuman::tools::HttpRequestTool;

/// [`HttpClient`] adapter over `HttpRequestTool`
/// (`src/openhuman/tools/impl/network/http_request.rs`). Allowlist + DNS-rebind
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
    pub security: Arc<SecurityPolicy>,
    pub http_config: HttpRequestConfig,
    pub http_creds: Arc<HttpCredentialsStore>,
}

/// Resolves an optional HTTP `connection_ref` to the stored credential to
/// inject. Split out as a free function (over the store, not `&self`) so the
/// resolve/fail-closed policy is unit-testable without constructing a full
/// [`OpenHumanHttp`] adapter.
///
/// - `None` conn, or a `connection_ref` whose prefix isn't `http_cred:` →
///   `Ok(None)` (no credential to inject; a non-`http_cred:` prefix is logged
///   and ignored, matching the pre-Phase-2 behavior).
/// - a `http_cred:<name>` naming a **known** credential → `Ok(Some(cred))`
///   (secret-bearing — the caller injects it server-side, never logs it).
/// - a `http_cred:<name>` naming an **unknown** credential, a malformed
///   (empty/whitespace-only) name, or a store error → `Err` — the request
///   must fail closed, never proceed unauthenticated. Distinguishing "no
///   `http_cred:` prefix at all" from "`http_cred:` prefix with a malformed
///   name" matters: [`http_cred_name`] collapses both to `None`, which would
///   otherwise let a typo'd or data-derived empty ref (e.g. `"http_cred:"`)
///   silently fall through to an unauthenticated request (Codex P2 finding).
pub(crate) fn resolve_http_credential(
    store: &HttpCredentialsStore,
    conn: Option<&str>,
) -> Result<Option<HttpCredential>> {
    let Some(conn) = conn else {
        return Ok(None);
    };
    if conn.strip_prefix("http_cred:").is_none() {
        tracing::debug!(target: "flows", %conn, "[flows] http conn: unrecognized connection_ref prefix (expected `http_cred:<name>`) — ignoring");
        return Ok(None);
    }
    let Some(name) = http_cred_name(conn) else {
        tracing::warn!(
            target: "flows",
            %conn,
            "[flows] http_request: connection_ref has the `http_cred:` prefix but no credential \
             name — failing the request closed rather than sending it unauthenticated"
        );
        return Err(EngineError::Capability(format!(
            "http_request connection_ref has a malformed http_cred name: {conn:?}"
        )));
    };

    match store.get(name) {
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

/// Merges a resolved credential's auth header into the outbound `request`'s
/// `headers` object (creating it when absent), returning the header **name**
/// that was injected for redacted logging. The header value carries the secret
/// and is placed only into the request handed to `HttpRequestTool` — it is
/// never logged or returned. An explicit stored credential wins over any inline
/// same-named header the flow author set.
pub(crate) fn inject_http_credential(request: &mut Value, cred: &HttpCredential) -> Result<String> {
    let (header_name, header_value) = cred
        .to_header()
        .map_err(|e| EngineError::Capability(e.to_string()))?;

    let obj = request.as_object_mut().ok_or_else(|| {
        EngineError::Capability("http_request config must be a JSON object".to_string())
    })?;
    let headers_entry = obj
        .entry("headers")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    // A flow author may leave `headers` unset (null) — coerce to an object so
    // the credential still injects. A non-object, non-null `headers` is a
    // malformed config we refuse rather than silently drop the credential.
    if headers_entry.is_null() {
        *headers_entry = Value::Object(serde_json::Map::new());
    }
    let headers_obj = headers_entry.as_object_mut().ok_or_else(|| {
        EngineError::Capability("http_request `headers` must be a JSON object".to_string())
    })?;
    headers_obj.insert(header_name.clone(), Value::String(header_value));

    tracing::info!(
        target: "flows",
        cred = %cred.name,
        scheme = cred.scheme.as_str(),
        header = %header_name,
        "[flows] http_request: injected stored credential header (value redacted)"
    );
    Ok(header_name)
}

#[async_trait]
impl HttpClient for OpenHumanHttp {
    async fn request(&self, mut request: Value, conn: Option<&str>) -> Result<Value> {
        const TOOL_NAME: &str = "flows_http_request";

        // Autonomy-tier gate (Phase 2): an http_request node reaches the network,
        // so it is Network-class. A read-only run `Block`s here and never
        // dispatches; Supervised/Full fall through to the ApprovalGate below.
        // `gate_call_for_tier` is what actually performs the `Prompt` round-trip
        // — it escalates a Supervised `Prompt` decision into a forced approval
        // regardless of the flow's own `require_approval` toggle (Codex P1).
        let tier_decision =
            enforce_node_tier_gate(&self.security, CommandClass::Network, "http_request")?;

        // The approval gate summarizes/redacts the request BEFORE any credential
        // is injected, so a stored secret never lands in the approval UI or
        // audit trail. Injection happens strictly after this point.
        let summary = crate::openhuman::security::approval::summarize_action(TOOL_NAME, &request);
        let redacted = crate::openhuman::security::approval::redact_args(&request);
        let (outcome, audit_id) =
            gate_call_for_tier(tier_decision, TOOL_NAME, &summary, redacted).await;
        if let crate::openhuman::security::approval::GateOutcome::Deny { reason } = outcome {
            return Err(EngineError::Capability(reason));
        }

        // Resolve `http_cred:<name>` to a stored credential and inject its auth
        // header server-side. An unknown name fails the request closed (see
        // `resolve_http_credential`) — we never send it unauthenticated.
        if let Some(cred) = resolve_http_credential(&self.http_creds, conn)? {
            inject_http_credential(&mut request, &cred)?;
        }

        let tool = HttpRequestTool::new(
            self.security.clone(),
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

        if let Some(id) = audit_id {
            if let Some(gate) = crate::openhuman::security::approval::ApprovalGate::try_global() {
                let exec = if outcome.is_ok() {
                    crate::openhuman::security::approval::ExecutionOutcome::Success
                } else {
                    crate::openhuman::security::approval::ExecutionOutcome::Failure
                };
                gate.record_execution(
                    &id,
                    exec,
                    outcome.as_ref().err().map(ToString::to_string).as_deref(),
                );
            }
        }

        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tmp: &tempfile::TempDir) -> HttpCredentialsStore {
        HttpCredentialsStore::new(tmp.path(), false)
    }

    #[test]
    fn credential_resolution_ignores_absent_and_foreign_refs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = store(&tmp);
        assert!(resolve_http_credential(&store, None).unwrap().is_none());
        assert!(resolve_http_credential(&store, Some("composio:x:y"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn credential_resolution_fails_closed_for_malformed_or_unknown_refs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = store(&tmp);
        assert!(resolve_http_credential(&store, Some("http_cred:")).is_err());
        assert!(resolve_http_credential(&store, Some("http_cred:missing")).is_err());
    }
}
