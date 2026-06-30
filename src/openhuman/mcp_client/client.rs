use crate::openhuman::config::{McpAuthConfig, McpClientIdentityConfig};
use crate::openhuman::workflows::types::ToolResult;
use anyhow::Context;
use base64::Engine;
use futures_util::StreamExt;
use parking_lot::Mutex;
#[cfg(test)]
use reqwest::header::HeaderMap;
use reqwest::header::{HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    "2024-11-05",
    "2025-03-26",
    "2025-06-18",
    LATEST_PROTOCOL_VERSION,
];
const HEADER_PROTOCOL_VERSION: &str = "MCP-Protocol-Version";
const HEADER_SESSION_ID: &str = "Mcp-Session-Id";
const HEADER_METHOD: &str = "Mcp-Method";
const HEADER_NAME: &str = "Mcp-Name";
const MCP_HTTP_ACCEPT: &str = "application/json, text/event-stream";

/// A tool advertised by a remote MCP server.
///
/// `description` and `title` arrive verbatim from an untrusted remote
/// peer. Callers in LLM-context code paths MUST read them through
/// [`McpRemoteTool::display_description`] / [`McpRemoteTool::display_title`]
/// — never the raw fields directly — so the registry's sanitization
/// pipeline (`mcp_client::sanitize`) is always applied. The raw fields
/// stay `pub` (rather than `pub(super)`) because the type is `serde`-
/// deserialized verbatim from server payloads and constructed by sibling
/// transport modules; the boundary that matters is the *consumption*
/// site, not the *storage* site.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpRemoteTool {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,
}

impl McpRemoteTool {
    /// Sanitized description suitable for inclusion in the agent LLM
    /// tool-use context.
    ///
    /// Always returns content that has been run through the
    /// `mcp_client::sanitize` pipeline (control-char strip, instruction-
    /// fence strip, length cap) regardless of what the remote server
    /// sent.
    pub fn display_description(&self) -> Option<String> {
        self.description.as_deref().map(|d| {
            crate::openhuman::mcp_client::sanitize::sanitize_for_llm(
                d,
                crate::openhuman::mcp_client::sanitize::MAX_DESCRIPTION_BYTES,
            )
        })
    }

    /// Sanitized title suitable for LLM / UI display.
    ///
    /// Same pipeline as [`Self::display_description`], capped at
    /// [`crate::openhuman::mcp_client::sanitize::MAX_TITLE_BYTES`].
    pub fn display_title(&self) -> Option<String> {
        self.title.as_deref().map(|t| {
            crate::openhuman::mcp_client::sanitize::sanitize_for_llm(
                t,
                crate::openhuman::mcp_client::sanitize::MAX_TITLE_BYTES,
            )
        })
    }
}

