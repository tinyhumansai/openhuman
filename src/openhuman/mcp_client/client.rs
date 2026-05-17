use crate::openhuman::config::{McpAuthConfig, McpClientIdentityConfig};
use crate::openhuman::skills::types::ToolResult;
use anyhow::Context;
use base64::Engine;
use parking_lot::Mutex;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
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
            .redirect(reqwest::redirect::Policy::none());
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
                    .header(CONTENT_TYPE, "application/json"),
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
            .header(CONTENT_TYPE, "application/json");
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
            .header(CONTENT_TYPE, "application/json");
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

fn parse_sse_message(body: &str) -> anyhow::Result<Value> {
    let events = parse_sse_events(body)?;
    let event = events
        .into_iter()
        .find_map(|event| event.data)
        .ok_or_else(|| anyhow::anyhow!("No SSE data frame found in MCP response: {body}"))?;
    Ok(event)
}

fn parse_sse_events(body: &str) -> anyhow::Result<Vec<McpSseEvent>> {
    let mut events = Vec::new();
    let mut event_type: Option<String> = None;
    let mut event_id: Option<String> = None;
    let mut data_lines: Vec<String> = Vec::new();

    let flush = |events: &mut Vec<McpSseEvent>,
                 event_type: &mut Option<String>,
                 event_id: &mut Option<String>,
                 data_lines: &mut Vec<String>|
     -> anyhow::Result<()> {
        if event_type.is_none() && event_id.is_none() && data_lines.is_empty() {
            return Ok(());
        }
        let data = if data_lines.is_empty() {
            None
        } else {
            let joined = data_lines.join("\n");
            Some(
                serde_json::from_str(&joined)
                    .with_context(|| format!("Failed to parse SSE data frame JSON: {joined}"))?,
            )
        };
        events.push(McpSseEvent {
            event: event_type.take(),
            id: event_id.take(),
            data,
        });
        data_lines.clear();
        Ok(())
    };

    for raw_line in body.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            flush(&mut events, &mut event_type, &mut event_id, &mut data_lines)?;
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("event:") {
            event_type = Some(value.trim_start().to_string());
        } else if let Some(value) = line.strip_prefix("id:") {
            event_id = Some(value.trim_start().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start().to_string());
        }
    }
    flush(&mut events, &mut event_type, &mut event_id, &mut data_lines)?;
    Ok(events)
}

fn parse_www_authenticate_challenge(headers: &HeaderMap) -> Option<McpAuthChallenge> {
    let raw = headers.get("WWW-Authenticate")?.to_str().ok()?.trim();
    let mut parts = raw.splitn(2, ' ');
    let scheme = parts.next()?.trim().to_string();
    let params = parts.next().unwrap_or("").trim();
    let attrs = parse_auth_attribute_list(params);
    Some(McpAuthChallenge {
        scheme,
        realm: attrs.get("realm").cloned(),
        resource_metadata: attrs.get("resource_metadata").cloned(),
    })
}

fn parse_auth_attribute_list(input: &str) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    for part in input.split(',') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_string();
        attrs.insert(key.trim().to_string(), value);
    }
    attrs
}

fn header_to_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name)?.to_str().ok().map(|s| s.to_string())
}

fn x_mcp_headers_from_schema(
    tool: &McpRemoteTool,
    arguments: &Value,
) -> anyhow::Result<Vec<(HeaderName, HeaderValue)>> {
    let mut headers = Vec::new();
    let Some(args) = arguments.as_object() else {
        return Ok(headers);
    };
    let properties = tool
        .input_schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    for (param_name, schema) in properties {
        let Some(header_suffix) = schema.get("x-mcp-header").and_then(Value::as_str) else {
            continue;
        };
        let Some(value) = args.get(&param_name) else {
            continue;
        };
        let header_name =
            HeaderName::from_bytes(format!("Mcp-Param-{header_suffix}").as_bytes())
                .with_context(|| format!("invalid x-mcp-header name for `{param_name}`"))?;
        let header_value = match value {
            Value::String(s) => HeaderValue::from_str(s),
            other => HeaderValue::from_str(&other.to_string()),
        }
        .with_context(|| format!("invalid x-mcp-header value for `{param_name}`"))?;
        headers.push((header_name, header_value));
    }

    Ok(headers)
}

