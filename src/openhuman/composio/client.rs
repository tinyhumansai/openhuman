//! Thin HTTP wrapper over the openhuman backend's
//! `/agent-integrations/composio/*` routes.
//!
//! All calls go through the shared
//! [`crate::openhuman::integrations::IntegrationClient`] so they inherit
//! the same Bearer JWT auth, timeout, envelope parsing, and proxy behavior
//! as the other backend-proxied integrations.
//!
//! Logging uses the `[composio]` grep-prefix so all sidecar output for
//! this domain can be filtered in one shot.

use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use crate::openhuman::integrations::IntegrationClient;

use super::types::{
    ComposioAuthorizeResponse, ComposioConnectionsResponse, ComposioDeleteResponse,
    ComposioExecuteResponse, ComposioToolkitsResponse, ComposioToolsResponse,
};

/// High-level client for all backend-proxied Composio operations.
#[derive(Clone)]
pub struct ComposioClient {
    inner: Arc<IntegrationClient>,
}

impl ComposioClient {
    pub fn new(inner: Arc<IntegrationClient>) -> Self {
        Self { inner }
    }

    /// Access the underlying integration client (useful for tests or for
    /// callers that need to reuse the same reqwest pool for bespoke calls).
    pub fn inner(&self) -> &Arc<IntegrationClient> {
        &self.inner
    }

    // ── Toolkits ────────────────────────────────────────────────────

    /// `GET /agent-integrations/composio/toolkits` — server-enforced
    /// allowlist of toolkits that composio calls may target.
    pub async fn list_toolkits(&self) -> Result<ComposioToolkitsResponse> {
        tracing::debug!("[composio] list_toolkits");
        self.inner
            .get::<ComposioToolkitsResponse>("/agent-integrations/composio/toolkits")
            .await
    }

    // ── Connections ─────────────────────────────────────────────────

    /// `GET /agent-integrations/composio/connections` — active connected
    /// accounts for the authenticated user, filtered to the allowlist.
    pub async fn list_connections(&self) -> Result<ComposioConnectionsResponse> {
        tracing::debug!("[composio] list_connections");
        self.inner
            .get::<ComposioConnectionsResponse>("/agent-integrations/composio/connections")
            .await
    }

    /// `POST /agent-integrations/composio/authorize` — begin an OAuth
    /// handoff for `toolkit` and return the hosted `connectUrl` the user
    /// must open in a browser.
    pub async fn authorize(&self, toolkit: &str) -> Result<ComposioAuthorizeResponse> {
        let toolkit = toolkit.trim();
        if toolkit.is_empty() {
            anyhow::bail!("composio.authorize: toolkit must not be empty");
        }
        tracing::debug!(toolkit = %toolkit, "[composio] authorize");
        let body = json!({ "toolkit": toolkit });
        self.inner
            .post::<ComposioAuthorizeResponse>("/agent-integrations/composio/authorize", &body)
            .await
    }

    /// `DELETE /agent-integrations/composio/connections/{id}`.
    ///
    /// The backend verifies that the caller owns the connection before
    /// deleting it. We call this via `POST` with a synthetic `_method`
    /// body because [`IntegrationClient`] does not currently expose a
    /// generic `delete()` — the backend accepts the method override.
    pub async fn delete_connection(&self, connection_id: &str) -> Result<ComposioDeleteResponse> {
        let connection_id = connection_id.trim();
        if connection_id.is_empty() {
            anyhow::bail!("composio.delete_connection: connectionId must not be empty");
        }
        tracing::debug!(connection_id = %connection_id, "[composio] delete_connection");
        // Fall through to the reusable raw HTTP delete helper below.
        self.raw_delete::<ComposioDeleteResponse>(&format!(
            "/agent-integrations/composio/connections/{connection_id}"
        ))
        .await
    }

    // ── Tools ───────────────────────────────────────────────────────

