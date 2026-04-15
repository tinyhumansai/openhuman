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
    ComposioAuthorizeResponse, ComposioConnectionsResponse, ComposioCreateTriggerResponse,
    ComposioDeleteResponse, ComposioExecuteResponse, ComposioGithubReposResponse,
    ComposioToolkitsResponse, ComposioToolsResponse,
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
pub fn build_composio_client(config: &crate::openhuman::config::Config) -> Option<ComposioClient> {
    let inner = crate::openhuman::integrations::build_client(config)?;
    Some(ComposioClient::new(inner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;

    /// `build_composio_client` must return `None` when the user has no auth
    /// token — callers treat that as "skip silently" (user not signed in).
    #[test]
    fn build_composio_client_none_without_auth_token() {
        let mut config = Config::default();
        config.api_key = None;
        assert!(build_composio_client(&config).is_none());
    }

    /// With an auth token, we should get a live client wrapping the
    /// shared integration client. We scope `config_path` to a temp dir
    /// so the session-token lookup doesn't pick up a real dev profile
    /// off-disk — the test exercises the pure `config.api_key` fallback.
    #[test]
    fn build_composio_client_some_with_auth_token() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");
        config.api_key = Some("test-token".into());
        let client =
            build_composio_client(&config).expect("client should build when api_key is set");
        assert!(
            !client.inner().auth_token.is_empty(),
            "resolved auth token should not be empty"
        );
    }

    /// `authorize()` is input-validated — an empty / whitespace toolkit
    /// must error without making any HTTP call.
    #[tokio::test]
    async fn authorize_rejects_empty_toolkit() {
        let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
            "http://127.0.0.1:0".into(),
            "test".into(),
        ));
        let client = ComposioClient::new(inner);
        let err = client.authorize("   ").await.unwrap_err();
        assert!(
            err.to_string().contains("toolkit must not be empty"),
            "unexpected error: {err}"
        );
    }

    /// `delete_connection()` likewise must reject empty connection ids.
    #[tokio::test]
    async fn delete_connection_rejects_empty_id() {
        let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
            "http://127.0.0.1:0".into(),
            "test".into(),
        ));
        let client = ComposioClient::new(inner);
        let err = client.delete_connection("").await.unwrap_err();
        assert!(
            err.to_string().contains("connectionId must not be empty"),
            "unexpected error: {err}"
        );
    }

    /// `execute_tool()` must refuse empty slugs — otherwise the backend
    /// would receive a malformed request.
    #[tokio::test]
    async fn execute_tool_rejects_empty_slug() {
        let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
            "http://127.0.0.1:0".into(),
            "test".into(),
        ));
        let client = ComposioClient::new(inner);
        let err = client.execute_tool("", None).await.unwrap_err();
        assert!(
            err.to_string().contains("tool slug must not be empty"),
            "unexpected error: {err}"
        );
    }

    /// ComposioClient is `Clone` so each tool gets a cheap handle share.
    /// Inner client must be Arc-shared — no duplication.
    #[test]
    fn client_clone_shares_inner_arc() {
        let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
            "http://127.0.0.1:0".into(),
            "test".into(),
        ));
        let client_a = ComposioClient::new(inner);
        let client_b = client_a.clone();
        assert!(
            Arc::ptr_eq(client_a.inner(), client_b.inner()),
            "clones should share the same Arc<IntegrationClient>"
        );
    }

    // ── Mock-backend integration tests ─────────────────────────────
    //
    // These stand up a real axum HTTP server on a random localhost port,
    // point a `ComposioClient` at it, and drive each method end-to-end.
    // That exercises the envelope parsing, HTTP plumbing, and URL
    // construction in `ComposioClient` — which is otherwise only covered
    // by live backend tests.

    use axum::{
        extract::{Path, Query},
        http::StatusCode,
        routing::{get, post},
        Json, Router,
    };
    use serde_json::{json, Value};
    use std::collections::HashMap;

    async fn start_mock_backend(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    fn build_client_for(base_url: String) -> ComposioClient {
        let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
            base_url,
            "test-token".into(),
        ));
        ComposioClient::new(inner)
    }

    #[tokio::test]
    async fn list_toolkits_parses_backend_envelope() {
        let app = Router::new().route(
            "/agent-integrations/composio/toolkits",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": { "toolkits": ["gmail", "notion"] }
                }))
            }),
        );
        let base = start_mock_backend(app).await;
        let client = build_client_for(base);
        let resp = client.list_toolkits().await.unwrap();
        assert_eq!(
            resp.toolkits,
            vec!["gmail".to_string(), "notion".to_string()]
        );
    }

    #[tokio::test]
    async fn list_connections_parses_connection_array() {
        let app = Router::new().route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {
                        "connections": [
                            { "id": "c1", "toolkit": "gmail", "status": "ACTIVE", "createdAt": "2026-01-01T00:00:00Z" },
                            { "id": "c2", "toolkit": "notion", "status": "PENDING" }
                        ]
                    }
                }))
            }),
        );
        let base = start_mock_backend(app).await;
        let client = build_client_for(base);
        let resp = client.list_connections().await.unwrap();
        assert_eq!(resp.connections.len(), 2);
        assert_eq!(resp.connections[0].id, "c1");
        assert_eq!(resp.connections[1].status, "PENDING");
    }

    #[tokio::test]
    async fn authorize_posts_toolkit_and_returns_connect_url() {
        let app = Router::new().route(
            "/agent-integrations/composio/authorize",
            post(|Json(body): Json<Value>| async move {
                // Echo toolkit back so we know our POST body made it.
                let tk = body["toolkit"].as_str().unwrap_or("").to_string();
                Json(json!({
                    "success": true,
                    "data": {
                        "connectUrl": format!("https://composio.example/{tk}/consent"),
                        "connectionId": "conn-abc"
                    }
                }))
            }),
        );
        let base = start_mock_backend(app).await;
        let client = build_client_for(base);
        let resp = client.authorize("gmail").await.unwrap();
        assert!(resp.connect_url.contains("gmail"));
        assert_eq!(resp.connection_id, "conn-abc");
    }

    #[tokio::test]
    async fn list_tools_filters_pass_through_as_csv_query_param() {
        let app = Router::new().route(
            "/agent-integrations/composio/tools",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                let filter = q.get("toolkits").cloned().unwrap_or_default();
                // Echo the requested filter back in the payload so the
                // test can assert it reached the server correctly.
                Json(json!({
                    "success": true,
                    "data": {
                        "tools": [{
                            "type": "function",
                            "function": {
                                "name": format!("ECHO_{filter}"),
                                "description": "echo",
                                "parameters": {}
                            }
                        }]
                    }
                }))
            }),
        );
        let base = start_mock_backend(app).await;
        let client = build_client_for(base);

        // No filter: URL should lack `toolkits` query
        let resp_all = client.list_tools(None).await.unwrap();
        assert_eq!(resp_all.tools.len(), 1);
        assert_eq!(resp_all.tools[0].function.name, "ECHO_");

        // With filter: CSV-joined
        let resp_filtered = client
            .list_tools(Some(&["gmail".to_string(), "notion".to_string()]))
            .await
            .unwrap();
        assert_eq!(resp_filtered.tools[0].function.name, "ECHO_gmail,notion");

        // Whitespace entries should be dropped before joining
        let resp_trimmed = client
            .list_tools(Some(&["gmail".to_string(), "  ".to_string()]))
            .await
            .unwrap();
        assert_eq!(resp_trimmed.tools[0].function.name, "ECHO_gmail");
    }

    #[tokio::test]
    async fn execute_tool_returns_cost_and_success_flags() {
        let app = Router::new().route(
            "/agent-integrations/composio/execute",
            post(|Json(body): Json<Value>| async move {
                let tool = body["tool"].as_str().unwrap_or("").to_string();
                Json(json!({
                    "success": true,
                    "data": {
                        "data": { "echoed_tool": tool },
                        "successful": true,
                        "error": null,
                        "costUsd": 0.0025
                    }
                }))
            }),
        );
        let base = start_mock_backend(app).await;
        let client = build_client_for(base);
        let resp = client
            .execute_tool("GMAIL_SEND_EMAIL", Some(json!({"to": "a@b.com"})))
            .await
            .unwrap();
        assert!(resp.successful);
        assert!((resp.cost_usd - 0.0025).abs() < f64::EPSILON);
        assert_eq!(resp.data["echoed_tool"], "GMAIL_SEND_EMAIL");
    }

    #[tokio::test]
    async fn execute_tool_without_arguments_sends_empty_object() {
        let app = Router::new().route(
            "/agent-integrations/composio/execute",
            post(|Json(body): Json<Value>| async move {
                // Verify default arguments is an object (not missing / null).
                assert!(body["arguments"].is_object());
                Json(json!({
                    "success": true,
                    "data": {
                        "data": {},
                        "successful": true,
                        "error": null,
                        "costUsd": 0.0
                    }
                }))
            }),
        );
        let base = start_mock_backend(app).await;
        let client = build_client_for(base);
        let resp = client.execute_tool("NOOP_ACTION", None).await.unwrap();
        assert!(resp.successful);
    }

    #[tokio::test]
    async fn backend_error_envelope_becomes_bail() {
        let app = Router::new().route(
            "/agent-integrations/composio/toolkits",
            get(|| async { Json(json!({ "success": false, "error": "backend unavailable" })) }),
        );
        let base = start_mock_backend(app).await;
        let client = build_client_for(base);
        let err = client.list_toolkits().await.unwrap_err();
        assert!(err.to_string().contains("backend unavailable"));
    }

    #[tokio::test]
    async fn http_error_status_propagates() {
        let app = Router::new().route(
            "/agent-integrations/composio/connections",
            get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
        );
        let base = start_mock_backend(app).await;
        let client = build_client_for(base);
        let err = client.list_connections().await.unwrap_err();
        assert!(err.to_string().contains("500") || err.to_string().contains("Backend returned"));
    }

    #[tokio::test]
    async fn delete_connection_happy_path_returns_deleted_true() {
        let app = Router::new().route(
            "/agent-integrations/composio/connections/{id}",
            axum::routing::delete(|Path(id): Path<String>| async move {
                assert_eq!(id, "conn-42");
                Json(json!({
                    "success": true,
                    "data": { "deleted": true }
                }))
            }),
        );
        let base = start_mock_backend(app).await;
        let client = build_client_for(base);
        let resp = client.delete_connection("conn-42").await.unwrap();
        assert!(resp.deleted);
    }
}