#[derive(Debug, Clone)]
pub struct McpServerToolResult {
    pub raw_result: Value,
    pub rendered: ToolResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpClientInfo {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpInitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(default, rename = "serverInfo")]
    pub server_info: Value,
    #[serde(default)]
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtectedResourceMetadata {
    pub resource: String,
    #[serde(default)]
    pub authorization_servers: Vec<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthorizationServerMetadata {
    pub issuer: String,
    #[serde(default)]
    pub authorization_endpoint: Option<String>,
    #[serde(default)]
    pub token_endpoint: Option<String>,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub response_types_supported: Vec<String>,
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpAuthChallenge {
    pub scheme: String,
    pub realm: Option<String>,
    pub resource_metadata: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpAuthorizationContext {
    pub challenge: McpAuthChallenge,
    pub protected_resource_metadata: Option<ProtectedResourceMetadata>,
    pub authorization_server_metadata: Vec<AuthorizationServerMetadata>,
}

/// Typed error for an HTTP 401 from a remote MCP server. Carried as the root
/// of the `anyhow` chain so the connect path can recognise an auth failure via
/// `downcast_ref` — instead of fragile string matching — and classify the
/// server as "needs authentication" rather than a generic transport error
/// (issue #3719). The `Display` form is the diagnostic string used in logs;
/// the user-facing copy is derived separately in `mcp_registry::connections`.
#[derive(Debug, Clone)]
pub struct McpUnauthorizedError {
    /// Redacted endpoint (scheme + authority only) the 401 came from.
    pub endpoint: String,
    /// `resource_metadata` URL advertised by the `WWW-Authenticate` challenge,
    /// when present — the entry point to OAuth discovery.
    pub resource_metadata: Option<String>,
}

impl std::fmt::Display for McpUnauthorizedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.resource_metadata {
            Some(rm) => write!(
                f,
                "MCP unauthorized for `{}` (HTTP 401; resource metadata: {rm})",
                self.endpoint
            ),
            None => write!(f, "MCP unauthorized for `{}` (HTTP 401)", self.endpoint),
        }
    }
}

impl std::error::Error for McpUnauthorizedError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpSseEvent {
    pub event: Option<String>,
    pub id: Option<String>,
    pub data: Option<Value>,
}

#[derive(Debug)]
pub struct McpHttpClient {
    endpoint: String,
    http: reqwest::Client,
    next_id: AtomicI64,
    client_info: McpClientInfo,
    auth: McpAuthConfig,
    state: Mutex<SessionState>,
}

#[derive(Debug, Default)]
struct SessionState {
    initialized: bool,
    negotiated_protocol_version: String,
    session_id: Option<String>,
    initialize: Option<McpInitializeResult>,
    cached_tools: HashMap<String, McpRemoteTool>,
}

impl McpHttpClient {
    pub fn new(endpoint: String, timeout_secs: u64) -> Self {
        Self::with_options(
            endpoint,
            timeout_secs,
            McpAuthConfig::None,
            McpClientIdentityConfig::default(),
        )
    }

    pub fn with_options(
        endpoint: String,
        timeout_secs: u64,
        auth: McpAuthConfig,
        identity: McpClientIdentityConfig,
    ) -> Self {
        let builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            // Follow a bounded number of redirects so servers published behind a
            // vanity/short URL that 30x-redirects to their real MCP endpoint
            // (e.g. `sh.inference.ac` -> `api.inference.sh/mcp`) connect instead
            // of failing with a raw `MCP HTTP 301`. `Policy::limited` is safe
            // here: reqwest strips sensitive headers (Authorization, Cookie) on
            // cross-origin redirects, so a server bearer token is never leaked
            // to the redirect target.
            .redirect(reqwest::redirect::Policy::limited(5));
        let builder =
            crate::openhuman::config::apply_runtime_proxy_to_builder(builder, "tool.mcp_client");
        let http = builder.build().expect("reqwest client must build");
        Self {
            endpoint,
            http,
            next_id: AtomicI64::new(1),
            client_info: McpClientInfo {
                name: identity.name,
                title: Some(identity.title),
                version: identity.version,
            },
            auth,
            state: Mutex::new(SessionState {
                negotiated_protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
                ..SessionState::default()
            }),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn initialize_snapshot(&self) -> Option<McpInitializeResult> {
        self.state.lock().initialize.clone()
    }

    pub async fn initialize(&self) -> anyhow::Result<McpInitializeResult> {
        if let Some(existing) = self.state.lock().initialize.clone() {
            return Ok(existing);
        }

        let params = json!({
            "protocolVersion": LATEST_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": self.client_info,
        });
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": params,
        });
        let request = self
            .apply_auth(
                self.http
                    .post(&self.endpoint)
                    .header(CONTENT_TYPE, "application/json")
                    .header(ACCEPT, MCP_HTTP_ACCEPT),
                true,
            )
            .body(serde_json::to_vec(&body)?);
        let response = self.read_response(request.send().await?).await?;
        let init: McpInitializeResult =
            serde_json::from_value(response.result.clone()).context("parsing initialize result")?;
        self.validate_protocol_version(&init.protocol_version)?;

        {
            let mut state = self.state.lock();
            state.initialized = true;
            state.negotiated_protocol_version = init.protocol_version.clone();
            state.session_id = response.session_id.clone();
            state.initialize = Some(init.clone());
        }

        self.send_notification("notifications/initialized", json!({}))
            .await?;

        Ok(init)
    }

    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpRemoteTool>> {
        self.initialize().await?;
        let result = self
            .send_jsonrpc(
                "tools/list",
                json!({}),
                RequestOptions::standard("tools/list", None, None),
            )
            .await?
            .result;
        let tools = serde_json::from_value::<Vec<McpRemoteTool>>(
            result
                .get("tools")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("MCP tools/list response missing `tools`"))?,
        )?;
        let mut state = self.state.lock();
        state.cached_tools = tools
            .iter()
            .cloned()
            .map(|tool| (tool.name.clone(), tool))
            .collect();
        Ok(tools)
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
    ) -> anyhow::Result<McpServerToolResult> {
        self.initialize().await?;
        let cached_tool = { self.state.lock().cached_tools.get(name).cloned() };
        let tool = if let Some(tool) = cached_tool {
            Some(tool)
        } else {
            self.list_tools()
                .await?
                .into_iter()
                .find(|tool| tool.name == name)
        };
        let extra_headers = tool
            .as_ref()
            .map(|tool| x_mcp_headers_from_schema(tool, &arguments))
            .transpose()?
            .unwrap_or_default();

        let result = self
            .send_jsonrpc(
                "tools/call",
                json!({
                    "name": name,
                    "arguments": arguments,
                }),
                RequestOptions::standard("tools/call", Some(name), Some(extra_headers)),
            )
            .await?
            .result;
        let rendered = render_tool_result(&result);
        Ok(McpServerToolResult {
            raw_result: result,
            rendered,
        })
    }

    pub async fn discover_authorization(&self) -> anyhow::Result<Option<McpAuthorizationContext>> {
        let request = self
            .http
            .post(&self.endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, MCP_HTTP_ACCEPT)
            .body(serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": self.next_id.fetch_add(1, Ordering::Relaxed),
                "method": "initialize",
                "params": {
                    "protocolVersion": LATEST_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": self.client_info,
                }
            }))?);
        let response = self.apply_auth(request, true).send().await?;
        if response.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Ok(None);
        }

        let challenge = parse_www_authenticate_challenge(response.headers())
            .ok_or_else(|| anyhow::anyhow!("401 response missing parseable WWW-Authenticate"))?;
        let prm = if let Some(url) = challenge.resource_metadata.as_deref() {
            Some(self.fetch_json::<ProtectedResourceMetadata>(url).await?)
        } else {
            None
        };
        let mut auth_servers = Vec::new();
        if let Some(prm) = prm.as_ref() {
            for issuer in &prm.authorization_servers {
                if let Ok(metadata) = self.fetch_authorization_server_metadata(issuer).await {
                    auth_servers.push(metadata);
                }
            }
        }
        Ok(Some(McpAuthorizationContext {
            challenge,
            protected_resource_metadata: prm,
            authorization_server_metadata: auth_servers,
        }))
    }

    pub async fn drain_events(
        &self,
        last_event_id: Option<&str>,
    ) -> anyhow::Result<Vec<McpSseEvent>> {
        self.initialize().await?;
        let protocol_version = self.state.lock().negotiated_protocol_version.clone();
        let session_id = self.state.lock().session_id.clone();
        let mut request = self
            .apply_auth(self.http.get(&self.endpoint), false)
            .header(ACCEPT, "text/event-stream")
            .header(HEADER_PROTOCOL_VERSION, protocol_version);
        if let Some(session_id) = session_id {
            request = request.header(HEADER_SESSION_ID, session_id);
        }
        if let Some(last_event_id) = last_event_id {
            request = request.header("Last-Event-ID", last_event_id);
        }
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("MCP events GET {} — {}", status.as_u16(), text);
        }
        parse_sse_events(&text)
    }

    pub async fn close_session(&self) -> anyhow::Result<()> {
        let session_id = self.state.lock().session_id.clone();
        let Some(session_id) = session_id else {
            return Ok(());
        };
        let response = self
            .http
            .delete(&self.endpoint)
            .header(HEADER_SESSION_ID, session_id)
            .send()
            .await?;
        if !(response.status().is_success()
            || response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED)
        {
            anyhow::bail!("MCP DELETE failed with {}", response.status());
        }
        let mut state = self.state.lock();
        state.initialized = false;
        state.session_id = None;
        state.initialize = None;
        state.cached_tools.clear();
        Ok(())
    }

    async fn send_notification(&self, method: &str, params: Value) -> anyhow::Result<()> {
        let body = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let request = self
            .http
            .post(&self.endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, MCP_HTTP_ACCEPT);
        let request = self.apply_standard_headers(request, false, method, None, &[]);
        let response = request.body(serde_json::to_vec(&body)?).send().await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "MCP notification {method} failed with {} — {}",
                status,
                text
            );
        }
        Ok(())
    }

    async fn send_jsonrpc(
        &self,
        method: &str,
        params: Value,
        options: RequestOptions,
    ) -> anyhow::Result<ResponseEnvelope> {
        self.send_jsonrpc_inner(method, params, options, true).await
    }

    async fn send_jsonrpc_inner(
        &self,
        method: &str,
        params: Value,
        options: RequestOptions,
        allow_reinitialize: bool,
    ) -> anyhow::Result<ResponseEnvelope> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        tracing::debug!(
            target: "[mcp_client]",
            endpoint = %redact_endpoint(&self.endpoint),
            method,
            initialize = options.initialize,
            "dispatch MCP request"
        );

        let request = self
            .http
            .post(&self.endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, MCP_HTTP_ACCEPT);
        let request = if options.initialize {
            self.apply_auth(request, true)
        } else {
            self.apply_standard_headers(
                request,
                false,
                options.method_header.unwrap_or(method),
                options.name_header.as_deref(),
                &options.extra_headers,
            )
        };
        let response = request.body(serde_json::to_vec(&body)?).send().await?;

        if response.status() == reqwest::StatusCode::NOT_FOUND
            && allow_reinitialize
            && self.state.lock().session_id.is_some()
        {
            tracing::info!(
                target: "[mcp_client]",
                endpoint = %redact_endpoint(&self.endpoint),
                method,
                "session expired with 404; reinitializing and retrying once"
            );
            self.reset_session();
            self.initialize().await?;
            return Box::pin(self.send_jsonrpc_inner(
                method,
                body["params"].clone(),
                options,
                false,
            ))
            .await;
        }

        self.read_response(response).await
    }

    fn apply_standard_headers(
        &self,
        request: reqwest::RequestBuilder,
        initialize: bool,
        method: &str,
        name: Option<&str>,
        extra_headers: &[(HeaderName, HeaderValue)],
    ) -> reqwest::RequestBuilder {
        let protocol_version = self.state.lock().negotiated_protocol_version.clone();
        let session_id = self.state.lock().session_id.clone();
        let mut request = self.apply_auth(request, initialize);
        request = request.header(HEADER_METHOD, method);
        if let Some(name) = name {
            request = request.header(HEADER_NAME, name);
        }
        if !initialize {
            request = request.header(HEADER_PROTOCOL_VERSION, protocol_version);
            if let Some(session_id) = session_id {
                request = request.header(HEADER_SESSION_ID, session_id);
            }
        }
        for (name, value) in extra_headers {
            request = request.header(name, value);
        }
        request
    }

    fn apply_auth(
        &self,
        request: reqwest::RequestBuilder,
        _initialize: bool,
    ) -> reqwest::RequestBuilder {
        match &self.auth {
            McpAuthConfig::None => request,
            McpAuthConfig::BearerToken { token } => {
                request.header(AUTHORIZATION, format!("Bearer {}", token.trim()))
            }
            McpAuthConfig::Basic { username, password } => {
                let encoded = base64::engine::general_purpose::STANDARD
                    .encode(format!("{username}:{password}"));
                request.header(AUTHORIZATION, format!("Basic {encoded}"))
            }
            McpAuthConfig::Header { name, value } => match (
                HeaderName::try_from(name.as_str()),
                HeaderValue::from_str(value),
            ) {
                (Ok(name), Ok(value)) => request.header(name, value),
                _ => request,
            },
            McpAuthConfig::Headers { headers } => {
                // Apply every header — for remotes that authenticate with more
                // than one (e.g. a client key + client secret). A header whose
                // name/value can't be encoded is skipped, not fatal.
                let mut req = request;
                for h in headers {
                    if let (Ok(name), Ok(value)) = (
                        HeaderName::try_from(h.name.as_str()),
                        HeaderValue::from_str(&h.value),
                    ) {
                        req = req.header(name, value);
                    }
                }
                req
            }
            McpAuthConfig::QueryParam { name, value } => {
                request.query(&[(name.as_str(), value.as_str())])
            }
        }
    }

    async fn fetch_json<T>(&self, url: &str) -> anyhow::Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self.http.get(url).send().await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("HTTP {} while fetching {} — {}", status.as_u16(), url, text);
        }
        serde_json::from_str(&text).with_context(|| format!("parsing JSON from {url}"))
    }

    async fn fetch_authorization_server_metadata(
        &self,
        issuer: &str,
    ) -> anyhow::Result<AuthorizationServerMetadata> {
        let trimmed = issuer.trim_end_matches('/');
        let oidc = format!("{trimmed}/.well-known/openid-configuration");
        if let Ok(metadata) = self.fetch_json::<AuthorizationServerMetadata>(&oidc).await {
            return Ok(metadata);
        }
        let oauth = format!("{trimmed}/.well-known/oauth-authorization-server");
        self.fetch_json::<AuthorizationServerMetadata>(&oauth).await
    }

    fn validate_protocol_version(&self, version: &str) -> anyhow::Result<()> {
        if SUPPORTED_PROTOCOL_VERSIONS.contains(&version) {
            Ok(())
        } else {
            anyhow::bail!("unsupported MCP protocol version negotiated by server: {version}");
        }
    }

    fn reset_session(&self) {
        let mut state = self.state.lock();
        state.initialized = false;
        state.session_id = None;
        state.initialize = None;
        state.cached_tools.clear();
        state.negotiated_protocol_version = LATEST_PROTOCOL_VERSION.to_string();
    }
}

