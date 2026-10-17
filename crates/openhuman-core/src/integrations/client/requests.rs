//! Outbound request plumbing for [`IntegrationClient`]: route guards, egress
//! disclosure/enforcement, budget gating, and the JSON verb methods
//! (`post`/`get`/`patch`/`delete`/`upload_multipart`).

use super::construct::IntegrationClient;
use crate::security::approval::ApprovalTurnContext;

pub(super) fn managed_budget_applies_to_path(path: &str) -> bool {
    path != "/agent-integrations/pricing" && path.starts_with("/agent-integrations/")
}

fn reject_privileged_backend_path(method: &str, path: &str) -> anyhow::Result<()> {
    let route = path.split('?').next().unwrap_or(path);
    if route.split('/').any(|segment| {
        segment.eq_ignore_ascii_case("webhooks") || segment.eq_ignore_ascii_case("admin")
    }) {
        anyhow::bail!(
            "route is intentionally not exposed by the SDK: {} {}",
            method,
            route
        );
    }
    Ok(())
}

/// Build the egress descriptor for a managed-backend round-trip. The query
/// string is stripped so only the endpoint (the "to where") is described, never
/// any data carried in the query.
pub(super) fn backend_egress_descriptor(path: &str) -> crate::security::egress::EgressDescriptor {
    let endpoint = path.split('?').next().unwrap_or(path);
    crate::security::egress::EgressDescriptor::integration(endpoint)
}

/// Egress spine (privacy epic S2, #4436): disclose an OpenHuman managed-backend
/// round-trip before it leaves the device. Fire-and-forget — never fails the
/// caller.
pub(super) fn emit_backend_egress(approval: &ApprovalTurnContext, path: &str) {
    crate::security::egress::emit_external_transfer(
        approval.chat.as_ref(),
        backend_egress_descriptor(path),
    );
}

/// Local-only enforcement (privacy epic S7, #4441): refuse a managed-backend
/// round-trip that ships user data when the live policy is `LocalOnly`. Returns
/// `Ok(())` for control-plane paths (session / team / billing / integration
/// connection-management + catalog) even under `LocalOnly` — blocking those
/// would break sign-in and the Connections UI for no privacy benefit. See
/// [`is_control_plane`](crate::security::egress). Call before
/// [`emit_backend_egress`] so a blocked call is neither disclosed nor sent.
pub(super) fn enforce_backend_egress(path: &str) -> anyhow::Result<()> {
    crate::security::egress::enforce_egress(&backend_egress_descriptor(path))
}

impl IntegrationClient {
    /// The process backend transport, or the typed "backend unavailable"
    /// error already classified for this call.
    pub(super) fn transport(
        &self,
        method: &str,
        path: &str,
        url: &str,
    ) -> anyhow::Result<std::sync::Arc<dyn crate::backend::transport::BackendTransport>> {
        crate::backend::transport::resolve_backend_transport()
            .map_err(|error| self.map_transport_error(error, method, path, url))
    }