    /// `GET /agent-integrations/composio/tools?toolkits=<csv>` — fetch
    /// OpenAI function-calling schemas. Omit `toolkits` to get every
    /// enabled toolkit's tools.
    pub async fn list_tools(&self, toolkits: Option<&[String]>) -> Result<ComposioToolsResponse> {
        let path = match toolkits {
            Some(list) if !list.is_empty() => {
                let joined = list
                    .iter()
                    .map(|t| t.trim())
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
                    .join(",");
                format!("/agent-integrations/composio/tools?toolkits={joined}")
            }
            _ => "/agent-integrations/composio/tools".to_string(),
        };
        tracing::debug!(path = %path, "[composio] list_tools");
        self.inner.get::<ComposioToolsResponse>(&path).await
    }

    // ── Execute ─────────────────────────────────────────────────────

    /// `POST /agent-integrations/composio/execute` — run a Composio
    /// action and return the provider result + cost.
    pub async fn execute_tool(
        &self,
        tool: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<ComposioExecuteResponse> {
        let tool = tool.trim();
        if tool.is_empty() {
            anyhow::bail!("composio.execute_tool: tool slug must not be empty");
        }
        let arguments = arguments.unwrap_or(serde_json::Value::Object(Default::default()));
        tracing::debug!(tool = %tool, "[composio] execute_tool");
        let body = json!({ "tool": tool, "arguments": arguments });
        self.inner
            .post::<ComposioExecuteResponse>("/agent-integrations/composio/execute", &body)
            .await
    }

    // ── Raw DELETE ──────────────────────────────────────────────────

    /// Perform an HTTP DELETE and parse the standard backend envelope.
    ///
    /// [`IntegrationClient`] only exposes `get` / `post` today, and the
    /// composio route actually requires a DELETE. We re-implement the
    /// envelope handling here so we don't have to widen the shared
    /// client's public surface just for one caller.
    async fn raw_delete<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        #[derive(serde::Deserialize)]
        struct Envelope<T> {
            #[serde(default)]
            success: bool,
            data: Option<T>,
            #[serde(default)]
            error: Option<String>,
        }

        let url = format!("{}{}", self.inner.backend_url, path);
        tracing::debug!("[composio] DELETE {}", url);

        // Build a fresh lightweight reqwest client for this DELETE.
        // Note: this allocates a *new* connection pool — it does NOT
        // reuse the pool inside `self.inner`. To reuse the shared pool
        // we'd need to clone or expose the existing `reqwest::Client`
        // from `IntegrationClient`, which we intentionally avoid so the
        // public surface of that type doesn't widen for one caller.
        //
        // Mirror the TLS settings of the shared client
        // (`use_rustls_tls + http1_only`) so this path has the same
        // connection behaviour as the other backend calls.
        let http_client = reqwest::Client::builder()
            .use_rustls_tls()
            .http1_only()
            .timeout(std::time::Duration::from_secs(60))
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()?;

        let resp = http_client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.inner.auth_token))
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            tracing::debug!(
                "[composio] DELETE {} → {} body={}",
                url,
                status,
                &body_text[..body_text.len().min(300)]
            );
            anyhow::bail!("Backend returned {} for DELETE {}", status, url);
        }

        let envelope: Envelope<T> = resp.json().await?;
        if !envelope.success {
            let msg = envelope
                .error
                .unwrap_or_else(|| "unknown backend error".into());
            anyhow::bail!("Backend error for DELETE {}: {}", url, msg);
        }
        envelope.data.ok_or_else(|| {
            anyhow::anyhow!("Backend returned success but no data for DELETE {}", url)
        })
    }
}

/// Build a [`ComposioClient`] from the root config.
///
/// Composio is **always enabled** — there are no configuration flags
/// gating it. The backend URL and auth token come from the shared
/// core defaults (`config.api_url` / `config.api_key`) via
/// [`crate::openhuman::integrations::build_client`]. The only reason
/// this returns `None` is that the user isn't signed in yet.
pub fn build_composio_client(
    config: &crate::openhuman::config::Config,
) -> Option<ComposioClient> {
    let inner = crate::openhuman::integrations::build_client(config)?;
    Some(ComposioClient::new(inner))
}