#[derive(Debug, Clone)]
struct RequestOptions {
    initialize: bool,
    method_header: Option<&'static str>,
    name_header: Option<String>,
    extra_headers: Vec<(HeaderName, HeaderValue)>,
}

impl RequestOptions {
    fn standard(
        method_header: &'static str,
        name_header: Option<&str>,
        extra_headers: Option<Vec<(HeaderName, HeaderValue)>>,
    ) -> Self {
        Self {
            initialize: false,
            method_header: Some(method_header),
            name_header: name_header.map(str::to_string),
            extra_headers: extra_headers.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone)]
struct ResponseEnvelope {
    result: Value,
    session_id: Option<String>,
}

pub fn render_tool_result(result: &Value) -> ToolResult {
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut out = String::new();
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for block in content {
            if let Some(t) = block.get("text").and_then(Value::as_str) {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(t);
            }
        }
    }
    if out.is_empty() {
        out = result.to_string();
    }

    if is_error {
        ToolResult::error(out)
    } else {
        ToolResult::success(out)
    }
}

pub fn redact_endpoint(raw: &str) -> String {
    let trimmed = raw.trim();
    let (scheme, rest) = if let Some(r) = trimmed.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        ("http", r)
    } else {
        return "<redacted>".into();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return "<redacted>".into();
    }
    format!("{scheme}://{authority}")
}