    /// Describe one `/agent-integrations/*` round-trip for the transport: the
    /// client's credential (session JWT as bearer, API key as `x-api-key`), no envelope unwrapping (this client parses
    /// the `{success,data}` envelope itself so its error classification sees
    /// the raw shape).
    pub(super) fn backend_request<'a>(
        &'a self,
        method: reqwest::Method,
        path: &'a str,
        body: Option<&'a serde_json::Value>,
    ) -> crate::backend::transport::BackendRequest<'a> {
        crate::backend::transport::BackendRequest {
            profile: crate::backend::transport::TransportProfile::Integrations,
            base_url: &self.backend_url,
            method,
            path,
            query: &[],
            body,
            credential: Some(&self.credential),
            unwrap_envelope: false,
        }
    }

    pub(super) async fn ensure_budget_available(&self, path: &str) -> anyhow::Result<()> {
        if !managed_budget_applies_to_path(path) {
            return Ok(());
        }
        if let Some(config) = &self.budget_config {
            if super::budget_gate::managed_tool_budget_exhausted(config).await {
                anyhow::bail!(
                    "Managed cloud tools are disabled because your OpenHuman AI credits are exhausted. Add credits or route the task to user-supplied providers."
                );
            }
        }
        Ok(())
    }

    /// POST JSON to a backend endpoint and parse the response `data` field.
    pub async fn post<T: serde::de::DeserializeOwned>(
        &self,
        approval: &ApprovalTurnContext,
        path: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<T> {
        self.request_json(approval, reqwest::Method::POST, path, Some(body))
            .await
    }

    /// GET from a backend endpoint and parse the response `data` field.
    pub async fn get<T: serde::de::DeserializeOwned>(
        &self,
        approval: &ApprovalTurnContext,
        path: &str,
    ) -> anyhow::Result<T> {
        self.request_json(approval, reqwest::Method::GET, path, None)
            .await
    }

    pub(super) async fn request_json<T: serde::de::DeserializeOwned>(
        &self,
        approval: &ApprovalTurnContext,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> anyhow::Result<T> {
        self.validate_credential_endpoint()?;
        reject_privileged_backend_path(method.as_str(), path)?;
        enforce_backend_egress(path)?;
        emit_backend_egress(approval, path);
        self.ensure_budget_available(path).await?;
        let url = crate::util::url::join_url(&self.backend_url, path);
        let method_name = method.as_str().to_ascii_lowercase();
        tracing::debug!("[integrations] {} {}", method.as_str(), url);
        let value = self
            .transport(&method_name, path, &url)?
            .send_json(self.backend_request(method, path, body))
            .await
            .map_err(|error| self.map_transport_error(error, &method_name, path, &url))?;
        Self::parse_envelope(&method_name, path, &url, value)
    }

    /// PATCH JSON to a backend endpoint and parse the response `data` field.
    /// Mirrors [`Self::post`] (auth header, 401 → session-expiry, error
    /// classification).
    pub async fn patch<T: serde::de::DeserializeOwned>(
        &self,
        approval: &ApprovalTurnContext,
        path: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<T> {
        self.request_json(approval, reqwest::Method::PATCH, path, Some(body))
            .await
    }

    /// DELETE a backend resource and parse the response `data` field.
    /// Mirrors [`Self::post`] (auth header, 401 → session-expiry, error
    /// classification).
    pub async fn delete<T: serde::de::DeserializeOwned>(
        &self,
        approval: &ApprovalTurnContext,
        path: &str,
    ) -> anyhow::Result<T> {
        self.request_json(approval, reqwest::Method::DELETE, path, None)
            .await
    }

    /// POST a `multipart/form-data` body to a backend endpoint and parse the
    /// response `data` field. Mirrors [`Self::post`] URL building, Bearer
    /// auth, 401 → session-expiry handling and error classification; the
    /// content type is set by reqwest from the form boundary.
    pub async fn upload_multipart<T: serde::de::DeserializeOwned>(
        &self,
        approval: &ApprovalTurnContext,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> anyhow::Result<T> {
        self.validate_credential_endpoint()?;
        reject_privileged_backend_path("POST", path)?;
        enforce_backend_egress(path)?;
        emit_backend_egress(approval, path);
        self.ensure_budget_available(path).await?;
        let url = crate::util::url::join_url(&self.backend_url, path);
        tracing::debug!("[integrations] POST(multipart) {}", url);

        let value = self
            .transport("post_multipart", path, &url)?
            .send_multipart(
                self.backend_request(reqwest::Method::POST, path, None),
                form,
            )
            .await
            .map_err(|error| self.map_transport_error(error, "post_multipart", path, &url))?;
        // The transport unwraps successful `{success,data}` responses. Preserve
        // compatibility with endpoints that return their payload directly,
        // while still recognizing a `success:false` envelope.
        if value.get("success") == Some(&serde_json::Value::Bool(false)) {
            return Self::parse_envelope("post_multipart", path, &url, value);
        }
        Ok(serde_json::from_value(value)?)
    }
}