impl McpHttpClient {
    async fn read_response(&self, response: reqwest::Response) -> anyhow::Result<ResponseEnvelope> {
        let status = response.status();
        let headers = response.headers().clone();
        let content_type = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let text = response.text().await?;

        if status == reqwest::StatusCode::UNAUTHORIZED {
            let auth_suffix = if let Some(challenge) = parse_www_authenticate_challenge(&headers) {
                match challenge.resource_metadata.as_deref() {
                    Some(resource_metadata) => {
                        format!("; resource metadata: {resource_metadata}")
                    }
                    None => String::new(),
                }
            } else {
                String::new()
            };
            anyhow::bail!(
                "MCP unauthorized for `{}` (HTTP 401{})",
                redact_endpoint(&self.endpoint),
                auth_suffix
            );
        }
        if !status.is_success() {
            anyhow::bail!("MCP HTTP {} — {}", status.as_u16(), text);
        }

        let payload: Value = if content_type.starts_with("text/event-stream") {
            parse_sse_message(&text)?
        } else {
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
mod tests {
    use super::*;
    use axum::{
        extract::State,
        http::{HeaderMap as AxumHeaderMap, Method, StatusCode},
        response::{IntoResponse, Response},
        routing::{get, post},
        Json, Router,
    };
    use serde_json::Value;
    use std::sync::{
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
        Arc,
    };

    #[derive(Clone)]
    struct TestState {
        init_count: Arc<AtomicUsize>,
        call_count: Arc<AtomicUsize>,
    }

    async fn mcp_handler(
        State(state): State<TestState>,
        headers: AxumHeaderMap,
        method: Method,
        Json(body): Json<Value>,
    ) -> Response {
        let rpc_method = body.get("method").and_then(Value::as_str).unwrap_or("");
        if method == Method::POST && rpc_method == "initialize" {
            state.init_count.fetch_add(1, AtomicOrdering::SeqCst);
            return (
                [(HEADER_SESSION_ID, "session-1")],
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": body["id"].clone(),
                    "result": {
                        "protocolVersion": LATEST_PROTOCOL_VERSION,
                        "capabilities": { "tools": { "listChanged": true } },
                        "serverInfo": { "name": "test-server", "version": "1.0.0" }
                    }
                })),
            )
                .into_response();
        }

        if headers.get(HEADER_SESSION_ID).and_then(|v| v.to_str().ok()) != Some("session-1") {
            return (
                StatusCode::BAD_REQUEST,
                "missing or invalid session".to_string(),
            )
                .into_response();
        }

        if headers
            .get(HEADER_PROTOCOL_VERSION)
            .and_then(|v| v.to_str().ok())
            != Some(LATEST_PROTOCOL_VERSION)
        {
            return (
                StatusCode::BAD_REQUEST,
                "missing protocol version".to_string(),
            )
                .into_response();
        }

        match rpc_method {
            "notifications/initialized" => StatusCode::NO_CONTENT.into_response(),
            "tools/list" => Json(json!({
                "jsonrpc": "2.0",
                "id": body["id"].clone(),
                "result": {
                    "tools": [{
                        "name": "needs_header",
                        "description": "needs x-mcp-header",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "tenant": {
                                    "type": "string",
                                    "x-mcp-header": "tenant"
                                }
                            }
                        }
                    }]
                }
            }))
            .into_response(),
            "tools/call" => {
                state.call_count.fetch_add(1, AtomicOrdering::SeqCst);
                if headers
                    .get("Mcp-Param-tenant")
                    .and_then(|v| v.to_str().ok())
                    != Some("acme")
                {
                    return (
                        StatusCode::BAD_REQUEST,
                        "missing mirrored tenant header".to_string(),
                    )
                        .into_response();
                }
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": body["id"].clone(),
                    "result": {
                        "content": [{
                            "type": "text",
                            "text": "remote result"
                        }]
                    }
                }))
                .into_response()
            }
            _ => (
                StatusCode::BAD_REQUEST,
                format!("unexpected method {rpc_method}"),
            )
                .into_response(),
        }
    }

    async fn events_handler(headers: AxumHeaderMap) -> Response {
        if headers.get(HEADER_SESSION_ID).is_none() {
            return (StatusCode::BAD_REQUEST, "no session".to_string()).into_response();
        }
        (
            [(CONTENT_TYPE.as_str(), "text/event-stream")],
            "id: 1\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"ok\":true}}\n\n",
        )
            .into_response()
    }

    async fn delete_handler() -> Response {
        StatusCode::NO_CONTENT.into_response()
    }

    async fn bearer_required_handler(headers: AxumHeaderMap, Json(body): Json<Value>) -> Response {
        if headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer secret-token") {
            return (StatusCode::UNAUTHORIZED, "missing bearer".to_string()).into_response();
        }
        Json(json!({
            "jsonrpc": "2.0",
            "id": body["id"].clone(),
            "result": {
                "protocolVersion": LATEST_PROTOCOL_VERSION,
                "capabilities": {},
                "serverInfo": { "name": "bearer-server", "version": "1.0.0" }
            }
        }))
        .into_response()
    }

    async fn retrying_mcp_handler(
        State(state): State<TestState>,
        headers: AxumHeaderMap,
        Json(body): Json<Value>,
    ) -> Response {
        let rpc_method = body.get("method").and_then(Value::as_str).unwrap_or("");
        if rpc_method == "initialize" {
            state.init_count.fetch_add(1, AtomicOrdering::SeqCst);
            return (
                [(HEADER_SESSION_ID, "session-retry")],
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": body["id"].clone(),
                    "result": {
                        "protocolVersion": LATEST_PROTOCOL_VERSION,
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": "retry-server", "version": "1.0.0" }
                    }
                })),
            )
                .into_response();
        }
        if rpc_method == "notifications/initialized" {
            return StatusCode::NO_CONTENT.into_response();
        }
        if rpc_method == "tools/list" {
            let call_number = state.call_count.fetch_add(1, AtomicOrdering::SeqCst);
            if call_number == 0
                && headers.get(HEADER_SESSION_ID).and_then(|v| v.to_str().ok())
                    == Some("session-retry")
            {
                return (StatusCode::NOT_FOUND, "expired".to_string()).into_response();
            }
            return Json(json!({
                "jsonrpc": "2.0",
                "id": body["id"].clone(),
                "result": { "tools": [] }
            }))
            .into_response();
        }
        (StatusCode::BAD_REQUEST, "unexpected".to_string()).into_response()
    }

    async fn spawn_test_server() -> (String, TestState) {
        let state = TestState {
            init_count: Arc::new(AtomicUsize::new(0)),
            call_count: Arc::new(AtomicUsize::new(0)),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new()
            .route(
                "/",
                post(mcp_handler).get(events_handler).delete(delete_handler),
            )
            .with_state(state.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/"), state)
    }

    async fn spawn_retry_server() -> (String, TestState) {
        let state = TestState {
            init_count: Arc::new(AtomicUsize::new(0)),
            call_count: Arc::new(AtomicUsize::new(0)),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/", post(retrying_mcp_handler))
            .with_state(state.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/"), state)
    }

    async fn spawn_discovery_server() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let auth_header = format!(
            "Bearer realm=\"mcp\", resource_metadata=\"{base}/.well-known/oauth-protected-resource\""
        );
        let prm_base = base.clone();
        let issuer_base = base.clone();
        let app = Router::new()
            .route(
                "/",
                post(move || {
                    let auth_header = auth_header.clone();
                    async move {
                        (
                            StatusCode::UNAUTHORIZED,
                            [("WWW-Authenticate", auth_header.as_str())],
                            "",
                        )
                            .into_response()
                    }
                }),
            )
            .route(
                "/.well-known/oauth-protected-resource",
                get(move || {
                    let prm_base = prm_base.clone();
                    async move {
                        let resource = format!("{prm_base}/");
                        Json(json!({
                            "resource": resource,
                            "authorization_servers": [prm_base],
                            "scopes_supported": ["mcp:tools"]
                        }))
                    }
                }),
            )
            .route(
                "/.well-known/openid-configuration",
                get(move || {
                    let issuer_base = issuer_base.clone();
                    async move {
                        let authorization_endpoint = format!("{}/authorize", issuer_base);
                        let token_endpoint = format!("{}/token", issuer_base);
                        Json(json!({
                            "issuer": issuer_base,
                            "authorization_endpoint": authorization_endpoint,
                            "token_endpoint": token_endpoint,
                            "grant_types_supported": ["authorization_code"],
                            "code_challenge_methods_supported": ["S256"]
                        }))
                    }
                }),
            );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn initialize_and_list_tools_negotiate_session() {
        let (endpoint, state) = spawn_test_server().await;
        let client = McpHttpClient::new(endpoint, 5);
        let tools = client.list_tools().await.expect("list_tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(state.init_count.load(AtomicOrdering::SeqCst), 1);
        let snapshot = client.initialize_snapshot().expect("snapshot");
        assert_eq!(snapshot.protocol_version, LATEST_PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn call_tool_mirrors_x_mcp_header_parameters() {
        let (endpoint, state) = spawn_test_server().await;
        let client = McpHttpClient::new(endpoint, 5);
        let result = client
            .call_tool("needs_header", json!({"tenant": "acme"}))
            .await
            .expect("call_tool");
        assert_eq!(result.rendered.output(), "remote result");
        assert_eq!(state.call_count.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test]
    async fn session_404_triggers_reinitialize_and_retry() {
        let (endpoint, state) = spawn_retry_server().await;
        let client = McpHttpClient::new(endpoint, 5);
        let tools = client.list_tools().await.expect("list_tools");
        assert!(tools.is_empty());
        assert_eq!(state.init_count.load(AtomicOrdering::SeqCst), 2);
        assert_eq!(state.call_count.load(AtomicOrdering::SeqCst), 2);
    }

    #[tokio::test]
    async fn drain_events_parses_sse_stream() {
        let (endpoint, _) = spawn_test_server().await;
        let client = McpHttpClient::new(endpoint, 5);
        let events = client.drain_events(None).await.expect("drain events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id.as_deref(), Some("1"));
        assert_eq!(events[0].event.as_deref(), Some("message"));
        assert_eq!(events[0].data.as_ref().unwrap()["params"]["ok"], true);
    }

    #[tokio::test]
    async fn close_session_sends_delete() {
        let (endpoint, _) = spawn_test_server().await;
        let client = McpHttpClient::new(endpoint, 5);
        client.initialize().await.expect("initialize");
        client.close_session().await.expect("close_session");
        assert!(client.initialize_snapshot().is_none());
    }

    #[test]
    fn redact_endpoint_hides_paths_and_credentials() {
        assert_eq!(
            redact_endpoint("https://example.com/path?x=1"),
            "https://example.com"
        );
        assert_eq!(
            redact_endpoint("https://user:pw@example.com/a"),
            "<redacted>"
        );
    }

    #[test]
    fn parse_sse_events_handles_multiple_frames() {
        let body = "id: 1\nevent: message\ndata: {\"a\":1}\n\ndata: {\"b\":2}\n\n";
        let events = parse_sse_events(body).expect("events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id.as_deref(), Some("1"));
        assert_eq!(events[1].data.as_ref().unwrap()["b"], 2);
    }

    #[test]
    fn parse_www_authenticate_extracts_resource_metadata() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "WWW-Authenticate",
            HeaderValue::from_static(
                "Bearer realm=\"mcp\", resource_metadata=\"https://example.com/.well-known/oauth-protected-resource\"",
            ),
        );
        let challenge = parse_www_authenticate_challenge(&headers).expect("challenge");
        assert_eq!(challenge.scheme, "Bearer");
        assert_eq!(challenge.realm.as_deref(), Some("mcp"));
        assert_eq!(
            challenge.resource_metadata.as_deref(),
            Some("https://example.com/.well-known/oauth-protected-resource")
        );
    }

    #[tokio::test]
    async fn discover_authorization_returns_none_when_not_401() {
        let (endpoint, _) = spawn_test_server().await;
        let client = McpHttpClient::new(endpoint, 5);
        assert!(client.discover_authorization().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn discover_authorization_fetches_metadata() {
        let endpoint = spawn_discovery_server().await;
        let client = McpHttpClient::new(endpoint, 2);
        let ctx = client
            .discover_authorization()
            .await
            .expect("discover")
            .expect("some");
        assert_eq!(ctx.challenge.scheme, "Bearer");
        assert_eq!(
            ctx.protected_resource_metadata
                .as_ref()
                .unwrap()
                .scopes_supported,
            vec!["mcp:tools"]
        );
        assert_eq!(ctx.authorization_server_metadata.len(), 1);
        let expected_authorization_endpoint = format!(
            "{}/authorize",
            ctx.protected_resource_metadata
                .as_ref()
                .unwrap()
                .authorization_servers[0]
        );
        assert_eq!(
            ctx.authorization_server_metadata[0]
                .authorization_endpoint
                .as_deref(),
            Some(expected_authorization_endpoint.as_str())
        );
    }

    #[tokio::test]
    async fn bearer_auth_is_attached_to_initialize() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route("/", post(bearer_required_handler));
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = McpHttpClient::with_options(
            format!("http://{addr}/"),
            2,
            McpAuthConfig::BearerToken {
                token: "secret-token".into(),
            },
            McpClientIdentityConfig::default(),
        );
        let init = client.initialize().await.expect("initialize");
        assert_eq!(init.server_info["name"], "bearer-server");
    }
}