#[path = "client_helpers.rs"]
mod client_helpers;
use client_helpers::{
    first_complete_sse_data, header_to_string, parse_sse_events, parse_sse_message,
    parse_www_authenticate_challenge, x_mcp_headers_from_schema,
};

impl McpHttpClient {
    async fn read_response(&self, response: reqwest::Response) -> anyhow::Result<ResponseEnvelope> {
        let status = response.status();
        let headers = response.headers().clone();
        let content_type = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            let resource_metadata = parse_www_authenticate_challenge(&headers)
                .and_then(|challenge| challenge.resource_metadata);
            // Return a TYPED error (not a string `bail!`) so callers can
            // `downcast_ref::<McpUnauthorizedError>()` and surface an
            // actionable "needs authentication" state (#3719) rather than a
            // generic failure. `anyhow` preserves the root type through `?`.
            return Err(anyhow::Error::new(McpUnauthorizedError {
                endpoint: redact_endpoint(&self.endpoint),
                resource_metadata,
            }));
        }
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("MCP HTTP {} — {}", status.as_u16(), text);
        }

        let payload: Value = if content_type.starts_with("text/event-stream") {
            // Read the SSE body incrementally and return as soon as the single
            // JSON-RPC reply frame arrives, instead of buffering to stream-close
            // (`response.text().await`). A server that holds the stream open
            // after replying would otherwise stall this call until the request
            // timeout, and those stalls compound across a skill's tool chain
            // (#4195). The request-level reqwest timeout still bounds the worst
            // case (a server that never replies).
            let mut raw: Vec<u8> = Vec::new();
            let mut stream = response.bytes_stream();
            let mut frame: Option<Value> = None;
            while let Some(chunk) = stream.next().await {
                raw.extend_from_slice(&chunk?);
                // Decode the whole buffer each pass so a multi-byte UTF-8
                // sequence split across chunk boundaries is never corrupted.
                if let Some(data) = first_complete_sse_data(&String::from_utf8_lossy(&raw))? {
                    frame = Some(data);
                    break;
                }
            }
            match frame {
                Some(data) => data,
                // Stream ended without a data frame — fall back to the whole-body
                // parser for a clear "no data frame" error that includes the body.
                None => parse_sse_message(&String::from_utf8_lossy(&raw))?,
            }
        } else {
            let text = response.text().await?;
            serde_json::from_str(&text).map_err(|e| {
                anyhow::anyhow!("Failed to parse MCP JSON response: {e} — body: {text}")
            })?
        };
        if let Some(err) = payload.get("error") {
            anyhow::bail!("MCP error: {err}");
        }
        let result = payload
            .get("result")
            .ok_or_else(|| anyhow::anyhow!("MCP response missing `result`: {payload}"))?
            .clone();
        Ok(ResponseEnvelope {
            result,
            session_id: header_to_string(&headers, HEADER_SESSION_ID),
        })
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
