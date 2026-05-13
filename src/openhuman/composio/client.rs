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
    ComposioActiveTriggersResponse, ComposioAuthorizeResponse, ComposioAvailableTriggersResponse,
    ComposioConnectionsResponse, ComposioCreateTriggerResponse, ComposioDeleteResponse,
    ComposioDisableTriggerResponse, ComposioEnableTriggerResponse, ComposioExecuteResponse,
    ComposioGithubReposResponse, ComposioToolkitsResponse, ComposioToolsResponse,
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
    ///
    /// `extra_params` is an optional JSON object whose key/value pairs are
    /// merged into the request body. Some toolkits (e.g. `whatsapp`) require
    /// additional fields (e.g. `waba_id`) that Composio will reject the
    /// authorization without.
    pub async fn authorize(
        &self,
        toolkit: &str,
        extra_params: Option<serde_json::Value>,
    ) -> Result<ComposioAuthorizeResponse> {
        let toolkit = toolkit.trim();
        if toolkit.is_empty() {
            anyhow::bail!("composio.authorize: toolkit must not be empty");
        }
        tracing::debug!(toolkit = %toolkit, has_extra_params = extra_params.is_some(), "[composio] authorize");
        let mut body = serde_json::json!({ "toolkit": toolkit });
        if let Some(extra) = extra_params {
            const RESERVED: &[&str] = &["toolkit", "toolkit_version", "auth", "client_id"];
            let extra_obj = extra.as_object().ok_or_else(|| {
                anyhow::anyhow!("composio.authorize: extra_params must be a JSON object")
            })?;
            let obj = body.as_object_mut().ok_or_else(|| {
                anyhow::anyhow!("composio.authorize: internal payload must be an object")
            })?;
            for (k, v) in extra_obj {
                if RESERVED.contains(&k.as_str()) {
                    anyhow::bail!(
                        "composio.authorize: extra_params cannot override reserved key '{k}'"
                    );
                }
                obj.insert(k.clone(), v.clone());
            }
        }
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

    /// `GET /agent-integrations/composio/github/repos` — list repositories
    /// available via the user's authorized GitHub connected account.
    pub async fn list_github_repos(
        &self,
        connection_id: Option<&str>,
    ) -> Result<ComposioGithubReposResponse> {
        let path = match connection_id.map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) => format!("/agent-integrations/composio/github/repos?connectionId={id}"),
            None => "/agent-integrations/composio/github/repos".to_string(),
        };
        tracing::debug!(path = %path, "[composio] list_github_repos");
        self.inner.get::<ComposioGithubReposResponse>(&path).await
    }

    /// `POST /agent-integrations/composio/triggers` — create a trigger
    /// instance for the authenticated user.
    pub async fn create_trigger(
        &self,
        slug: &str,
        connection_id: Option<&str>,
        trigger_config: Option<serde_json::Value>,
    ) -> Result<ComposioCreateTriggerResponse> {
        let slug = slug.trim();
        if slug.is_empty() {
            anyhow::bail!("composio.create_trigger: slug must not be empty");
        }
        let mut body = json!({ "slug": slug });
        if let Some(connection_id) = connection_id.map(str::trim).filter(|id| !id.is_empty()) {
            body["connectionId"] = json!(connection_id);
        }
        if let Some(config) = trigger_config {
            body["triggerConfig"] = config;
        }
        tracing::debug!(slug = %slug, "[composio] create_trigger");
        self.inner
            .post::<ComposioCreateTriggerResponse>("/agent-integrations/composio/triggers", &body)
            .await
    }

    // ── Trigger management (PR #671) ────────────────────────────────

    /// `GET /agent-integrations/composio/triggers/available` — catalog of
    /// triggers the user could enable for a toolkit. For GitHub the
    /// backend fans out into per-repo entries scoped by `connection_id`.
    pub async fn list_available_triggers(
        &self,
        toolkit: &str,
        connection_id: Option<&str>,
    ) -> Result<ComposioAvailableTriggersResponse> {
        let toolkit = toolkit.trim();
        if toolkit.is_empty() {
            anyhow::bail!("composio.list_available_triggers: toolkit must not be empty");
        }
        let toolkit_q = urlencoding::encode(toolkit);
        let path = match connection_id.map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) => format!(
                "/agent-integrations/composio/triggers/available?toolkit={toolkit_q}&connectionId={}",
                urlencoding::encode(id)
            ),
            None => format!(
                "/agent-integrations/composio/triggers/available?toolkit={toolkit_q}"
            ),
        };
        tracing::debug!(path = %path, "[composio] list_available_triggers");
        self.inner
            .get::<ComposioAvailableTriggersResponse>(&path)
            .await
    }

    /// `GET /agent-integrations/composio/triggers` — currently enabled
    /// triggers for the user, optionally filtered to a toolkit.
    pub async fn list_active_triggers(
        &self,
        toolkit: Option<&str>,
    ) -> Result<ComposioActiveTriggersResponse> {
        let path = match toolkit.map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => format!(
                "/agent-integrations/composio/triggers?toolkit={}",
                urlencoding::encode(t)
            ),
            None => "/agent-integrations/composio/triggers".to_string(),
        };
        tracing::debug!(path = %path, "[composio] list_active_triggers");
        self.inner
            .get::<ComposioActiveTriggersResponse>(&path)
            .await
    }

    /// `POST /agent-integrations/composio/triggers` — enable a single
    /// trigger on a connection the caller owns.
    pub async fn enable_trigger(
        &self,
        connection_id: &str,
        slug: &str,
        trigger_config: Option<serde_json::Value>,
    ) -> Result<ComposioEnableTriggerResponse> {
        let connection_id = connection_id.trim();
        let slug = slug.trim();
        if connection_id.is_empty() {
            anyhow::bail!("composio.enable_trigger: connectionId must not be empty");
        }
        if slug.is_empty() {
            anyhow::bail!("composio.enable_trigger: slug must not be empty");
        }
        let mut body = json!({ "connectionId": connection_id, "slug": slug });
        if let Some(config) = trigger_config {
            body["triggerConfig"] = config;
        }
        tracing::debug!(slug = %slug, connection_id = %connection_id, "[composio] enable_trigger");
        self.inner
            .post::<ComposioEnableTriggerResponse>("/agent-integrations/composio/triggers", &body)
            .await
    }

    /// `DELETE /agent-integrations/composio/triggers/:triggerId`.
    pub async fn disable_trigger(
        &self,
        trigger_id: &str,
    ) -> Result<ComposioDisableTriggerResponse> {
        let trigger_id = trigger_id.trim();
        if trigger_id.is_empty() {
            anyhow::bail!("composio.disable_trigger: triggerId must not be empty");
        }
        tracing::debug!(trigger_id = %trigger_id, "[composio] disable_trigger");
        self.raw_delete::<ComposioDisableTriggerResponse>(&format!(
            "/agent-integrations/composio/triggers/{}",
            urlencoding::encode(trigger_id)
        ))
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
            let detail = crate::openhuman::integrations::client::extract_error_detail(
                &body_text,
                crate::openhuman::integrations::client::MAX_ERROR_BODY_LEN,
            );
            // Use the same UTF-8-safe truncation for the debug-log preview
            // — direct byte-slicing (`&body_text[..len.min(300)]`) panics
            // when the cutoff lands inside a multibyte codepoint.
            let logged_body =
                crate::openhuman::integrations::client::extract_error_detail(&body_text, 300);
            tracing::debug!(
                "[composio] DELETE {} → {} body={}",
                url,
                status,
                logged_body
            );
            let status_str = status.as_u16().to_string();
            crate::core::observability::report_error(
                format!("Backend returned {status} for DELETE {url}: {detail}").as_str(),
                "composio",
                "delete",
                &[
                    ("path", path),
                    ("status", status_str.as_str()),
                    ("failure", "non_2xx"),
                ],
            );
            anyhow::bail!("Backend returned {status} for DELETE {url}: {detail}");
        }

        let envelope: Envelope<T> = resp.json().await?;
        if !envelope.success {
            let msg = envelope
                .error
                .unwrap_or_else(|| "unknown backend error".into());
            crate::core::observability::report_error(
                msg.as_str(),
                "composio",
                "delete",
                &[("path", path), ("failure", "envelope_error")],
            );
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
/// core defaults (`config.api_url` plus the app-session JWT) via
/// [`crate::openhuman::integrations::build_client`]. The only reason
/// this returns `None` is that the user isn't signed in yet.
pub fn build_composio_client(config: &crate::openhuman::config::Config) -> Option<ComposioClient> {
    let inner = crate::openhuman::integrations::build_client(config)?;
    Some(ComposioClient::new(inner))
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
