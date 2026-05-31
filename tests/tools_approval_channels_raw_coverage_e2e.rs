//! Raw-line oriented integration coverage for tools, approval, channels, and
//! tool_registry surfaces that are not covered by the narrower controller tests.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::Request;
use axum::http::{header::AUTHORIZATION, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use reqwest::StatusCode as ReqwestStatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::event_bus::{DomainEvent, EventHandler};
use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::core::socketio::WebChannelEvent;
use openhuman_core::openhuman::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, AgentTier, DefinitionSource, ModelSpec, PromptSource,
    SandboxMode, SkillsWildcard, SubagentEntry, ToolScope as AgentToolScope,
};
use openhuman_core::openhuman::agent::host_runtime::NativeRuntime;
use openhuman_core::openhuman::channels::email_channel::EmailConfig;
use openhuman_core::openhuman::channels::irc::IrcChannelConfig;
use openhuman_core::openhuman::channels::proactive::ProactiveMessageSubscriber;
use openhuman_core::openhuman::channels::traits::ChannelMessage;
use openhuman_core::openhuman::channels::yuanbao::config::YuanbaoConfig;
use openhuman_core::openhuman::channels::yuanbao::errors::{
    AUTH_FAILED_CODES, AUTH_RETRYABLE_CODES, NO_RECONNECT_CLOSE_CODES,
};
use openhuman_core::openhuman::channels::yuanbao::inbound::{
    InboundPipeline, PipelineOutcome, PipelineState,
};
use openhuman_core::openhuman::channels::yuanbao::media::{
    build_file_msg_body, build_image_msg_body, guess_mime_type, image_format_code, is_image,
    parse_image_size,
};
use openhuman_core::openhuman::channels::yuanbao::proto::{
    decode_auth_bind_rsp, decode_conn_msg, decode_inbound_json, decode_inbound_push,
    decode_push_msg, encode_auth_bind, encode_conn_msg, encode_msg_body_element, encode_ping,
    encode_push_ack,
};
use openhuman_core::openhuman::channels::yuanbao::proto_constants::{cmd, cmd_type, module};
use openhuman_core::openhuman::channels::yuanbao::sign::{
    build_timestamp, compute_signature, generate_nonce, SignManager,
};
use openhuman_core::openhuman::channels::yuanbao::splitter::split_markdown;
use openhuman_core::openhuman::channels::yuanbao::types::{
    Account as YuanbaoAccount, ConnFrame as YuanbaoConnFrame,
    ConnectionState as YuanbaoConnectionState, GroupInfo as YuanbaoGroupInfo,
    GroupMember as YuanbaoGroupMember, GroupMemberListPage as YuanbaoGroupMemberListPage,
    ImMsgSeq as YuanbaoImMsgSeq, ImageInfo as YuanbaoImageInfo,
    InboundMessage as YuanbaoInboundMessage, MessageKind as YuanbaoMessageKind,
    MsgBodyElement as YuanbaoMsgBodyElement, MsgContent as YuanbaoMsgContent,
    Source as YuanbaoSource,
};
use openhuman_core::openhuman::channels::yuanbao::wire::{
    decode_varint, encode_field_bytes, encode_field_string, encode_field_varint, encode_varint,
    get_bytes, get_repeated_bytes, get_string, get_varint, next_seq_no, parse_fields, FieldValue,
};
use openhuman_core::openhuman::channels::yuanbao::YuanbaoChannel;
use openhuman_core::openhuman::channels::{
    doctor_channels, Channel, CliChannel, DingTalkChannel, EmailChannel, IMessageChannel,
    IrcChannel, LinqChannel, MattermostChannel, QQChannel, SendMessage, SignalChannel,
    SlackChannel, WhatsAppChannel,
};
use openhuman_core::openhuman::composio::all_composio_agent_tools;
use openhuman_core::openhuman::config::schema::{
    CapabilityProviderConfig, CapabilityProviderTrustState, NodeConfig, WhatsAppConfig,
};
use openhuman_core::openhuman::config::{Config, IMessageConfig, WebhookConfig};
use openhuman_core::openhuman::context::prompt::ConnectedIntegration;
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::javascript::NodeBootstrap;
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::security::{AuditLogger, AutonomyLevel, SecurityPolicy};
use openhuman_core::openhuman::tool_registry::ops::diagnostics_for_config;
use openhuman_core::openhuman::tool_registry::{
    all_tool_registry_controller_schemas, all_tool_registry_registered_controllers,
    capability_provider_by_id, capability_provider_diagnostics, capability_provider_registry,
    denials, get_tool, is_capability_provider_trusted_enabled, list_capability_providers,
    list_tools, normalize_capability_provider_id, registry_entries,
    CapabilityProviderRegistryError,
};
use openhuman_core::openhuman::tools::generated::{
    admit_generated_tool_definitions, generated_tools_from_definitions, GeneratedToolAdapter,
    GeneratedToolAdmissionConfig, GeneratedToolDefinition, GeneratedToolRisk,
};
use openhuman_core::openhuman::tools::local_cli::tools_wrappers_list_json;
use openhuman_core::openhuman::tools::orchestrator_tools::collect_orchestrator_tools;
use openhuman_core::openhuman::tools::{
    all_tools, all_tools_controller_schemas, all_tools_registered_controllers,
    decode_data_url_bytes, default_tools, extract_data_url, extract_saved_path,
    write_bytes_to_path, ApplyPatchTool, BrowserAction, BrowserTool, CleaningStrategy,
    ComputerUseConfig, CsvExportTool, CurrentTimeTool, DefaultToolPolicy, DetectToolsTool,
    EditFileTool, FileReadTool, FileWriteTool, GitbooksGetPageTool, GitbooksSearchTool, GlobTool,
    GrepTool, InsertSqlRecordTool, ListFilesTool, LspTool, NodeExecTool, NpmExecTool,
    PermissionLevel, PolicyDecision, ProxyConfigTool, ReadDiffTool, RunLinterTool, RunTestsTool,
    SchemaCleanr, Tool, ToolCallOptions, ToolCategory, ToolPolicy, ToolResult, ToolScope,
    UpdateApplyTool, UpdateMemoryMdTool, WebFetchTool, WorkspaceStateTool,
};

const TEST_RPC_TOKEN: &str = "tools-approval-channels-raw-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

struct Harness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    rpc_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
    backend_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

#[derive(Default)]
struct StubMemory;

#[async_trait]
impl Memory for StubMemory {
    fn name(&self) -> &str {
        "stub"
    }

    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(&self) -> Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

struct EchoGeneratedAdapter;

#[async_trait]
impl GeneratedToolAdapter for EchoGeneratedAdapter {
    fn id(&self) -> &str {
        "echo-generated"
    }

    async fn execute(
        &self,
        definition: &GeneratedToolDefinition,
        args: Value,
    ) -> Result<ToolResult> {
        Ok(ToolResult::success(
            json!({
                "tool": definition.name,
                "adapter": definition.adapter_id,
                "args": args
            })
            .to_string(),
        ))
    }
}

#[derive(Default)]
struct CapturingChannel {
    sent: Mutex<Vec<SendMessage>>,
}

#[async_trait]
impl Channel for CapturingChannel {
    fn name(&self) -> &str {
        "capture"
    }

    async fn send(&self, message: &SendMessage) -> Result<()> {
        self.sent
            .lock()
            .expect("capture lock")
            .push(message.clone());
        Ok(())
    }

    async fn listen(&self, _tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> Result<()> {
        Ok(())
    }
}

fn coverage_agent_definition(
    id: &str,
    when_to_use: &str,
    delegate_name: Option<&str>,
) -> AgentDefinition {
    AgentDefinition {
        id: id.into(),
        when_to_use: when_to_use.into(),
        display_name: None,
        system_prompt: PromptSource::Inline(String::new()),
        omit_identity: true,
        omit_memory_context: true,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: true,
        omit_memory_md: true,
        model: ModelSpec::Inherit,
        temperature: 0.4,
        tools: AgentToolScope::Wildcard,
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        max_iterations: 8,
        iteration_policy: Default::default(),
        max_result_chars: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        subagents: vec![],
        delegate_name: delegate_name.map(str::to_string),
        agent_tier: AgentTier::Worker,
        source: DefinitionSource::Builtin,
    }
}

fn coverage_connected_integration(
    toolkit: &str,
    description: &str,
    connected: bool,
) -> ConnectedIntegration {
    ConnectedIntegration {
        toolkit: toolkit.into(),
        description: description.into(),
        tools: vec![],
        gated_tools: vec![],
        connected,
        non_active_status: None,
    }
}

struct DefaultPathTool;

#[async_trait]
impl openhuman_core::openhuman::tools::Tool for DefaultPathTool {
    fn name(&self) -> &str {
        "default_path_tool"
    }

    fn description(&self) -> &str {
        "Covers default Tool trait metadata paths."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "value": { "type": "string" }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        Ok(ToolResult::success(args.to_string()))
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn ensure_rpc_auth() {
    AUTH_INIT.get_or_init(|| {
        std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN);
        let token_dir = std::env::temp_dir().join("openhuman-tools-channels-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
}

async fn serve_rpc() -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr = listener.local_addr().expect("rpc listener addr");
    let router = build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });
    (addr, join)
}

async fn serve_backend() -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind backend listener");
    let addr = listener.local_addr().expect("backend listener addr");
    let router = Router::new().route("/{*path}", any(mock_backend));
    let join = tokio::spawn(async move { axum::serve(listener, router).await });
    (addr, join)
}

async fn mock_backend(request: Request) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or_default().to_string();
    let body = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .unwrap_or_else(|_| Bytes::new());
    let json_body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or_else(|_| Value::Null)
    };

    if method == Method::GET && path == "/plain" {
        return (StatusCode::OK, "plain coverage body").into_response();
    }
    if method == Method::GET && path == "/redirect" {
        return (
            StatusCode::FOUND,
            [(axum::http::header::LOCATION, "https://example.test/next")],
            "redirecting",
        )
            .into_response();
    }
    if method == Method::POST && path == "/mcp" {
        let rpc_method = json_body
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("");
        if rpc_method == "notifications/initialized" {
            return StatusCode::NO_CONTENT.into_response();
        }
        let id = json_body.get("id").cloned().unwrap_or(json!(1));
        let result = match rpc_method {
            "initialize" => json!({
                "protocolVersion": "2025-11-25",
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "coverage-gitbooks", "version": "1.0.0" }
            }),
            "tools/list" => json!({
                "tools": [
                    {
                        "name": "searchDocumentation",
                        "description": "Search docs",
                        "inputSchema": { "type": "object", "properties": { "query": { "type": "string" } } }
                    },
                    {
                        "name": "getPage",
                        "description": "Get page",
                        "inputSchema": { "type": "object", "properties": { "url": { "type": "string" } } }
                    }
                ]
            }),
            "tools/call" => {
                let params = json_body.get("params").cloned().unwrap_or(Value::Null);
                json!({
                    "content": [{
                        "type": "text",
                        "text": format!(
                            "gitbooks mocked {}",
                            params.get("name").and_then(Value::as_str).unwrap_or("unknown")
                        )
                    }]
                })
            }
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    axum::Json(json!({ "error": format!("unexpected mcp method {rpc_method}") })),
                )
                    .into_response();
            }
        };
        return (
            [(
                axum::http::header::HeaderName::from_static("mcp-session-id"),
                "coverage-session",
            )],
            axum::Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })),
        )
            .into_response();
    }

    let payload = match (method, path.as_str()) {
        (Method::GET, "/auth/me") => json!({
            "success": true,
            "user": {
                "id": "user-e2e",
                "telegramId": "telegram-user-1",
                "discord_id": "discord-user-1"
            }
        }),
        (Method::POST, "/auth/channels/telegram/link-token") => {
            json!({ "success": true, "data": { "linkToken": "telegram-link-e2e" } })
        }
        (Method::POST, "/auth/channels/discord/link-token") => {
            json!({ "success": true, "data": { "token": "discord-link-e2e" } })
        }
        (Method::POST, "/channels/telegram/messages") => json!({
            "success": true,
            "data": { "messageId": "msg-1", "channel": "telegram", "echo": json_body }
        }),
        (Method::POST, "/channels/telegram/reactions") => json!({
            "success": true,
            "data": { "ok": true, "reaction": json_body }
        }),
        (Method::POST, "/channels/telegram/threads") => json!({
            "success": true,
            "data": { "threadId": "thread-1", "title": json_body.get("title").cloned().unwrap_or(Value::Null) }
        }),
        (Method::PATCH, "/channels/telegram/threads/thread-1") => json!({
            "success": true,
            "data": { "threadId": "thread-1", "action": json_body.get("action").cloned().unwrap_or(Value::Null) }
        }),
        (Method::GET, "/channels/telegram/threads") => json!({
            "success": true,
            "data": {
                "query": query,
                "threads": [{ "threadId": "thread-1", "active": true }]
            }
        }),
        (Method::GET, "/agent-integrations/composio/toolkits") => json!({
            "success": true,
            "data": { "toolkits": ["gmail", "github", "slack"] }
        }),
        (Method::GET, "/agent-integrations/composio/connections") => json!({
            "success": true,
            "data": {
                "connections": [
                    {
                        "id": "conn-gmail-1",
                        "toolkit": " Gmail ",
                        "status": "ACTIVE",
                        "createdAt": "2026-05-29T12:00:00Z"
                    },
                    {
                        "id": "conn-slack-pending",
                        "toolkit": "slack",
                        "status": "pending",
                        "createdAt": "2026-05-29T12:05:00Z"
                    }
                ]
            }
        }),
        (Method::POST, "/agent-integrations/composio/authorize") => json!({
            "success": true,
            "data": {
                "connectUrl": format!(
                    "https://connect.example.test/{}",
                    json_body.get("toolkit").and_then(Value::as_str).unwrap_or("unknown")
                ),
                "connectionId": "conn-new-1"
            }
        }),
        (Method::GET, "/agent-integrations/composio/tools") => json!({
            "success": true,
            "data": {
                "tools": [
                    {
                        "type": "function",
                        "function": {
                            "name": "GMAIL_FETCH_EMAILS",
                            "description": "Fetch matching Gmail messages for the user.",
                            "parameters": {
                                "type": "object",
                                "properties": {
                                    "query": { "type": "string" },
                                    "maxResults": { "type": "integer" }
                                },
                                "required": ["query"]
                            }
                        }
                    },
                    {
                        "type": "function",
                        "function": {
                            "name": "GMAIL_UNCURATED_INTERNAL",
                            "description": "Backend-only action that should be filtered out.",
                            "parameters": { "type": "object", "properties": {} }
                        }
                    },
                    {
                        "type": "function",
                        "function": {
                            "name": "SLACK_POST_MESSAGE",
                            "description": "Slack write action for an unconnected toolkit.",
                            "parameters": {
                                "type": "object",
                                "properties": { "text": { "type": "string" } },
                                "required": ["text"]
                            }
                        }
                    }
                ]
            }
        }),
        (Method::POST, "/agent-integrations/composio/execute") => json!({
            "success": true,
            "data": {
                "data": {
                    "tool": json_body.get("tool").cloned().unwrap_or(Value::Null),
                    "arguments": json_body.get("arguments").cloned().unwrap_or(Value::Null)
                },
                "successful": true,
                "error": null,
                "costUsd": 0.015,
                "markdownFormatted": "Fetched 1 matching Gmail message."
            }
        }),
        (Method::POST, "/api/v5/robotLogic/sign-token") => json!({
            "code": 0,
            "data": {
                "token": "yuanbao-token-e2e",
                "bot_id": "yuanbao-bot-e2e",
                "product": "openhuman",
                "source": "coverage",
                "duration": 120
            }
        }),
        _ => {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(json!({ "success": false, "error": format!("unhandled {path}") })),
            )
                .into_response();
        }
    };

    (StatusCode::OK, axum::Json(payload)).into_response()
}

fn write_config(openhuman_dir: &Path, api_url: &str) {
    std::fs::create_dir_all(openhuman_dir).expect("create config dir");
    let cfg = format!(
        r#"api_url = "{api_url}"
default_model = "e2e-model"

[secrets]
encrypt = false

[local_ai]
enabled = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0

[memory_tree]
embedding_strict = false

[autonomy]
level = "full"
workspace_only = false
max_actions_per_hour = 50
require_approval_for_medium_risk = false
block_high_risk_commands = false
auto_approve = []

[node]
enabled = false

[gitbooks]
enabled = false

[mcp_client]
enabled = true

[[mcp_client.servers]]
name = "filesystem"
command = "node"
args = ["server.js"]
enabled = true
allowed_tools = ["read_file"]
disallowed_tools = ["write_file"]
"#
    );
    std::fs::write(openhuman_dir.join("config.toml"), cfg).expect("write config.toml");
}

async fn setup() -> Harness {
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let workspace = home.join("openhuman-workspace");
    let (backend_addr, backend_join) = serve_backend().await;
    let api_url = format!("http://{backend_addr}");

    write_config(&workspace, &api_url);
    write_config(&home.join(".openhuman"), &api_url);

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_TELEGRAM_BOT_USERNAME", "coverage_bot"),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::unset("OPENHUMAN_LSP_ENABLED"),
    ];

    let config = Config::load_or_init()
        .await
        .expect("load config for app session seed");
    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        APP_SESSION_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
        "header.payload.local",
        HashMap::from([("user_id".to_string(), "user-e2e".to_string())]),
        true,
    )
    .expect("seed app-session token");

    let (rpc_addr, rpc_join) = serve_rpc().await;
    Harness {
        _tmp: tmp,
        _guards: guards,
        rpc_base: format!("http://{rpc_addr}"),
        rpc_join,
        backend_join,
    }
}

async fn rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("client");
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .unwrap_or_else(|err| panic!("POST {url} {method}: {err}"));
    assert_eq!(
        response.status(),
        ReqwestStatusCode::OK,
        "{method} HTTP status"
    );
    response
        .json::<Value>()
        .await
        .unwrap_or_else(|err| panic!("json for {method}: {err}"))
}

fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    if let Some(error) = value.get("error") {
        panic!("{context}: unexpected JSON-RPC error: {error}");
    }
    let result = value
        .get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {value}"));
    result.get("result").unwrap_or(result)
}

fn error_message<'a>(value: &'a Value, context: &str) -> &'a str {
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: missing error message: {value}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn composio_agent_tools_cover_backend_discovery_markdown_and_execution_paths() {
    let _lock = env_lock();
    let harness = setup().await;
    let config = Config::load_or_init()
        .await
        .expect("load config for composio tools");
    let tools = all_composio_agent_tools(&config);
    let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "composio_list_toolkits",
            "composio_list_connections",
            "composio_authorize",
            "composio_list_tools",
            "composio_execute",
        ]
    );
    assert!(tools
        .iter()
        .all(|tool| tool.category() == ToolCategory::Skill));

    let list_toolkits = tools
        .iter()
        .find(|tool| tool.name() == "composio_list_toolkits")
        .expect("list toolkits tool");
    let toolkits = list_toolkits
        .execute(json!({}))
        .await
        .expect("list toolkits executes");
    assert!(!toolkits.is_error, "{}", toolkits.output());
    assert!(toolkits.output().contains("gmail"));

    let list_connections = tools
        .iter()
        .find(|tool| tool.name() == "composio_list_connections")
        .expect("list connections tool");
    let connections = list_connections
        .execute(json!({}))
        .await
        .expect("list connections executes");
    assert!(!connections.is_error, "{}", connections.output());
    assert!(connections.output().contains("conn-gmail-1"));
    assert!(
        !connections.output().contains("conn-slack-pending"),
        "pending connections should be filtered before reaching the agent"
    );

    let list_tools = tools
        .iter()
        .find(|tool| tool.name() == "composio_list_tools")
        .expect("list tools tool");
    assert!(list_tools.supports_markdown());
    let discovered = list_tools
        .execute_with_options(
            json!({
                "toolkits": [" gmail "],
                "tags": ["readOnlyHint"],
                "include_unconnected": false
            }),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("list tools executes");
    assert!(!discovered.is_error, "{}", discovered.output());
    assert!(discovered.output().contains("GMAIL_FETCH_EMAILS"));
    assert!(!discovered.output().contains("GMAIL_UNCURATED_INTERNAL"));
    assert!(!discovered.output().contains("SLACK_POST_MESSAGE"));
    let markdown = discovered
        .markdown_formatted
        .as_deref()
        .expect("markdown rendering");
    assert!(markdown.contains("# Composio tools"));
    assert!(markdown.contains("**req:** query"));
    assert!(markdown.contains("**opt:** maxResults"));

    let authorize = tools
        .iter()
        .find(|tool| tool.name() == "composio_authorize")
        .expect("authorize tool");
    let missing_toolkit = authorize
        .execute(json!({}))
        .await
        .expect("authorize validates params");
    assert!(missing_toolkit.is_error);
    assert!(missing_toolkit.output().contains("'toolkit' is required"));
    let handoff = authorize
        .execute(json!({ "toolkit": "gmail" }))
        .await
        .expect("authorize executes");
    assert!(!handoff.is_error, "{}", handoff.output());
    assert!(handoff
        .output()
        .contains("https://connect.example.test/gmail"));
    assert!(handoff.output().contains("conn-new-1"));

    let execute = tools
        .iter()
        .find(|tool| tool.name() == "composio_execute")
        .expect("execute tool");
    let missing_action = execute
        .execute(json!({ "arguments": {} }))
        .await
        .expect("execute validates tool");
    assert!(missing_action.is_error);
    assert!(missing_action.output().contains("'tool' is required"));
    let executed = execute
        .execute(json!({
            "tool": "GMAIL_FETCH_EMAILS",
            "connection_id": "conn-gmail-1",
            "arguments": { "query": "from:alice@example.test", "maxResults": 1 }
        }))
        .await
        .expect("execute dispatches");
    assert!(!executed.is_error, "{}", executed.output());
    assert_eq!(executed.output(), "Fetched 1 matching Gmail message.");

    harness.rpc_join.abort();
    harness.backend_join.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channels_rpc_covers_credentials_managed_backend_and_error_paths() {
    let _lock = env_lock();
    let harness = setup().await;

    let list = rpc(&harness.rpc_base, 1, "openhuman.channels_list", json!({})).await;
    let channels = payload(&list, "channels_list")
        .as_array()
        .expect("channels list");
    assert!(channels
        .iter()
        .any(|channel| channel.get("id").and_then(Value::as_str) == Some("telegram")));

    let describe = rpc(
        &harness.rpc_base,
        2,
        "openhuman.channels_describe",
        json!({ "channel": " telegram " }),
    )
    .await;
    assert_eq!(
        payload(&describe, "channels_describe")
            .get("id")
            .and_then(Value::as_str),
        Some("telegram")
    );
    let unknown = rpc(
        &harness.rpc_base,
        3,
        "openhuman.channels_describe",
        json!({ "channel": "missing" }),
    )
    .await;
    assert!(error_message(&unknown, "unknown channel").contains("unknown channel"));

    let bad_mode = rpc(
        &harness.rpc_base,
        4,
        "openhuman.channels_connect",
        json!({ "channel": "telegram", "authMode": "bad_mode" }),
    )
    .await;
    assert!(error_message(&bad_mode, "bad auth mode").contains("invalid authMode"));

    let unsupported_mode = rpc(
        &harness.rpc_base,
        24,
        "openhuman.channels_connect",
        json!({ "channel": "web", "authMode": "bot_token", "credentials": {} }),
    )
    .await;
    assert!(error_message(&unsupported_mode, "unsupported auth mode").contains("does not support"));

    let non_object_creds = rpc(
        &harness.rpc_base,
        25,
        "openhuman.channels_test",
        json!({ "channel": "telegram", "authMode": "bot_token", "credentials": "bad" }),
    )
    .await;
    assert!(error_message(&non_object_creds, "non-object credentials")
        .contains("credentials must be a JSON object"));

    let telegram_managed = rpc(
        &harness.rpc_base,
        26,
        "openhuman.channels_connect",
        json!({ "channel": "telegram", "authMode": "managed_dm", "credentials": {} }),
    )
    .await;
    assert_eq!(
        payload(&telegram_managed, "connect telegram managed dm")
            .get("status")
            .and_then(Value::as_str),
        Some("pending_auth")
    );
    assert_eq!(
        payload(&telegram_managed, "connect telegram managed dm")
            .get("auth_action")
            .and_then(Value::as_str),
        Some("telegram_managed_dm")
    );

    let discord_oauth = rpc(
        &harness.rpc_base,
        27,
        "openhuman.channels_connect",
        json!({ "channel": "discord", "authMode": "oauth", "credentials": {} }),
    )
    .await;
    assert_eq!(
        payload(&discord_oauth, "connect discord oauth")
            .get("auth_action")
            .and_then(Value::as_str),
        Some("discord_oauth")
    );

    let missing_creds = rpc(
        &harness.rpc_base,
        5,
        "openhuman.channels_test",
        json!({ "channel": "telegram", "authMode": "bot_token", "credentials": {} }),
    )
    .await;
    assert!(error_message(&missing_creds, "missing creds").contains("missing required fields"));

    let test_ok = rpc(
        &harness.rpc_base,
        6,
        "openhuman.channels_test",
        json!({
            "channel": "telegram",
            "authMode": "bot_token",
            "credentials": { "bot_token": "123456:test", "allowed_users": "alice, bob" }
        }),
    )
    .await;
    assert_eq!(
        payload(&test_ok, "channels_test")
            .get("success")
            .and_then(Value::as_bool),
        Some(true)
    );

    let connect_telegram = rpc(
        &harness.rpc_base,
        7,
        "openhuman.channels_connect",
        json!({
            "channel": "telegram",
            "authMode": "bot_token",
            "credentials": { "bot_token": "123456:test", "allowed_users": "alice, bob" }
        }),
    )
    .await;
    assert_eq!(
        payload(&connect_telegram, "connect telegram")
            .get("status")
            .and_then(Value::as_str),
        Some("connected")
    );

    let connect_discord = rpc(
        &harness.rpc_base,
        8,
        "openhuman.channels_connect",
        json!({
            "channel": "discord",
            "authMode": "bot_token",
            "credentials": {
                "bot_token": "discord-token",
                "guild_id": "guild-1",
                "channel_id": "channel-1",
                "allowed_users": ["u1", "u2"],
                "listen_to_bots": true,
                "mention_only": false
            }
        }),
    )
    .await;
    assert_eq!(
        payload(&connect_discord, "connect discord")
            .get("restart_required")
            .and_then(Value::as_bool),
        Some(true)
    );

    let connect_imessage = rpc(
        &harness.rpc_base,
        9,
        "openhuman.channels_connect",
        json!({
            "channel": "imessage",
            "authMode": "managed_dm",
            "credentials": { "allowed_contacts": "mom@example.com, +15551234567" }
        }),
    )
    .await;
    assert_eq!(
        payload(&connect_imessage, "connect imessage")
            .get("status")
            .and_then(Value::as_str),
        Some("connected")
    );

    let lark_test = rpc(
        &harness.rpc_base,
        28,
        "openhuman.channels_test",
        json!({
            "channel": "lark",
            "authMode": "api_key",
            "credentials": {
                "app_id": "cli_lark",
                "app_secret": "lark-secret",
                "use_feishu": "yes",
                "allowed_users": [" user-a ", "@user-b,user-a"]
            }
        }),
    )
    .await;
    assert_eq!(
        payload(&lark_test, "test lark")
            .get("success")
            .and_then(Value::as_bool),
        Some(true)
    );

    let connect_lark = rpc(
        &harness.rpc_base,
        29,
        "openhuman.channels_connect",
        json!({
            "channel": "lark",
            "authMode": "api_key",
            "credentials": {
                "app_id": "cli_lark",
                "app_secret": "lark-secret",
                "receive_mode": "websocket",
                "port": "8080"
            }
        }),
    )
    .await;
    assert_eq!(
        payload(&connect_lark, "connect lark")
            .get("status")
            .and_then(Value::as_str),
        Some("connected")
    );

    let connect_dingtalk = rpc(
        &harness.rpc_base,
        30,
        "openhuman.channels_connect",
        json!({
            "channel": "dingtalk",
            "authMode": "api_key",
            "credentials": {
                "client_id": "ding-client",
                "client_secret": "ding-secret",
                "allowed_users": "u1\n@u2"
            }
        }),
    )
    .await;
    assert_eq!(
        payload(&connect_dingtalk, "connect dingtalk")
            .get("status")
            .and_then(Value::as_str),
        Some("connected")
    );

    let status = rpc(
        &harness.rpc_base,
        10,
        "openhuman.channels_status",
        json!({}),
    )
    .await;
    let entries = payload(&status, "channels_status")
        .as_array()
        .expect("status entries");
    assert!(entries.iter().any(|entry| {
        entry.get("channel_id").and_then(Value::as_str) == Some("telegram")
            && entry.get("connected").and_then(Value::as_bool) == Some(true)
    }));
    assert!(entries.iter().any(|entry| {
        entry.get("channel_id").and_then(Value::as_str) == Some("imessage")
            && entry.get("connected").and_then(Value::as_bool) == Some(true)
    }));
    assert!(entries.iter().any(|entry| {
        entry.get("channel_id").and_then(Value::as_str) == Some("lark")
            && entry.get("connected").and_then(Value::as_bool) == Some(true)
    }));
    assert!(entries.iter().any(|entry| {
        entry.get("channel_id").and_then(Value::as_str) == Some("dingtalk")
            && entry.get("connected").and_then(Value::as_bool) == Some(true)
    }));

    let filtered_status = rpc(
        &harness.rpc_base,
        11,
        "openhuman.channels_status",
        json!({ "channel": " discord " }),
    )
    .await;
    assert!(payload(&filtered_status, "filtered status")
        .as_array()
        .expect("filtered status entries")
        .iter()
        .all(|entry| entry.get("channel_id").and_then(Value::as_str) == Some("discord")));

    let telegram_start = rpc(
        &harness.rpc_base,
        12,
        "openhuman.channels_telegram_login_start",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&telegram_start, "telegram login start")
            .get("botUsername")
            .and_then(Value::as_str),
        Some("coverage_bot")
    );
    let telegram_check = rpc(
        &harness.rpc_base,
        13,
        "openhuman.channels_telegram_login_check",
        json!({ "linkToken": "telegram-link-e2e" }),
    )
    .await;
    assert_eq!(
        payload(&telegram_check, "telegram login check")
            .get("linked")
            .and_then(Value::as_bool),
        Some(true)
    );

    let discord_start = rpc(
        &harness.rpc_base,
        14,
        "openhuman.channels_discord_link_start",
        json!({}),
    )
    .await;
    assert!(payload(&discord_start, "discord link start")
        .get("instructions")
        .and_then(Value::as_str)
        .is_some_and(|instructions| instructions.contains("discord-link-e2e")));
    let discord_check = rpc(
        &harness.rpc_base,
        15,
        "openhuman.channels_discord_link_check",
        json!({ "linkToken": "discord-link-e2e" }),
    )
    .await;
    assert_eq!(
        payload(&discord_check, "discord link check")
            .get("linked")
            .and_then(Value::as_bool),
        Some(true)
    );

    let send = rpc(
        &harness.rpc_base,
        16,
        "openhuman.channels_send_message",
        json!({ "channel": "telegram", "message": { "text": "hello", "threadId": "thread-1" } }),
    )
    .await;
    assert_eq!(
        payload(&send, "send message")
            .get("messageId")
            .and_then(Value::as_str),
        Some("msg-1")
    );
    let reaction = rpc(
        &harness.rpc_base,
        17,
        "openhuman.channels_send_reaction",
        json!({ "channel": "telegram", "reaction": { "messageId": "msg-1", "emoji": "+1" } }),
    )
    .await;
    assert_eq!(
        payload(&reaction, "send reaction")
            .get("ok")
            .and_then(Value::as_bool),
        Some(true)
    );
    let create_thread = rpc(
        &harness.rpc_base,
        18,
        "openhuman.channels_create_thread",
        json!({ "channel": "telegram", "title": "Coverage Thread" }),
    )
    .await;
    assert_eq!(
        payload(&create_thread, "create thread")
            .get("threadId")
            .and_then(Value::as_str),
        Some("thread-1")
    );
    let update_thread = rpc(
        &harness.rpc_base,
        19,
        "openhuman.channels_update_thread",
        json!({ "channel": "telegram", "threadId": "thread-1", "action": "close" }),
    )
    .await;
    assert_eq!(
        payload(&update_thread, "update thread")
            .get("action")
            .and_then(Value::as_str),
        Some("close")
    );
    let list_threads = rpc(
        &harness.rpc_base,
        20,
        "openhuman.channels_list_threads",
        json!({ "channel": "telegram", "active": true }),
    )
    .await;
    assert_eq!(
        payload(&list_threads, "list threads")
            .pointer("/threads/0/threadId")
            .and_then(Value::as_str),
        Some("thread-1")
    );

    let bad_update = rpc(
        &harness.rpc_base,
        21,
        "openhuman.channels_update_thread",
        json!({ "channel": "telegram", "threadId": "thread-1", "action": "archive" }),
    )
    .await;
    assert!(error_message(&bad_update, "bad update action").contains("action must be"));

    let disconnect_telegram = rpc(
        &harness.rpc_base,
        22,
        "openhuman.channels_disconnect",
        json!({ "channel": "telegram", "authMode": "bot_token", "clearMemory": false }),
    )
    .await;
    assert_eq!(
        payload(&disconnect_telegram, "disconnect telegram")
            .get("disconnected")
            .and_then(Value::as_bool),
        Some(true)
    );

    let disconnect_imessage = rpc(
        &harness.rpc_base,
        23,
        "openhuman.channels_disconnect",
        json!({ "channel": "imessage", "authMode": "managed_dm", "clearMemory": false }),
    )
    .await;
    assert_eq!(
        payload(&disconnect_imessage, "disconnect imessage")
            .get("memory_chunks_deleted")
            .and_then(Value::as_u64),
        Some(0)
    );

    let disconnect_lark = rpc(
        &harness.rpc_base,
        31,
        "openhuman.channels_disconnect",
        json!({ "channel": "lark", "authMode": "api_key", "clearMemory": false }),
    )
    .await;
    assert_eq!(
        payload(&disconnect_lark, "disconnect lark")
            .get("disconnected")
            .and_then(Value::as_bool),
        Some(true)
    );

    let disconnect_dingtalk = rpc(
        &harness.rpc_base,
        32,
        "openhuman.channels_disconnect",
        json!({ "channel": "dingtalk", "authMode": "api_key", "clearMemory": false }),
    )
    .await;
    assert_eq!(
        payload(&disconnect_dingtalk, "disconnect dingtalk")
            .get("disconnected")
            .and_then(Value::as_bool),
        Some(true)
    );

    harness.rpc_join.abort();
    harness.backend_join.abort();
}

#[test]
fn tools_and_tool_registry_public_surfaces_cover_schema_and_assembly_paths() {
    let dir = tempdir().expect("tempdir");
    let mut config = Config {
        workspace_dir: dir.path().to_path_buf(),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };
    config.node.enabled = false;
    config.browser.enabled = true;
    config.http_request.allowed_domains = vec![
        "*".to_string(),
        "docs.openhuman.ai".to_string(),
        "example.com".to_string(),
    ];
    config.gitbooks.enabled = true;
    config.computer_control.enabled = true;
    config.learning.enabled = true;
    config.learning.tool_tracking_enabled = true;
    config.mcp_client.enabled = true;

    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));
    let memory: Arc<dyn Memory> = Arc::new(StubMemory);
    let tools = all_tools(
        Arc::new(config.clone()),
        &security,
        AuditLogger::disabled(),
        memory,
        &config.browser,
        &config.http_request,
        &config.workspace_dir,
        &HashMap::new(),
        &config,
    );
    let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();
    for expected in [
        "shell",
        "file_read",
        "grep",
        "browser_open",
        "browser",
        "http_request",
        "web_fetch",
        "curl",
        "gitbooks_search",
        "gitbooks_get_page",
        "mouse",
        "keyboard",
        "tool_stats",
        "screenshot",
        "image_info",
    ] {
        assert!(
            names.contains(&expected),
            "missing tool {expected}; got {names:?}"
        );
    }
    assert!(!names.contains(&"node_exec"));
    assert!(!names.contains(&"npm_exec"));

    let baseline = default_tools(security);
    assert_eq!(baseline.len(), 3);
    assert_eq!(baseline[0].scope(), ToolScope::All);
    assert_eq!(baseline[0].permission_level(), PermissionLevel::Execute);

    let wrappers = tools_wrappers_list_json();
    assert!(wrappers
        .pointer("/result/wrappers")
        .and_then(Value::as_array)
        .expect("wrapper list")
        .iter()
        .any(|wrapper| wrapper.get("name").and_then(Value::as_str) == Some("screenshot")));

    let tool_schemas = all_tools_controller_schemas();
    let tool_controllers = all_tools_registered_controllers();
    assert_eq!(tool_schemas.len(), tool_controllers.len());
    assert!(tool_schemas
        .iter()
        .any(|schema| schema.function == "web_search"));

    let registry_schemas = all_tool_registry_controller_schemas();
    let registry_controllers = all_tool_registry_registered_controllers();
    assert_eq!(registry_schemas.len(), 3);
    assert_eq!(registry_schemas.len(), registry_controllers.len());
    assert!(registry_entries()
        .iter()
        .any(|entry| entry.tool_id == "tools.web_search"));
    let listed = list_tools()
        .into_cli_compatible_json()
        .expect("list_tools json");
    let listed_tools = listed
        .get("tools")
        .and_then(Value::as_array)
        .expect("listed tools");
    let first_tool_id = listed_tools
        .first()
        .and_then(|tool| tool.get("tool_id"))
        .and_then(Value::as_str)
        .expect("first registry tool id");
    let found = get_tool(first_tool_id)
        .expect("get first registry tool")
        .into_cli_compatible_json()
        .expect("get_tool json");
    assert_eq!(
        found.get("tool_id").and_then(Value::as_str),
        Some(first_tool_id)
    );
    assert!(get_tool("  ")
        .expect_err("blank registry id should fail")
        .contains("non-empty"));
    assert!(get_tool("tools.missing")
        .expect_err("missing registry id should fail")
        .contains("tools.missing"));

    let dirty_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": {
                "anyOf": [
                    { "type": "string", "const": "alpha" },
                    { "type": "string", "const": "beta" }
                ]
            },
            "age": { "$ref": "#/$defs/Age", "description": "age field" },
            "nullable": { "type": ["string", "null"] },
            "unresolved": { "$ref": "#/$defs/Missing", "title": "missing ref" },
            "cycle": { "$ref": "#/$defs/Cycle", "description": "cycle ref" }
        },
        "$defs": {
            "Age": { "type": "integer", "minimum": 0 },
            "Cycle": { "$ref": "#/$defs/Cycle" }
        }
    });
    let gemini = SchemaCleanr::clean_for_gemini(dirty_schema.clone());
    assert_eq!(
        gemini.pointer("/properties/kind/type"),
        Some(&json!("string"))
    );
    assert_eq!(
        gemini.pointer("/properties/kind/enum"),
        Some(&json!(["alpha", "beta"]))
    );
    assert_eq!(
        gemini.pointer("/properties/age/type"),
        Some(&json!("integer"))
    );
    assert_eq!(
        gemini.pointer("/properties/age/description"),
        Some(&json!("age field"))
    );
    assert_eq!(
        gemini.pointer("/properties/nullable/type"),
        Some(&json!("string"))
    );
    assert_eq!(
        gemini.pointer("/properties/unresolved/title"),
        Some(&json!("missing ref"))
    );
    assert!(SchemaCleanr::validate(&gemini).is_ok());
    assert!(SchemaCleanr::validate(&json!("not-object")).is_err());
    assert!(SchemaCleanr::validate(&json!({ "properties": {} })).is_err());

    let anthropic = SchemaCleanr::clean(dirty_schema.clone(), CleaningStrategy::Anthropic);
    assert!(anthropic.get("$defs").is_none());
    let openai = SchemaCleanr::clean_for_openai(dirty_schema);
    assert!(openai.get("$defs").is_some());

    let policy = DefaultToolPolicy;
    assert_eq!(
        policy.evaluate("anything", &json!({ "arg": true })),
        PolicyDecision::Allow
    );

    let default_tool = DefaultPathTool;
    let spec = default_tool.spec();
    assert_eq!(spec.name, "default_path_tool");
    assert_eq!(
        spec.description,
        "Covers default Tool trait metadata paths."
    );
    assert_eq!(default_tool.permission_level(), PermissionLevel::ReadOnly);
    assert_eq!(
        default_tool.permission_level_with_args(&json!({ "value": "x" })),
        PermissionLevel::ReadOnly
    );
    assert_eq!(default_tool.scope(), ToolScope::All);
    assert_eq!(default_tool.category(), ToolCategory::System);
    assert!(!default_tool.supports_markdown());
    assert!(!default_tool.is_concurrency_safe(&json!({})));
    assert!(!default_tool.external_effect());
    assert!(!default_tool.external_effect_with_args(&json!({})));
    assert!(default_tool.generated_runtime_context(&json!({})).is_none());
    assert!(default_tool.max_result_size_chars().is_none());

    let png_data_url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    let raw_screenshot = format!(
        "noise\nScreenshot saved to: {}\n{png_data_url}\n",
        dir.path().join("shot.png").display()
    );
    assert_eq!(
        extract_data_url(&raw_screenshot).as_deref(),
        Some(png_data_url)
    );
    assert_eq!(
        extract_saved_path(&raw_screenshot).as_deref(),
        Some(dir.path().join("shot.png").as_path())
    );
    let decoded = decode_data_url_bytes(png_data_url).expect("decode png");
    assert_eq!(&decoded[..4], b"\x89PNG");
    assert!(decode_data_url_bytes("data:text/plain;base64,aGVsbG8=")
        .expect_err("non-image data URL rejected")
        .contains("invalid data URL"));
    let nested = dir.path().join("screens").join("nested").join("shot.png");
    write_bytes_to_path(&nested, &decoded).expect("write screenshot bytes");
    assert_eq!(
        std::fs::read(&nested).expect("read screenshot bytes"),
        decoded
    );

    let computer = ComputerUseConfig {
        api_key: Some("secret-key".into()),
        window_allowlist: vec!["OpenHuman".into()],
        max_coordinate_x: Some(1920),
        max_coordinate_y: Some(1080),
        ..ComputerUseConfig::default()
    };
    let debug = format!("{computer:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("secret-key"));
    let action = serde_json::to_value(BrowserAction::Screenshot {
        path: Some("shot.png".into()),
        full_page: true,
    })
    .expect("serialize browser action");
    assert_eq!(action.pointer("/screenshot/path"), Some(&json!("shot.png")));
    assert_eq!(action.pointer("/screenshot/full_page"), Some(&json!(true)));
}

#[tokio::test]
async fn orchestrator_tool_synthesis_covers_agent_and_integration_delegation_edges() {
    let mut registry = AgentDefinitionRegistry::default();
    registry.insert(coverage_agent_definition(
        "researcher",
        "Use for careful public-source research.",
        Some("research"),
    ));

    let mut orchestrator = coverage_agent_definition("orchestrator", "Route to specialists.", None);
    orchestrator.subagents = vec![
        SubagentEntry::AgentId("researcher".into()),
        SubagentEntry::AgentId("summarizer".into()),
        SubagentEntry::AgentId("missing-agent".into()),
        SubagentEntry::Skills(SkillsWildcard {
            skills: "gmail".into(),
        }),
        SubagentEntry::Skills(SkillsWildcard { skills: "*".into() }),
    ];

    let tools = collect_orchestrator_tools(
        &orchestrator,
        &registry,
        &[
            coverage_connected_integration("GMail Pro", "Send and triage mail.", true),
            coverage_connected_integration("Slack-Bot", "", true),
            coverage_connected_integration(
                "Slack.Bot",
                "Duplicate sanitized slug should be dropped.",
                true,
            ),
            coverage_connected_integration("Disconnected", "Should be skipped.", false),
        ],
    );

    let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();
    assert_eq!(names, vec!["research", "delegate_to_integrations_agent"]);

    let research = &tools[0];
    assert!(research
        .description()
        .contains("direct tools are insufficient"));
    assert!(research
        .description()
        .contains("careful public-source research"));
    assert_eq!(research.permission_level(), PermissionLevel::Execute);
    assert_eq!(research.category(), ToolCategory::System);
    assert_eq!(
        research.parameters_schema().pointer("/required/0"),
        Some(&json!("prompt"))
    );
    let missing_prompt = research
        .execute(json!({}))
        .await
        .expect("blank delegation prompt returns tool error");
    assert!(missing_prompt.is_error);
    assert!(missing_prompt.output().contains("prompt"));

    let integrations = &tools[1];
    let schema = integrations.parameters_schema();
    assert_eq!(
        schema.pointer("/properties/toolkit/enum"),
        Some(&json!(["gmail_pro", "slack_bot"]))
    );
    let description = integrations.description();
    assert!(description.contains("gmail_pro: Send and triage mail."));
    assert!(description.contains("slack_bot: External integration via Slack-Bot"));
    assert!(!description.contains("Slack.Bot"));
    assert!(!description.contains("Disconnected"));

    let missing_toolkit = integrations
        .execute(json!({ "prompt": "send a message" }))
        .await
        .expect("missing toolkit returns tool error");
    assert!(missing_toolkit.is_error);
    assert!(missing_toolkit.output().contains("toolkit"));

    let unknown_toolkit = integrations
        .execute(json!({ "toolkit": "calendar", "prompt": "create an event" }))
        .await
        .expect("unknown toolkit returns tool error");
    assert!(unknown_toolkit.is_error);
    assert!(unknown_toolkit.output().contains("gmail_pro"));
    assert!(unknown_toolkit.output().contains("slack_bot"));

    let blank_prompt = integrations
        .execute(json!({ "toolkit": "GMail-Pro", "prompt": "   " }))
        .await
        .expect("blank prompt returns tool error after slug normalization");
    assert!(blank_prompt.is_error);
    assert!(blank_prompt.output().contains("prompt"));
}

#[tokio::test]
async fn browser_tool_with_agent_browser_shim_covers_action_parser_and_command_paths() {
    let _lock = env_lock();
    let dir = tempdir().expect("tempdir");
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir(&bin_dir).expect("create fake bin dir");
    let shim_path = bin_dir.join("agent-browser");
    std::fs::write(
        &shim_path,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo agent-browser-shim; exit 0; fi\necho '{\"success\":true,\"data\":{\"ok\":true}}'\n",
    )
    .expect("write agent-browser shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod agent-browser shim");
    }

    let old_path = std::env::var("PATH").unwrap_or_default();
    let _path_guard = EnvVarGuard::set("PATH", &format!("{}:{old_path}", bin_dir.display()));

    let security = Arc::new(SecurityPolicy::from_config(
        &Config::default().autonomy,
        dir.path(),
    ));
    let tool = BrowserTool::new_with_backend(
        security,
        vec!["example.com".into()],
        Some("coverage-session".into()),
        "agent_browser".into(),
        true,
        "http://127.0.0.1:9515".into(),
        None,
        ComputerUseConfig::default(),
    );
    assert!(BrowserTool::is_agent_browser_available().await);
    assert_eq!(tool.name(), "browser");

    for args in [
        json!({ "action": "open", "url": "https://example.com/path" }),
        json!({ "action": "snapshot", "interactive_only": false, "compact": false, "depth": u64::MAX }),
        json!({ "action": "click", "selector": "@e1" }),
        json!({ "action": "fill", "selector": "#name", "value": "Ada" }),
        json!({ "action": "type", "selector": "#name", "text": " Lovelace" }),
        json!({ "action": "get_text", "selector": "main" }),
        json!({ "action": "get_title" }),
        json!({ "action": "get_url" }),
        json!({ "action": "screenshot", "path": "shot.png", "full_page": true }),
        json!({ "action": "wait", "selector": ".ready" }),
        json!({ "action": "wait", "ms": 25 }),
        json!({ "action": "wait", "text": "Loaded" }),
        json!({ "action": "press", "key": "Enter" }),
        json!({ "action": "hover", "selector": ".menu" }),
        json!({ "action": "scroll", "direction": "down", "pixels": u64::MAX }),
        json!({ "action": "is_visible", "selector": ".result" }),
        json!({ "action": "close" }),
        json!({ "action": "find", "by": "text", "value": "Submit", "find_action": "fill", "fill_value": "done" }),
    ] {
        let result = tool
            .execute(args)
            .await
            .expect("browser action should execute through shim");
        assert!(!result.is_error, "{}", result.output());
        assert!(result.output().contains("\"ok\": true"));
    }

    for (args, expected) in [
        (
            json!({ "action": "open", "url": "file:///tmp/secret.txt" }),
            "file:// URLs",
        ),
        (
            json!({ "action": "fill", "selector": "#name" }),
            "Missing 'value'",
        ),
        (
            json!({ "action": "find", "by": "text", "value": "Submit" }),
            "Missing 'find_action'",
        ),
        (
            json!({ "action": "mouse_move", "x": 1, "y": 2 }),
            "agent_browser",
        ),
        (json!({ "action": "does_not_exist" }), "Unknown action"),
    ] {
        let observed = match tool.execute(args).await {
            Ok(result) => {
                assert!(result.is_error);
                result.output().to_string()
            }
            Err(error) => error.to_string(),
        };
        assert!(
            observed.contains(expected),
            "expected {expected:?} in {observed}"
        );
    }
}

#[tokio::test]
async fn read_diff_tool_reports_empty_diff_and_git_errors() {
    let dir = tempdir().expect("tempdir");
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .expect("git init");
    std::fs::write(dir.path().join("README.md"), "coverage\n").expect("write fixture");
    std::process::Command::new("git")
        .args(["add", "README.md"])
        .current_dir(dir.path())
        .output()
        .expect("git add");
    std::process::Command::new("git")
        .args([
            "-c",
            "user.email=coverage@example.test",
            "-c",
            "user.name=Coverage",
            "commit",
            "-m",
            "initial",
        ])
        .current_dir(dir.path())
        .output()
        .expect("git commit");

    let tool = ReadDiffTool::new(dir.path().to_path_buf());
    assert_eq!(tool.name(), "read_diff");
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    assert_eq!(
        tool.parameters_schema().pointer("/properties/staged/type"),
        Some(&json!("boolean"))
    );
    let empty = tool.execute(json!({})).await.expect("empty diff");
    assert!(!empty.is_error);
    assert!(empty.output().contains("No changes found"));

    std::fs::write(dir.path().join("README.md"), "coverage\nchanged\n").expect("edit fixture");
    let diff = tool
        .execute(json!({ "path_filter": "README.md" }))
        .await
        .expect("diff");
    assert!(!diff.is_error);
    assert!(diff.output().contains("+changed"));

    let missing_ref = tool
        .execute(json!({ "base": "refs/heads/does-not-exist" }))
        .await
        .expect("git error is a tool result");
    assert!(missing_ref.is_error);
    assert!(missing_ref.output().contains("does-not-exist"));
}

#[tokio::test]
async fn channels_public_helpers_cover_cli_trait_defaults_and_webhook_parsers() {
    let cli = CliChannel::new();
    assert_eq!(cli.name(), "cli");
    assert!(cli
        .send(&SendMessage::new("hello from coverage", "stdout"))
        .await
        .is_ok());
    assert!(cli.health_check().await);
    assert!(cli.start_typing("user").await.is_ok());
    assert!(cli.stop_typing("user").await.is_ok());
    assert!(!cli.supports_reactions());
    assert!(!cli.supports_draft_updates());
    assert!(cli
        .send_draft(&SendMessage::new("draft", "user"))
        .await
        .expect("default send_draft")
        .is_none());
    assert!(cli.update_draft("user", "msg-1", "draft").await.is_ok());
    assert!(cli
        .finalize_draft("user", "msg-1", "final", Some("thread-1"))
        .await
        .is_ok());

    let threaded = SendMessage::with_subject("body", "recipient", "subject")
        .in_thread(Some("thread-1".to_string()));
    assert_eq!(threaded.subject.as_deref(), Some("subject"));
    assert_eq!(threaded.thread_ts.as_deref(), Some("thread-1"));

    let whatsapp = WhatsAppChannel::new(
        "token".into(),
        "phone-id".into(),
        "verify-me".into(),
        vec!["+15551234567".into()],
    );
    assert_eq!(whatsapp.name(), "whatsapp");
    assert_eq!(whatsapp.verify_token(), "verify-me");
    let whatsapp_messages = whatsapp.parse_webhook_payload(&json!({
        "entry": [{
            "changes": [{
                "value": {
                    "messages": [{
                        "id": "wamid.1",
                        "from": "15551234567",
                        "timestamp": "1780000000",
                        "text": { "body": "hi from whatsapp" }
                    }, {
                        "id": "wamid.2",
                        "from": "15550000000",
                        "timestamp": "1780000001",
                        "text": { "body": "blocked" }
                    }, {
                        "id": "wamid.3",
                        "from": "15551234567",
                        "timestamp": "bad",
                        "image": { "id": "media" }
                    }]
                }
            }]
        }]
    }));
    assert_eq!(whatsapp_messages.len(), 1);
    assert_eq!(whatsapp_messages[0].sender, "+15551234567");
    assert_eq!(whatsapp_messages[0].content, "hi from whatsapp");
    assert!(whatsapp.parse_webhook_payload(&json!({})).is_empty());

    let linq = LinqChannel::new("linq-token".into(), "+15557654321".into(), vec!["*".into()]);
    assert_eq!(linq.name(), "linq");
    assert_eq!(linq.phone_number(), "+15557654321");
    let linq_messages = linq.parse_webhook_payload(&json!({
        "event_type": "message.received",
        "data": {
            "chat_id": "chat-1",
            "from": "15551234567",
            "recipient_phone": "+15557654321",
            "service": "iMessage",
            "is_from_me": false,
            "message": {
                "id": "linq-msg-1",
                "parts": [
                    { "type": "text", "value": "hello" },
                    { "type": "media", "url": "https://cdn.example.test/image.png", "mime_type": "image/png" },
                    { "type": "media", "url": "https://cdn.example.test/file.pdf", "mime_type": "application/pdf" }
                ]
            }
        }
    }));
    assert_eq!(linq_messages.len(), 1);
    assert_eq!(linq_messages[0].sender, "+15551234567");
    assert!(linq_messages[0].content.contains("hello"));
    assert!(linq_messages[0]
        .content
        .contains("[IMAGE:https://cdn.example.test/image.png]"));
    assert!(linq
        .parse_webhook_payload(&json!({ "event_type": "message.sent" }))
        .is_empty());
    assert!(linq
        .parse_webhook_payload(&json!({
            "event_type": "message.received",
            "data": { "is_from_me": true }
        }))
        .is_empty());
}

#[tokio::test]
async fn channel_provider_public_paths_cover_pre_network_errors_and_utilities() {
    let dingtalk = DingTalkChannel::new("client".into(), "secret".into(), vec!["*".into()]);
    assert_eq!(dingtalk.name(), "dingtalk");
    let missing_webhook = dingtalk
        .send(&SendMessage::new("reply", "chat-without-session"))
        .await
        .expect_err("dingtalk send should fail before network without a session webhook");
    assert!(missing_webhook.to_string().contains("No session webhook"));

    let slack = SlackChannel::new("xoxb-test".into(), None, vec!["U1".into()]);
    assert_eq!(slack.name(), "slack");
    let (tx, _rx) = tokio::sync::mpsc::channel::<ChannelMessage>(1);
    let slack_listen = slack
        .listen(tx)
        .await
        .expect_err("slack listen should require channel_id before polling");
    assert!(slack_listen.to_string().contains("channel_id required"));

    let mattermost = MattermostChannel::new(
        "https://mattermost.example.test///".into(),
        "token".into(),
        None,
        vec!["*".into()],
        true,
        false,
    );
    assert_eq!(mattermost.name(), "mattermost");
    let (tx, _rx) = tokio::sync::mpsc::channel::<ChannelMessage>(1);
    let mattermost_listen = mattermost
        .listen(tx)
        .await
        .expect_err("mattermost listen should require channel_id before polling");
    assert!(mattermost_listen
        .to_string()
        .contains("channel_id required"));
    assert!(mattermost.stop_typing("channel:root").await.is_ok());

    let imessage = IMessageChannel::new(vec!["friend@example.test".into()]);
    assert_eq!(imessage.name(), "imessage");
    let invalid_target = imessage
        .send(&SendMessage::new("hello", "not a valid recipient"))
        .await
        .expect_err("invalid iMessage target should be rejected before osascript");
    assert!(invalid_target
        .to_string()
        .contains("Invalid iMessage target"));

    let mut email_config = EmailConfig {
        allowed_senders: vec![
            "ALICE@example.test".into(),
            "@trusted.test".into(),
            "domain.test".into(),
        ],
        ..EmailConfig::default()
    };
    assert_eq!(email_config.imap_port, 993);
    assert_eq!(email_config.smtp_port, 465);
    assert_eq!(email_config.imap_folder, "INBOX");
    assert!(email_config.smtp_tls);
    let email = EmailChannel::new(email_config.clone());
    assert_eq!(email.name(), "email");
    assert!(email.is_sender_allowed("alice@example.test"));
    assert!(email.is_sender_allowed("alerts@trusted.test"));
    assert!(email.is_sender_allowed("bot@domain.test"));
    assert!(!email.is_sender_allowed("mallory@example.test"));
    assert_eq!(
        EmailChannel::strip_html("<div>Hello<br><strong>friend</strong></div>"),
        "Hellofriend"
    );
    email_config.allowed_senders = vec!["*".into()];
    assert!(EmailChannel::new(email_config).is_sender_allowed("anyone@elsewhere.test"));

    let qq = QQChannel::new("app".into(), "secret".into(), vec!["*".into()]);
    assert_eq!(qq.name(), "qq");
    let signal = SignalChannel::new(
        "http://127.0.0.1:1///".into(),
        "+15551234567".into(),
        Some("dm".into()),
        vec!["*".into()],
        true,
        true,
    );
    assert_eq!(signal.name(), "signal");
}

#[tokio::test]
async fn web_channel_public_paths_cover_event_delivery_and_validation_errors() {
    let mut rx = openhuman_core::openhuman::channels::web::subscribe_web_channel_events();
    openhuman_core::openhuman::channels::web::publish_web_channel_event(WebChannelEvent {
        event: "coverage_event".to_string(),
        client_id: "client-1".to_string(),
        thread_id: "thread-1".to_string(),
        request_id: "request-1".to_string(),
        message: Some("hello web channel".to_string()),
        ..Default::default()
    });
    let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("web channel event should be delivered")
        .expect("web channel event");
    assert_eq!(event.event, "coverage_event");
    assert_eq!(event.client_id, "client-1");
    assert_eq!(event.thread_id, "thread-1");
    assert_eq!(event.message.as_deref(), Some("hello web channel"));

    assert_eq!(
        openhuman_core::openhuman::channels::web::start_chat(
            "", "thread-1", "hello", None, None, None, None,
        )
        .await
        .expect_err("blank client_id"),
        "client_id is required"
    );
    assert_eq!(
        openhuman_core::openhuman::channels::web::start_chat(
            "client-1", "", "hello", None, None, None, None,
        )
        .await
        .expect_err("blank thread_id"),
        "thread_id is required"
    );
    assert_eq!(
        openhuman_core::openhuman::channels::web::start_chat(
            "client-1", "thread-1", "   ", None, None, None, None,
        )
        .await
        .expect_err("blank message"),
        "message is required"
    );

    assert_eq!(
        openhuman_core::openhuman::channels::web::cancel_chat("", "thread-1")
            .await
            .expect_err("blank cancel client_id"),
        "client_id is required"
    );
    assert_eq!(
        openhuman_core::openhuman::channels::web::cancel_chat("client-1", "")
            .await
            .expect_err("blank cancel thread_id"),
        "thread_id is required"
    );
    assert!(
        openhuman_core::openhuman::channels::web::cancel_chat("client-1", "thread-1")
            .await
            .expect("cancel with no in-flight request")
            .is_none()
    );
    openhuman_core::openhuman::channels::web::invalidate_thread_sessions("thread-1").await;
    assert!(
        openhuman_core::openhuman::channels::web::in_flight_entries_for_test()
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn proactive_subscriber_routes_web_and_active_external_channel_without_network() {
    async fn recv_proactive_thread(
        rx: &mut tokio::sync::broadcast::Receiver<WebChannelEvent>,
        thread_id: &str,
    ) -> WebChannelEvent {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let now = tokio::time::Instant::now();
            assert!(now < deadline, "timed out waiting for {thread_id}");
            let remaining = deadline - now;
            let event = tokio::time::timeout(remaining, rx.recv())
                .await
                .expect("proactive web event should be delivered")
                .expect("proactive web event");
            if event.event == "proactive_message" && event.thread_id == thread_id {
                return event;
            }
        }
    }

    let mut rx = openhuman_core::openhuman::channels::web::subscribe_web_channel_events();
    let capture = Arc::new(CapturingChannel::default());
    let mut channels: HashMap<String, Arc<dyn Channel>> = HashMap::new();
    channels.insert("capture".into(), capture.clone());

    let subscriber = ProactiveMessageSubscriber::new(Arc::new(channels), Some("capture".into()));
    assert_eq!(subscriber.name(), "channels::proactive");
    assert_eq!(subscriber.domains(), Some(&["cron"][..]));

    subscriber
        .handle(&DomainEvent::AgentTurnStarted {
            session_id: "ignored".into(),
            channel: "web".into(),
        })
        .await;
    assert!(capture.sent.lock().expect("capture lock").is_empty());

    subscriber
        .handle(&DomainEvent::ProactiveMessageRequested {
            source: "cron:coverage".into(),
            message: "send through active external channel".into(),
            job_name: Some("coverage_job".into()),
        })
        .await;

    let web_event = recv_proactive_thread(&mut rx, "proactive:coverage_job").await;
    assert_eq!(web_event.event, "proactive_message");
    assert_eq!(web_event.client_id, "system");
    assert_eq!(web_event.thread_id, "proactive:coverage_job");
    assert_eq!(
        web_event.full_response.as_deref(),
        Some("send through active external channel")
    );
    assert_eq!(web_event.success, Some(true));

    let sent = capture.sent.lock().expect("capture lock").clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].content, "send through active external channel");
    assert_eq!(sent[0].recipient, "");

    subscriber.set_active_channel(Some("web".into()));
    subscriber
        .handle(&DomainEvent::ProactiveMessageRequested {
            source: "cron:web-only".into(),
            message: "web skips external duplicate".into(),
            job_name: None,
        })
        .await;
    let web_only_event = recv_proactive_thread(&mut rx, "proactive:system").await;
    assert_eq!(web_only_event.thread_id, "proactive:system");
    assert_eq!(
        web_only_event.full_response.as_deref(),
        Some("web skips external duplicate")
    );
    assert_eq!(capture.sent.lock().expect("capture lock").len(), 1);

    subscriber.set_active_channel(Some("missing".into()));
    subscriber
        .handle(&DomainEvent::ProactiveMessageRequested {
            source: "cron:missing".into(),
            message: "missing external channel is logged only".into(),
            job_name: Some("missing".into()),
        })
        .await;
    let missing_event = recv_proactive_thread(&mut rx, "proactive:missing").await;
    assert_eq!(missing_event.thread_id, "proactive:missing");
    assert_eq!(capture.sent.lock().expect("capture lock").len(), 1);
}

#[test]
fn yuanbao_shared_types_cover_message_extractors_and_state_variants() {
    let frame = YuanbaoConnFrame {
        cmd_type: 2,
        cmd: "push".into(),
        module: "yuanbao_openclaw_proxy".into(),
        seq_no: 42,
        msg_id: "frame-1".into(),
        need_ack: true,
        status: 0,
        data: vec![1, 2, 3],
    };
    assert_eq!(frame.cmd_type, 2);
    assert_eq!(frame.cmd, "push");
    assert_eq!(frame.module, "yuanbao_openclaw_proxy");
    assert_eq!(frame.seq_no, 42);
    assert_eq!(frame.msg_id, "frame-1");
    assert!(frame.need_ack);
    assert_eq!(frame.status, 0);
    assert_eq!(frame.data, vec![1, 2, 3]);

    let text = YuanbaoMsgBodyElement {
        msg_type: "TIMTextElem".into(),
        msg_content: YuanbaoMsgContent {
            text: Some("first".into()),
            ..Default::default()
        },
    };
    let second_text = YuanbaoMsgBodyElement {
        msg_type: "TIMTextElem".into(),
        msg_content: YuanbaoMsgContent {
            text: Some("second".into()),
            ..Default::default()
        },
    };
    let image = YuanbaoMsgBodyElement {
        msg_type: "TIMImageElem".into(),
        msg_content: YuanbaoMsgContent {
            uuid: Some("uuid".into()),
            image_format: Some(3),
            data: Some("inline-data".into()),
            desc: Some("image desc".into()),
            ext: Some("{}".into()),
            sound: Some("sound-id".into()),
            image_info_array: vec![
                YuanbaoImageInfo {
                    image_type: 1,
                    size: 100,
                    width: 640,
                    height: 480,
                    url: "https://cdn.example.test/original.png".into(),
                },
                YuanbaoImageInfo {
                    image_type: 3,
                    size: 10,
                    width: 64,
                    height: 48,
                    url: String::new(),
                },
            ],
            index: Some(1),
            url: Some("https://cdn.example.test/original.png".into()),
            file_size: Some(100),
            file_name: Some("photo.png".into()),
            ..Default::default()
        },
    };

    let message = YuanbaoInboundMessage {
        callback_command: "C2C.Callback".into(),
        from_account: "sender-1".into(),
        to_account: "bot-1".into(),
        sender_nickname: "Alice".into(),
        group_id: "group-id".into(),
        group_code: "group-code".into(),
        group_name: "Coverage Group".into(),
        msg_seq: 7,
        msg_random: 9,
        msg_time: 1_780_000_000,
        msg_key: "key".into(),
        msg_id: "msg-1".into(),
        msg_body: vec![text, image, second_text],
        cloud_custom_data: "{}".into(),
        event_time: 1_780_000_001,
        bot_owner_id: "owner".into(),
        recall_msg_seq_list: vec![YuanbaoImMsgSeq {
            msg_seq: 6,
            msg_id: "old-msg".into(),
        }],
        claw_msg_type: 1,
        private_from_group_code: "private-group".into(),
        trace_id: "trace".into(),
    };
    assert!(message.is_group());
    assert!(message.is_recall());
    assert_eq!(message.chat_id(), "group-code");
    assert_eq!(message.extract_text(), "first\nsecond");
    assert_eq!(
        message.extract_image_urls(),
        vec!["https://cdn.example.test/original.png".to_string()]
    );
    assert_eq!(message.callback_command, "C2C.Callback");
    assert_eq!(message.to_account, "bot-1");
    assert_eq!(message.sender_nickname, "Alice");
    assert_eq!(message.group_id, "group-id");
    assert_eq!(message.group_name, "Coverage Group");
    assert_eq!(message.msg_seq, 7);
    assert_eq!(message.msg_random, 9);
    assert_eq!(message.msg_time, 1_780_000_000);
    assert_eq!(message.msg_key, "key");
    assert_eq!(message.cloud_custom_data, "{}");
    assert_eq!(message.event_time, 1_780_000_001);
    assert_eq!(message.bot_owner_id, "owner");
    assert_eq!(message.claw_msg_type, 1);
    assert_eq!(message.private_from_group_code, "private-group");
    assert_eq!(message.trace_id, "trace");

    let dm = YuanbaoInboundMessage {
        from_account: "dm-user".into(),
        ..Default::default()
    };
    assert!(!dm.is_group());
    assert!(!dm.is_recall());
    assert_eq!(dm.chat_id(), "dm-user");
    assert!(dm.extract_text().is_empty());
    assert!(dm.extract_image_urls().is_empty());

    let group_source = YuanbaoSource {
        from_account: "sender".into(),
        sender_nickname: "Alice".into(),
        group_code: "group-code".into(),
        is_group: true,
    };
    assert_eq!(group_source.reply_target(), "g:group-code");
    assert_eq!(group_source.sender_nickname, "Alice");
    let dm_source = YuanbaoSource {
        from_account: "sender".into(),
        is_group: false,
        ..Default::default()
    };
    assert_eq!(dm_source.reply_target(), "sender");

    for kind in [
        YuanbaoMessageKind::Text,
        YuanbaoMessageKind::Image,
        YuanbaoMessageKind::File,
        YuanbaoMessageKind::Voice,
        YuanbaoMessageKind::Mixed,
        YuanbaoMessageKind::Recall,
    ] {
        assert_eq!(kind, kind);
    }
    assert_eq!(YuanbaoMessageKind::default(), YuanbaoMessageKind::Text);

    let group_info = YuanbaoGroupInfo {
        code: 0,
        message: "ok".into(),
        group_name: "Coverage Group".into(),
        owner_id: "owner".into(),
        owner_nickname: "Owner".into(),
        member_count: 2,
    };
    assert_eq!(group_info.owner_nickname, "Owner");
    assert_eq!(group_info.member_count, 2);
    let member = YuanbaoGroupMember {
        user_id: "member-1".into(),
        nickname: "Member".into(),
        role: 1,
        join_time: 123,
        name_card: "Card".into(),
    };
    let page = YuanbaoGroupMemberListPage {
        code: 0,
        message: "ok".into(),
        members: vec![member],
        next_offset: 20,
        is_complete: false,
    };
    assert_eq!(page.members[0].role, 1);
    assert_eq!(page.next_offset, 20);
    assert!(!page.is_complete);

    let account = YuanbaoAccount {
        uid: "bot".into(),
        nickname: "Coverage Bot".into(),
        connect_id: "connect".into(),
    };
    let account_json = serde_json::to_string(&account).expect("serialize yuanbao account");
    assert!(account_json.contains("Coverage Bot"));
    let decoded: YuanbaoAccount =
        serde_json::from_str(&account_json).expect("deserialize yuanbao account");
    assert_eq!(decoded.connect_id, "connect");

    assert_ne!(
        YuanbaoConnectionState::Disconnected,
        YuanbaoConnectionState::Connecting
    );
    assert_eq!(
        YuanbaoConnectionState::Authenticating,
        YuanbaoConnectionState::Authenticating
    );
    assert_eq!(
        YuanbaoConnectionState::Connected,
        YuanbaoConnectionState::Connected
    );
    assert_eq!(
        YuanbaoConnectionState::Reconnecting,
        YuanbaoConnectionState::Reconnecting
    );
}

fn yuanbao_pipeline_config() -> YuanbaoConfig {
    YuanbaoConfig {
        app_key: "app-key".into(),
        app_secret: String::new(),
        token: "token".into(),
        ws_domain: "wss://yuanbao.example.test/ws".into(),
        api_domain: "https://yuanbao.example.test".into(),
        bot_id: "bot-uid".into(),
        bot_name: "CoverageBot".into(),
        owner_id: "owner-uid".into(),
        dm_access: "open".into(),
        group_access: "open".into(),
        group_at_required: true,
        ..YuanbaoConfig::default()
    }
}

fn yuanbao_pipeline(config: &YuanbaoConfig) -> InboundPipeline {
    let state = PipelineState::new(config, config.bot_id.clone());
    InboundPipeline::new(state)
}

fn yuanbao_inbound_json(fields: Value) -> Vec<u8> {
    let mut base = json!({
        "callback_command": "C2C.Callback",
        "from_account": "alice-uid",
        "to_account": "bot-uid",
        "sender_nickname": "Alice",
        "msg_seq": 1,
        "msg_time": 1_780_000_000u64,
        "msg_id": "msg-coverage-1",
        "msg_body": [{
            "msg_type": "TIMTextElem",
            "msg_content": { "text": "hello CoverageBot" }
        }]
    });
    let obj = base.as_object_mut().expect("base object");
    for (key, value) in fields.as_object().expect("fields object") {
        obj.insert(key.clone(), value.clone());
    }
    serde_json::to_vec(&base).expect("serialize yuanbao inbound json")
}

#[tokio::test]
async fn yuanbao_channel_and_inbound_pipeline_cover_dispatch_filter_and_error_paths() {
    let mut config = yuanbao_pipeline_config();
    config.apply_env_defaults();
    assert!(config.validate().is_ok());
    assert_eq!(config.api_domain, "https://yuanbao.example.test");
    assert_eq!(config.ws_domain, "wss://yuanbao.example.test/ws");

    let channel = YuanbaoChannel::new(config.clone()).expect("construct yuanbao channel");
    assert_eq!(channel.name(), "yuanbao");
    assert!(channel.supports_draft_updates());
    assert!(!channel.supports_reactions());
    assert!(!channel.health_check().await);
    let draft = channel
        .send_draft(&SendMessage::new("draft body", "alice-uid"))
        .await
        .expect("yuanbao draft marker");
    assert_eq!(draft.as_deref(), Some("yb-draft:alice-uid"));
    assert!(channel
        .update_draft("alice-uid", "yb-draft:alice-uid", "partial")
        .await
        .is_ok());

    let pipeline = yuanbao_pipeline(&config);
    match pipeline
        .process(&yuanbao_inbound_json(json!({
            "msg_id": "dm-1",
            "msg_body": [{
                "msg_type": "TIMTextElem",
                "msg_content": { "text": "plain dm" }
            }]
        })))
        .await
    {
        PipelineOutcome::Dispatch(ctx) => {
            assert_eq!(ctx.text, "plain dm");
            assert_eq!(ctx.source.reply_target(), "alice-uid");
            assert_eq!(ctx.kind, YuanbaoMessageKind::Text);
            assert!(!ctx.is_owner_command);
        }
        other => panic!("expected DM dispatch, got {other:?}"),
    }

    let duplicate = pipeline
        .process(&yuanbao_inbound_json(json!({
            "msg_id": "dm-1",
            "msg_body": [{
                "msg_type": "TIMTextElem",
                "msg_content": { "text": "plain dm duplicate" }
            }]
        })))
        .await;
    assert!(matches!(duplicate, PipelineOutcome::Filtered("dedup")));

    let recall = pipeline
        .process(&yuanbao_inbound_json(json!({
            "msg_id": "recall-1",
            "recall_msg_seq_list": [{ "msg_seq": 1, "msg_id": "old" }]
        })))
        .await;
    assert!(matches!(recall, PipelineOutcome::Filtered("recall_guard")));

    let placeholder = pipeline
        .process(&yuanbao_inbound_json(json!({
            "msg_id": "placeholder-1",
            "msg_body": [{
                "msg_type": "TIMTextElem",
                "msg_content": { "text": "[image]" }
            }]
        })))
        .await;
    assert!(matches!(
        placeholder,
        PipelineOutcome::Filtered("placeholder_filter")
    ));

    let mut closed_config = config.clone();
    closed_config.dm_access = "closed".into();
    let closed = yuanbao_pipeline(&closed_config)
        .process(&yuanbao_inbound_json(json!({ "msg_id": "closed-1" })))
        .await;
    assert!(matches!(closed, PipelineOutcome::Filtered("access_guard")));

    let group_without_mention = pipeline
        .process(&yuanbao_inbound_json(json!({
            "callback_command": "Group.Callback",
            "from_account": "group-user",
            "group_code": "group-1",
            "msg_id": "group-no-at",
            "msg_body": [{
                "msg_type": "TIMTextElem",
                "msg_content": { "text": "hello group" }
            }]
        })))
        .await;
    assert!(matches!(
        group_without_mention,
        PipelineOutcome::Filtered("group_at_guard")
    ));

    match pipeline
        .process(&yuanbao_inbound_json(json!({
            "callback_command": "Group.Callback",
            "from_account": "group-user",
            "group_code": "group-1",
            "sender_nickname": "Group Alice",
            "msg_id": "group-at-1",
            "msg_body": [{
                "msg_type": "TIMTextElem",
                "msg_content": { "text": "@CoverageBot summarize this" }
            }]
        })))
        .await
    {
        PipelineOutcome::Dispatch(ctx) => {
            assert!(ctx.source.is_group);
            assert_eq!(ctx.source.reply_target(), "g:group-1");
            assert_eq!(ctx.text, "summarize this");
            assert!(ctx.is_at_bot);
        }
        other => panic!("expected group dispatch, got {other:?}"),
    }

    match pipeline
        .process(&yuanbao_inbound_json(json!({
            "callback_command": "Group.Callback",
            "from_account": "owner-uid",
            "group_code": "group-1",
            "msg_id": "owner-command-1",
            "msg_body": [{
                "msg_type": "TIMTextElem",
                "msg_content": { "text": "/status" }
            }]
        })))
        .await
    {
        PipelineOutcome::Dispatch(ctx) => {
            assert!(ctx.is_owner_command);
            assert_eq!(ctx.text, "/status");
        }
        other => panic!("expected owner command dispatch, got {other:?}"),
    }

    match pipeline
        .process(&yuanbao_inbound_json(json!({
            "msg_id": "mixed-1",
            "msg_body": [{
                "msg_type": "TIMTextElem",
                "msg_content": { "text": "caption" }
            }, {
                "msg_type": "TIMImageElem",
                "msg_content": {
                    "image_info_array": [{
                        "image_type": 1,
                        "size": 10,
                        "width": 4,
                        "height": 3,
                        "url": "https://cdn.example.test/cat.png"
                    }]
                }
            }]
        })))
        .await
    {
        PipelineOutcome::Dispatch(ctx) => {
            assert_eq!(ctx.kind, YuanbaoMessageKind::Mixed);
            assert_eq!(ctx.image_urls, vec!["https://cdn.example.test/cat.png"]);
        }
        other => panic!("expected mixed dispatch, got {other:?}"),
    }

    match pipeline.process(b"{not valid json").await {
        PipelineOutcome::Failed(err) => assert!(err.to_string().contains("decode")),
        other => panic!("expected decode failure, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_registry_rpc_controllers_cover_list_get_diagnostics_and_errors() {
    let _lock = env_lock();
    let harness = setup().await;

    let list = rpc(
        &harness.rpc_base,
        101,
        "openhuman.tool_registry_list",
        json!({}),
    )
    .await;
    let tools = payload(&list, "tool_registry_list")
        .get("tools")
        .and_then(Value::as_array)
        .expect("registry tools");
    assert!(tools
        .iter()
        .any(|tool| tool.get("tool_id").and_then(Value::as_str) == Some("tools.web_search")));

    let found = rpc(
        &harness.rpc_base,
        102,
        "openhuman.tool_registry_get",
        json!({ "tool_id": " tools.web_search " }),
    )
    .await;
    assert_eq!(
        payload(&found, "tool_registry_get")
            .get("tool_id")
            .and_then(Value::as_str),
        Some("tools.web_search")
    );

    let missing_id = rpc(
        &harness.rpc_base,
        103,
        "openhuman.tool_registry_get",
        json!({ "tool_id": "   " }),
    )
    .await;
    assert!(error_message(&missing_id, "tool_registry_get blank id")
        .contains("tool_id must be a non-empty string"));
    let unknown_tool = rpc(
        &harness.rpc_base,
        104,
        "openhuman.tool_registry_get",
        json!({ "tool_id": "tools.not_real" }),
    )
    .await;
    assert!(
        error_message(&unknown_tool, "tool_registry_get missing tool").contains("tool not found")
    );

    let diagnostics = rpc(
        &harness.rpc_base,
        105,
        "openhuman.tool_registry_diagnostics",
        json!({}),
    )
    .await;
    let diagnostics = payload(&diagnostics, "tool_registry_diagnostics");
    assert!(diagnostics
        .get("total_tools")
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 0));
    assert!(diagnostics
        .pointer("/mcp_allowlists/server_count")
        .and_then(Value::as_u64)
        .is_some());
    assert!(diagnostics
        .pointer("/mcp_write_audit/enabled")
        .and_then(Value::as_bool)
        .is_some());

    harness.rpc_join.abort();
    harness.backend_join.abort();
}

#[test]
fn tool_registry_provider_and_denial_paths_cover_diagnostics() {
    assert_eq!(
        normalize_capability_provider_id(" Trusted Runtime.Provider "),
        Ok("trusted-runtime.provider".to_string())
    );
    assert!(matches!(
        normalize_capability_provider_id("!!!"),
        Err(CapabilityProviderRegistryError::InvalidId { .. })
    ));
    let empty_provider_id_error =
        normalize_capability_provider_id("   ").expect_err("empty provider id should fail");
    assert_eq!(
        empty_provider_id_error.to_string(),
        "invalid provider id: \"\""
    );
    assert!(matches!(
        normalize_capability_provider_id(&"x".repeat(128)),
        Err(CapabilityProviderRegistryError::InvalidId { .. })
    ));

    let mut config = Config::default();
    config.capability_providers = vec![
        CapabilityProviderConfig {
            id: "Trusted Runtime.Provider".into(),
            display_name: "  Trusted Runtime  ".into(),
            source_uri: Some(" https://example.test/catalog.json ".into()),
            source_digest: Some(" sha256:abc123 ".into()),
            trust_state: CapabilityProviderTrustState::Trusted,
            enabled: true,
        },
        CapabilityProviderConfig {
            id: "Disabled Provider".into(),
            display_name: String::new(),
            source_uri: Some("   ".into()),
            source_digest: None,
            trust_state: CapabilityProviderTrustState::Untrusted,
            enabled: false,
        },
    ];

    let registry = capability_provider_registry(&config).expect("provider registry");
    assert_eq!(registry.list().len(), 2);
    assert!(registry.is_trusted_enabled("trusted runtime.provider"));
    assert!(!registry.is_trusted_enabled("disabled provider"));
    assert_eq!(
        registry
            .get("disabled provider")
            .expect("disabled provider")
            .display_name,
        "disabled-provider"
    );
    assert_eq!(
        list_capability_providers(&config)
            .expect("list providers")
            .len(),
        2
    );
    assert_eq!(
        capability_provider_by_id(&config, "TRUSTED RUNTIME.PROVIDER")
            .expect("provider lookup")
            .expect("provider")
            .source_uri
            .as_deref(),
        Some("https://example.test/catalog.json")
    );
    assert!(is_capability_provider_trusted_enabled(
        &config,
        "trusted runtime.provider"
    ));
    let provider_diagnostics = capability_provider_diagnostics(&config);
    assert_eq!(provider_diagnostics.total_providers, 2);
    assert_eq!(provider_diagnostics.enabled_providers, 1);
    assert_eq!(provider_diagnostics.trusted_providers, 1);
    assert_eq!(provider_diagnostics.trusted_enabled_providers, 1);

    let mut duplicate_config = config.clone();
    duplicate_config
        .capability_providers
        .push(CapabilityProviderConfig {
            id: "Trusted Runtime.Provider".into(),
            ..CapabilityProviderConfig::default()
        });
    assert!(matches!(
        capability_provider_registry(&duplicate_config),
        Err(CapabilityProviderRegistryError::DuplicateId { .. })
    ));
    assert!(!is_capability_provider_trusted_enabled(
        &duplicate_config,
        "trusted runtime.provider"
    ));
    let duplicate_diagnostics = capability_provider_diagnostics(&duplicate_config);
    assert_eq!(duplicate_diagnostics.total_providers, 3);
    assert_eq!(duplicate_diagnostics.registry_errors.len(), 1);

    denials::record(" coverage.tool ", "", "", "");
    denials::record(
        "secret.tool",
        "generated",
        "execute",
        "blocked Bearer super-secret-token",
    );
    let recent_denials = denials::list(2);
    assert_eq!(recent_denials[0].tool_name, "secret.tool");
    assert_eq!(recent_denials[0].reason, "[redacted: sensitive content]");
    assert_eq!(recent_denials[1].policy, "unknown");
    assert_eq!(recent_denials[1].action, "blocked");
    assert_eq!(recent_denials[1].reason, "<empty>");
    denials::record("  ", "policy", "deny", "ignored because tool name is blank");
    assert_eq!(denials::list(1)[0].tool_name, "secret.tool");
    denials::record("long.tool", "policy", "deny", &"a".repeat(10_000));
    let long_denial = denials::list(1).into_iter().next().expect("long denial");
    assert_eq!(long_denial.tool_name, "long.tool");
    assert!(long_denial.reason.ends_with('…'));
    assert!(long_denial.reason.chars().count() <= 241);

    let diagnostics = diagnostics_for_config(&config).value;
    assert!(diagnostics.total_tools > 0);
    assert!(diagnostics.enabled_tools > 0);
    assert!(diagnostics
        .policy_surfaces
        .iter()
        .any(|surface| surface == "approval.decide"));
    assert!(diagnostics
        .recent_denials
        .iter()
        .any(|denial| denial.tool_name == "secret.tool"));
    assert_eq!(diagnostics.capability_providers.total_providers, 2);
}

#[tokio::test]
async fn generated_tools_raw_paths_cover_admission_validation_and_execution() {
    let schema = json!({
        "type": "object",
        "properties": {
            "message": { "type": "string" }
        },
        "required": ["message"]
    });
    let mut definition = GeneratedToolDefinition::new(
        " generated.echo ",
        " Execute through the generated adapter. ",
        schema.clone(),
        " echo-generated ",
    );
    definition.permission_level = PermissionLevel::Write;
    definition.category = ToolCategory::Skill;
    definition.scope = ToolScope::All;
    definition.provider_id = Some(" Trusted.Runtime ".into());
    definition.capability_id = Some(" messages.send ".into());
    definition.source_digest = Some(" sha256:generated ".into());
    definition.risk = Some(GeneratedToolRisk::ExternalWrite);
    definition.policy_surface = Some(" generated.surface ".into());

    let admission = GeneratedToolAdmissionConfig {
        enforce_provenance: true,
        trusted_providers: BTreeSet::from(["TRUSTED.RUNTIME".to_string(), "bad/provider".into()]),
        disabled_providers: BTreeSet::from(["ignored/provider".into()]),
        existing_tool_names: BTreeSet::from(["reserved.tool".to_string()]),
        ..Default::default()
    };
    let report = admit_generated_tool_definitions(vec![definition.clone()], &admission);
    assert_eq!(report.rejected, Vec::new());
    assert_eq!(report.admitted.len(), 1);
    let admitted = report.admitted[0].clone();
    assert_eq!(admitted.name, "generated.echo");
    assert_eq!(
        admitted.description,
        "Execute through the generated adapter."
    );
    assert_eq!(admitted.adapter_id, "echo-generated");
    assert_eq!(admitted.provider_id.as_deref(), Some("trusted.runtime"));
    assert_eq!(admitted.capability_id.as_deref(), Some("messages.send"));
    assert_eq!(admitted.source_digest.as_deref(), Some("sha256:generated"));
    assert_eq!(
        admitted.policy_surface.as_deref(),
        Some("generated.surface")
    );

    let adapter = Arc::new(EchoGeneratedAdapter);
    let tools = generated_tools_from_definitions(vec![admitted.clone()], adapter.clone())
        .expect("generated tools should instantiate");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name(), "generated.echo");
    assert_eq!(
        tools[0].description(),
        "Execute through the generated adapter."
    );
    assert_eq!(tools[0].permission_level(), PermissionLevel::Write);
    assert_eq!(tools[0].category(), ToolCategory::Skill);
    assert_eq!(tools[0].scope(), ToolScope::All);
    assert_eq!(tools[0].parameters_schema(), schema);
    assert!(tools[0].external_effect());
    let result = tools[0]
        .execute(json!({ "message": "hello" }))
        .await
        .expect("generated tool execution");
    assert!(result.output().contains("generated.echo"));
    assert!(result.output().contains("hello"));

    let mut duplicate = admitted.clone();
    duplicate.name = "reserved.tool".into();
    let duplicate_report = admit_generated_tool_definitions(vec![duplicate], &admission);
    assert!(duplicate_report.admitted.is_empty());
    assert!(duplicate_report.rejected[0].reason.contains("duplicate"));

    let mut unsafe_name = admitted.clone();
    unsafe_name.name = "Bad Tool".into();
    let unsafe_report = admit_generated_tool_definitions(vec![unsafe_name], &admission);
    assert!(unsafe_report.admitted.is_empty());
    assert!(unsafe_report.rejected[0]
        .reason
        .contains("unsupported characters"));

    let mut missing_provider = admitted.clone();
    missing_provider.provider_id = None;
    let missing_provider_report =
        admit_generated_tool_definitions(vec![missing_provider], &admission);
    assert!(missing_provider_report.admitted.is_empty());
    assert!(missing_provider_report.rejected[0]
        .reason
        .contains("missing provider_id"));

    let mut bad_schema = admitted.clone();
    bad_schema.parameters_schema = json!({ "properties": {} });
    let bad_schema_report = admit_generated_tool_definitions(vec![bad_schema], &admission);
    assert!(bad_schema_report.admitted.is_empty());
    assert!(bad_schema_report.rejected[0]
        .reason
        .contains("invalid schema"));

    let mut adapter_mismatch = admitted.clone();
    adapter_mismatch.adapter_id = "other-adapter".into();
    match generated_tools_from_definitions(vec![adapter_mismatch], adapter) {
        Ok(_) => panic!("adapter mismatch should fail"),
        Err(error) => assert!(error.to_string().contains("requires adapter")),
    }
}

#[tokio::test]
async fn filesystem_and_system_tool_edges_cover_deterministic_error_and_success_paths() {
    let dir = tempdir().expect("tempdir");

    let memory_tool = UpdateMemoryMdTool::new(dir.path().to_path_buf());
    assert_eq!(memory_tool.name(), "update_memory_md");
    assert_eq!(memory_tool.permission_level(), PermissionLevel::Write);
    assert_eq!(
        memory_tool
            .parameters_schema()
            .pointer("/properties/file/enum/0"),
        Some(&json!("MEMORY.md"))
    );

    let bad_file = memory_tool
        .execute(json!({
            "file": "NOTES.md",
            "action": "append",
            "content": "ignored"
        }))
        .await
        .expect("bad memory file returns tool error");
    assert!(bad_file.is_error);
    assert!(bad_file.output().contains("not allowed"));

    let appended = memory_tool
        .execute(json!({
            "file": "MEMORY.md",
            "action": "append",
            "content": "first note"
        }))
        .await
        .expect("append memory note");
    assert!(!appended.is_error, "{}", appended.output());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("MEMORY.md")).expect("read MEMORY.md"),
        "first note\n"
    );

    let replaced_new = memory_tool
        .execute(json!({
            "file": "MEMORY.md",
            "action": "replace_section",
            "section_title": "Facts",
            "content": "alpha"
        }))
        .await
        .expect("append missing section");
    assert!(!replaced_new.is_error, "{}", replaced_new.output());
    assert!(std::fs::read_to_string(dir.path().join("MEMORY.md"))
        .expect("read MEMORY.md after section append")
        .contains("## Facts\nalpha\n"));

    let replaced_existing = memory_tool
        .execute(json!({
            "file": "MEMORY.md",
            "action": "replace_section",
            "section_title": "Facts",
            "content": "beta"
        }))
        .await
        .expect("replace existing section");
    assert!(
        !replaced_existing.is_error,
        "{}",
        replaced_existing.output()
    );
    let memory_md = std::fs::read_to_string(dir.path().join("MEMORY.md"))
        .expect("read MEMORY.md after replace");
    assert!(memory_md.contains("## Facts\nbeta\n"));
    assert!(!memory_md.contains("alpha"));

    let missing_section_title = memory_tool
        .execute(json!({
            "file": "SKILL.md",
            "action": "replace_section",
            "content": "body"
        }))
        .await
        .expect_err("missing section_title is argument error");
    assert!(missing_section_title.to_string().contains("section_title"));

    let unknown_action = memory_tool
        .execute(json!({
            "file": "SKILL.md",
            "action": "rewrite",
            "content": "body"
        }))
        .await
        .expect("unknown action returns tool error");
    assert!(unknown_action.is_error);
    assert!(unknown_action.output().contains("Unknown action"));

    let linter = RunLinterTool::new(dir.path().to_path_buf());
    assert_eq!(linter.name(), "run_linter");
    assert_eq!(linter.permission_level(), PermissionLevel::Execute);
    assert_eq!(
        linter
            .parameters_schema()
            .pointer("/properties/linter/default"),
        Some(&json!("auto"))
    );
    let auto = linter
        .execute(json!({ "linter": "auto" }))
        .await
        .expect("auto linter without project files");
    assert!(auto.is_error);
    assert!(auto.output().contains("Could not detect project type"));
    let bad_eslint_path = linter
        .execute(json!({ "linter": "eslint", "path": "../escape.js" }))
        .await
        .expect("eslint rejects escaping path before spawn");
    assert!(bad_eslint_path.is_error);
    assert!(bad_eslint_path.output().contains("relative path"));
    let unknown_linter = linter
        .execute(json!({ "linter": "rubocop" }))
        .await
        .expect("unknown linter");
    assert!(unknown_linter.is_error);
    assert!(unknown_linter.output().contains("Unknown linter"));

    std::fs::write(dir.path().join("visible.txt"), "hello").expect("write visible file");
    std::fs::create_dir(dir.path().join("visible_dir")).expect("create visible dir");
    let workspace = WorkspaceStateTool::new(dir.path().to_path_buf());
    assert_eq!(workspace.name(), "read_workspace_state");
    assert_eq!(workspace.permission_level(), PermissionLevel::ReadOnly);
    let state = workspace
        .execute(json!({ "include_tree": true, "recent_commits": 2 }))
        .await
        .expect("workspace state");
    assert!(!state.is_error, "{}", state.output());
    assert!(state.output().contains("## Git Status"));
    assert!(state.output().contains("visible.txt"));
    assert!(state.output().contains("visible_dir/"));
    let no_tree = workspace
        .execute(json!({ "include_tree": false }))
        .await
        .expect("workspace state without tree");
    assert!(!no_tree.output().contains("Directory Tree"));

    let insert = InsertSqlRecordTool::new();
    assert_eq!(insert.name(), "insert_sql_record");
    assert_eq!(insert.permission_level(), PermissionLevel::Write);
    let missing_session = insert
        .execute(json!({ "role": "user", "content": "hello" }))
        .await
        .expect_err("missing session_id");
    assert!(missing_session.to_string().contains("session_id"));
    let invalid_role = insert
        .execute(json!({
            "session_id": "s1",
            "role": "system",
            "content": "hello"
        }))
        .await
        .expect("invalid role returns tool error");
    assert!(invalid_role.is_error);
    assert!(invalid_role.output().contains("Invalid role"));
    let blank_content = insert
        .execute(json!({
            "session_id": "s1",
            "role": "tool",
            "content": "   "
        }))
        .await
        .expect("blank content returns tool error");
    assert!(blank_content.is_error);
    assert!(blank_content.output().contains("content"));
    let staged = insert
        .execute(json!({
            "session_id": "s1",
            "role": "assistant",
            "content": "remember this",
            "lesson": "short lesson"
        }))
        .await
        .expect("valid insert is currently pending implementation");
    assert!(staged.is_error);
    assert!(staged.output().contains("FTS5/SQLite insert pending"));
}

#[tokio::test]
async fn proxy_config_tool_covers_temp_config_runtime_env_and_validation_paths() {
    let _lock = env_lock();
    let dir = tempdir().expect("tempdir");
    let _http_guard = EnvVarGuard::unset("HTTP_PROXY");
    let _https_guard = EnvVarGuard::unset("HTTPS_PROXY");
    let _all_guard = EnvVarGuard::unset("ALL_PROXY");
    let _no_guard = EnvVarGuard::unset("NO_PROXY");

    let mut config = Config {
        workspace_dir: dir.path().join("workspace"),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };
    config.autonomy.level = openhuman_core::openhuman::security::AutonomyLevel::Full;
    config.save().await.expect("write temp config");

    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));
    let tool = ProxyConfigTool::new(Arc::new(config.clone()), security);
    assert_eq!(tool.name(), "proxy_config");
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    assert_eq!(
        tool.parameters_schema()
            .pointer("/properties/action/enum/0"),
        Some(&json!("get"))
    );

    let initial = tool.execute(json!({ "action": "get" })).await.expect("get");
    assert!(!initial.is_error, "{}", initial.output());
    assert!(initial.output().contains("\"proxy\""));

    let services = tool
        .execute(json!({ "action": "list_services" }))
        .await
        .expect("list services");
    assert!(!services.is_error, "{}", services.output());
    assert!(services.output().contains("provider.openai"));
    assert!(services.output().contains("tool.http_request"));

    let invalid_scope = tool
        .execute(json!({ "action": "set", "scope": "elsewhere" }))
        .await
        .expect("invalid scope is tool error");
    assert!(invalid_scope.is_error);
    assert!(invalid_scope.output().contains("Invalid scope"));

    let bad_no_proxy = tool
        .execute(json!({ "action": "set", "no_proxy": [123] }))
        .await
        .expect("bad no_proxy is tool error");
    assert!(bad_no_proxy.is_error);
    assert!(bad_no_proxy
        .output()
        .contains("array must only contain strings"));

    let set_services = tool
        .execute(json!({
            "action": "set",
            "enabled": true,
            "scope": "services",
            "http_proxy": "http://127.0.0.1:8888",
            "https_proxy": null,
            "no_proxy": "localhost, 127.0.0.1",
            "services": [" provider.openai ", "tool.http_request", ""]
        }))
        .await
        .expect("set services proxy");
    assert!(!set_services.is_error, "{}", set_services.output());
    assert!(set_services
        .output()
        .contains("Proxy configuration updated"));
    assert!(set_services.output().contains("provider.openai"));

    let apply_wrong_scope = tool
        .execute(json!({ "action": "apply_env" }))
        .await
        .expect("apply_env wrong scope is tool error");
    assert!(apply_wrong_scope.is_error);
    assert!(apply_wrong_scope.output().contains("environment"));

    let set_environment = tool
        .execute(json!({
            "action": "set",
            "enabled": true,
            "scope": "environment",
            "http_proxy": "http://127.0.0.1:8888",
            "https_proxy": "http://127.0.0.1:8889",
            "all_proxy": "",
            "no_proxy": ["localhost", "127.0.0.1"]
        }))
        .await
        .expect("set environment proxy");
    assert!(!set_environment.is_error, "{}", set_environment.output());
    assert_eq!(
        std::env::var("HTTP_PROXY").as_deref(),
        Ok("http://127.0.0.1:8888")
    );
    assert_eq!(
        std::env::var("HTTPS_PROXY").as_deref(),
        Ok("http://127.0.0.1:8889")
    );

    let clear_env = tool
        .execute(json!({ "action": "clear_env" }))
        .await
        .expect("clear env");
    assert!(!clear_env.is_error, "{}", clear_env.output());
    assert!(std::env::var("HTTP_PROXY").is_err());

    let disable = tool
        .execute(json!({ "action": "disable", "clear_env": true }))
        .await
        .expect("disable proxy");
    assert!(!disable.is_error, "{}", disable.output());
    assert!(disable.output().contains("Proxy disabled"));

    let unknown = tool
        .execute(json!({ "action": "unknown" }))
        .await
        .expect_err("unknown action is an argument error");
    assert!(unknown.to_string().contains("Unknown action"));
}

#[tokio::test]
async fn filesystem_search_and_system_probe_tools_cover_success_and_error_paths() {
    let _lock = env_lock();
    let dir = tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    std::fs::create_dir_all(dir.path().join("node_modules")).expect("create skipped dir");
    std::fs::write(
        dir.path().join("src").join("main.rs"),
        "Alpha\nbeta\nalpha tail\n",
    )
    .expect("write grep fixture");
    std::fs::write(
        dir.path().join("node_modules").join("hidden.txt"),
        "alpha hidden\n",
    )
    .expect("write skipped fixture");

    let security = Arc::new(SecurityPolicy::from_config(
        &Config::default().autonomy,
        dir.path(),
    ));

    let file_write = FileWriteTool::new(security.clone());
    assert_eq!(file_write.name(), "file_write");
    assert_eq!(file_write.permission_level(), PermissionLevel::Write);
    assert!(!file_write.external_effect_with_args(&json!({ "path": "new.txt" })));
    let wrote = file_write
        .execute(json!({ "path": "notes/new.txt", "content": "one\ntwo\none\n" }))
        .await
        .expect("file write executes");
    assert!(!wrote.is_error, "{}", wrote.output());
    assert!(file_write.external_effect_with_args(&json!({ "path": "notes/new.txt" })));

    let file_read = FileReadTool::new(security.clone());
    assert_eq!(file_read.name(), "file_read");
    assert!(file_read.is_concurrency_safe(&json!({})));
    let read = file_read
        .execute(json!({ "path": "notes/new.txt" }))
        .await
        .expect("file read executes");
    assert_eq!(read.output(), "one\ntwo\none\n");
    let missing_path = file_read
        .execute(json!({ "path": "missing.txt" }))
        .await
        .expect("missing read is tool error");
    assert!(missing_path.is_error);
    assert!(missing_path.output().contains("Failed to resolve"));

    let list = ListFilesTool::new(security.clone());
    assert_eq!(list.name(), "list");
    let listing = list.execute(json!({ "path": "." })).await.expect("list");
    assert!(!listing.is_error, "{}", listing.output());
    assert!(listing.output().contains("dir\tnotes"));

    let glob = GlobTool::new(security.clone());
    assert_eq!(glob.name(), "glob");
    assert!(glob.is_concurrency_safe(&json!({})));
    let globbed = glob
        .execute(json!({ "pattern": "notes/*.txt", "max_results": 1 }))
        .await
        .expect("glob");
    assert!(!globbed.is_error, "{}", globbed.output());
    assert!(globbed.output().contains("notes/new.txt"));
    let bad_glob = glob
        .execute(json!({ "pattern": "[" }))
        .await
        .expect("bad glob is tool error");
    assert!(bad_glob.is_error);
    assert!(bad_glob.output().contains("Invalid glob pattern"));

    let edit = EditFileTool::new(security.clone());
    assert_eq!(edit.name(), "edit");
    assert!(edit.external_effect_with_args(&json!({})));
    let duplicate_edit = edit
        .execute(json!({
            "path": "notes/new.txt",
            "old_string": "one",
            "new_string": "three"
        }))
        .await
        .expect("duplicate edit is tool error");
    assert!(duplicate_edit.is_error);
    assert!(duplicate_edit.output().contains("matches 2 times"));
    let edited = edit
        .execute(json!({
            "path": "notes/new.txt",
            "old_string": "one",
            "new_string": "three",
            "replace_all": true
        }))
        .await
        .expect("edit replace all");
    assert!(!edited.is_error, "{}", edited.output());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("notes/new.txt")).expect("read edited"),
        "three\ntwo\nthree\n"
    );

    let patch = ApplyPatchTool::new(security.clone());
    assert_eq!(patch.name(), "apply_patch");
    let empty_patch = patch
        .execute(json!({ "edits": [] }))
        .await
        .expect("empty patch is tool error");
    assert!(empty_patch.is_error);
    assert!(empty_patch.output().contains("empty"));
    let patched = patch
        .execute(json!({
            "edits": [{
                "path": "notes/new.txt",
                "old_string": "two",
                "new_string": "four"
            }]
        }))
        .await
        .expect("apply patch");
    assert!(!patched.is_error, "{}", patched.output());
    assert!(std::fs::read_to_string(dir.path().join("notes/new.txt"))
        .expect("read patched")
        .contains("four"));

    let csv = CsvExportTool::new(security.clone());
    assert_eq!(csv.name(), "csv_export");
    let csv_result = csv
        .execute(json!({
            "data": r#"[{"name":"Ada, Lovelace","active":true,"score":7},{"name":"Grace","active":false,"score":9}]"#,
            "filename": "coverage.csv",
            "columns": ["name", "active", "score"]
        }))
        .await
        .expect("csv export");
    assert!(!csv_result.is_error, "{}", csv_result.output());
    let csv_body =
        std::fs::read_to_string(dir.path().join("exports/coverage.csv")).expect("read csv export");
    assert!(csv_body.contains("\"Ada, Lovelace\",true,7"));
    let csv_bad_json = csv
        .execute(json!({ "data": "not-json", "filename": "bad.csv" }))
        .await
        .expect("bad csv json is tool error");
    assert!(csv_bad_json.is_error);
    assert!(csv_bad_json.output().contains("Failed to parse data"));

    let grep = GrepTool::new(security);
    assert_eq!(grep.name(), "grep");
    assert_eq!(grep.permission_level(), PermissionLevel::ReadOnly);
    assert!(grep.is_concurrency_safe(&json!({})));
    let matches = grep
        .execute(json!({
            "pattern": "alpha",
            "case_insensitive": true,
            "path": "src",
            "max_matches": 1
        }))
        .await
        .expect("grep executes");
    assert!(!matches.is_error, "{}", matches.output());
    assert!(matches.output().contains("truncated at 1"));
    assert!(matches.output().contains("src/main.rs:1:Alpha"));
    assert!(!matches.output().contains("hidden"));
    let invalid_regex = grep
        .execute(json!({ "pattern": "([unterminated" }))
        .await
        .expect("invalid regex returns tool error");
    assert!(invalid_regex.is_error);
    assert!(invalid_regex.output().contains("Invalid regex"));

    let detect = DetectToolsTool::new();
    assert_eq!(detect.name(), "detect_tools");
    let detected = detect
        .execute(json!({ "tools": ["definitely_not_a_real_binary_xyz_123"] }))
        .await
        .expect("detect tools executes");
    assert!(!detected.is_error);
    let detected_json: Value = serde_json::from_str(&detected.output()).expect("detect json");
    assert_eq!(detected_json.get("probed").and_then(Value::as_u64), Some(1));
    assert_eq!(
        detected_json.pointer("/missing/0").and_then(Value::as_str),
        Some("definitely_not_a_real_binary_xyz_123")
    );

    let lsp = LspTool::new();
    assert_eq!(lsp.name(), "lsp");
    assert_eq!(lsp.permission_level(), PermissionLevel::ReadOnly);
    let lsp_result = lsp
        .execute(json!({ "kind": "hover", "language": "rust", "file": "src/main.rs" }))
        .await
        .expect("lsp returns stub result");
    assert!(lsp_result.is_error);
    assert!(lsp_result.output().contains("not yet implemented"));

    let run_tests = RunTestsTool::new(dir.path().to_path_buf());
    assert_eq!(run_tests.name(), "run_tests");
    let no_project = run_tests
        .execute(json!({ "runner": "auto" }))
        .await
        .expect("run_tests auto without project");
    assert!(no_project.is_error);
    assert!(no_project
        .output()
        .contains("Could not detect project type"));
    let bad_runner = run_tests
        .execute(json!({ "runner": "gradle" }))
        .await
        .expect("run_tests bad runner");
    assert!(bad_runner.is_error);
    assert!(bad_runner.output().contains("Unknown test runner"));

    let update_apply = UpdateApplyTool::new(Arc::new(SecurityPolicy::from_config(
        &Config::default().autonomy,
        dir.path(),
    )));
    assert_eq!(update_apply.name(), "update_apply");
    assert_eq!(update_apply.permission_level(), PermissionLevel::Dangerous);
    let missing_consent = update_apply
        .execute(json!({}))
        .await
        .expect("update apply consent guard");
    assert!(missing_consent.is_error);
    assert!(missing_consent.output().contains("explicit user consent"));

    let current_time = CurrentTimeTool::new();
    assert!(current_time.supports_markdown());
    let time_result = current_time
        .execute_with_options(
            json!({ "timezone": "Not/AReal_Zone" }),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("current time executes");
    assert!(!time_result.is_error, "{}", time_result.output());
    assert!(time_result.output().contains("requested_timezone_error"));
    assert!(time_result
        .markdown_formatted
        .as_deref()
        .is_some_and(|md| md.contains("timezone error")));
}

#[tokio::test]
async fn node_and_npm_exec_tools_cover_validation_policy_and_disabled_runtime_paths() {
    let dir = tempdir().expect("tempdir");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");

    let mut config = Config {
        workspace_dir: workspace.clone(),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };
    config.autonomy.level = AutonomyLevel::Full;
    config.node = NodeConfig {
        enabled: false,
        ..NodeConfig::default()
    };

    let full_security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));
    let readonly_security = Arc::new(SecurityPolicy::from_config(
        &openhuman_core::openhuman::config::AutonomyConfig {
            level: AutonomyLevel::ReadOnly,
            ..config.autonomy.clone()
        },
        &config.workspace_dir,
    ));
    let runtime = Arc::new(NativeRuntime::new());
    let bootstrap = Arc::new(NodeBootstrap::new(
        config.node.clone(),
        workspace,
        reqwest::Client::new(),
    ));

    let node = NodeExecTool::new(full_security.clone(), runtime.clone(), bootstrap.clone());
    assert_eq!(node.name(), "node_exec");
    assert_eq!(node.permission_level(), PermissionLevel::Execute);
    assert!(node.description().contains("Execute JavaScript"));
    assert_eq!(
        node.parameters_schema()
            .pointer("/properties/inline_code/type"),
        Some(&json!("string"))
    );

    let missing_mode = node.execute(json!({})).await.expect("node mode validation");
    assert!(missing_mode.is_error);
    assert!(missing_mode.output().contains("exactly one"));
    let conflicting_mode = node
        .execute(json!({
            "inline_code": "console.log('hi')",
            "script_path": "script.js",
        }))
        .await
        .expect("node conflict validation");
    assert!(conflicting_mode.is_error);
    assert!(conflicting_mode.output().contains("exactly one"));

    let readonly_node = NodeExecTool::new(
        readonly_security.clone(),
        runtime.clone(),
        bootstrap.clone(),
    );
    let blocked = readonly_node
        .execute(json!({ "inline_code": "console.log('blocked')" }))
        .await
        .expect("node read-only block");
    assert!(blocked.is_error);
    assert!(blocked.output().contains("read-only mode"));

    let disabled_runtime = node
        .execute(json!({ "inline_code": "console.log('disabled')" }))
        .await
        .expect("node disabled runtime");
    assert!(disabled_runtime.is_error);
    assert!(disabled_runtime
        .output()
        .contains("Node.js runtime unavailable"));

    let npm = NpmExecTool::new(full_security, runtime.clone(), bootstrap.clone());
    assert_eq!(npm.name(), "npm_exec");
    assert_eq!(npm.permission_level(), PermissionLevel::Execute);
    assert!(npm.description().contains("npm subcommand"));
    assert_eq!(
        npm.parameters_schema().pointer("/required/0"),
        Some(&json!("subcommand"))
    );

    let missing_subcommand = npm.execute(json!({})).await.expect("npm missing");
    assert!(missing_subcommand.is_error);
    assert!(missing_subcommand
        .output()
        .contains("requires a `subcommand`"));
    let empty_subcommand = npm
        .execute(json!({ "subcommand": "   " }))
        .await
        .expect("npm empty");
    assert!(empty_subcommand.is_error);
    assert!(empty_subcommand.output().contains("cannot be empty"));
    let rejected_subcommand = npm
        .execute(json!({ "subcommand": "run && echo nope" }))
        .await
        .expect("npm metachar rejection");
    assert!(rejected_subcommand.is_error);
    assert!(rejected_subcommand.output().contains("rejected subcommand"));
    let disallowed_subcommand = npm
        .execute(json!({ "subcommand": "publish" }))
        .await
        .expect("npm disallowed mutation");
    assert!(disallowed_subcommand.is_error);
    assert!(disallowed_subcommand.output().contains("refuses to run"));

    let readonly_npm = NpmExecTool::new(readonly_security, runtime, bootstrap);
    let blocked_npm = readonly_npm
        .execute(json!({ "subcommand": "test" }))
        .await
        .expect("npm read-only block");
    assert!(blocked_npm.is_error);
    assert!(blocked_npm.output().contains("read-only mode"));

    let disabled_npm = npm
        .execute(json!({ "subcommand": "test", "timeout_secs": 1 }))
        .await
        .expect("npm disabled runtime");
    assert!(disabled_npm.is_error);
    assert!(disabled_npm
        .output()
        .contains("Node.js runtime unavailable"));
}

#[tokio::test]
async fn doctor_channels_covers_no_channel_and_local_validation_paths() {
    let mut empty = Config::default();
    empty.channels_config = openhuman_core::openhuman::config::ChannelsConfig::default();
    doctor_channels(empty)
        .await
        .expect("empty channel doctor is ok");

    let mut config = Config::default();
    config.channels_config = openhuman_core::openhuman::config::ChannelsConfig::default();
    config.channels_config.imessage = Some(IMessageConfig {
        allowed_contacts: Vec::new(),
    });
    config.channels_config.whatsapp = Some(WhatsAppConfig {
        access_token: None,
        phone_number_id: Some("phone-number-id".into()),
        verify_token: None,
        app_secret: None,
        session_path: None,
        pair_phone: None,
        pair_code: None,
        allowed_numbers: Vec::new(),
    });
    config.channels_config.webhook = Some(WebhookConfig {
        port: 0,
        secret: Some("secret".into()),
    });

    doctor_channels(config)
        .await
        .expect("doctor handles local iMessage check and invalid WhatsApp config");
}

#[tokio::test]
async fn irc_channel_public_constructor_and_preconnect_send_are_deterministic() {
    let irc = IrcChannel::new(IrcChannelConfig {
        server: "irc.example.test".into(),
        port: 6697,
        nickname: "openhuman".into(),
        username: None,
        channels: vec!["#coverage".into()],
        allowed_users: vec!["Alice".into()],
        server_password: None,
        nickserv_password: Some("nickserv-secret".into()),
        sasl_password: Some("sasl-secret".into()),
        verify_tls: false,
    });

    assert_eq!(irc.name(), "irc");
    assert_eq!(
        irc.send(&SendMessage::new("hello", "#coverage"))
            .await
            .expect_err("send before listen should not hit network")
            .to_string(),
        "IRC not connected"
    );
}

#[tokio::test]
async fn web_fetch_and_gitbooks_tools_use_local_http_backends() {
    let dir = tempdir().expect("tempdir");
    let (addr, join) = serve_backend().await;
    let base = format!("http://{addr}");
    let security = Arc::new(SecurityPolicy::from_config(
        &Config::default().autonomy,
        dir.path(),
    ));

    let fetch = WebFetchTool::new(security, vec!["*".into()], Some(0), Some(5));
    assert_eq!(fetch.name(), "web_fetch");
    assert_eq!(fetch.permission_level(), PermissionLevel::ReadOnly);
    assert!(fetch.is_concurrency_safe(&json!({})));
    assert_eq!(fetch.max_result_size_chars(), Some(50_000));
    let loopback_block = fetch
        .execute(json!({ "url": format!("{base}/plain"), "max_bytes": 8 }))
        .await
        .expect("web fetch blocks loopback before network");
    assert!(loopback_block.is_error);
    assert!(loopback_block
        .output()
        .contains("Blocked local/private host"));
    let bad_scheme = fetch
        .execute(json!({ "url": "file:///tmp/secret" }))
        .await
        .expect("web fetch bad scheme");
    assert!(bad_scheme.is_error);
    assert!(bad_scheme.output().contains("URL rejected"));

    let endpoint = format!("{base}/mcp");
    let search = GitbooksSearchTool::new(endpoint.clone(), 5);
    assert_eq!(search.name(), "gitbooks_search");
    assert_eq!(search.permission_level(), PermissionLevel::ReadOnly);
    let blank_query = search
        .execute(json!({ "query": "  " }))
        .await
        .expect("blank query");
    assert!(blank_query.is_error);
    assert!(blank_query.output().contains("empty"));
    let searched = search
        .execute(json!({ "query": "channels coverage" }))
        .await
        .expect("gitbooks search");
    assert!(!searched.is_error, "{}", searched.output());
    assert!(searched
        .output()
        .contains("gitbooks mocked searchDocumentation"));

    let get_page = GitbooksGetPageTool::new(endpoint, 5);
    assert_eq!(get_page.name(), "gitbooks_get_page");
    let blank_url = get_page
        .execute(json!({ "url": "" }))
        .await
        .expect("blank page url");
    assert!(blank_url.is_error);
    assert!(blank_url.output().contains("empty"));
    let page = get_page
        .execute(json!({ "url": "https://tinyhumans.gitbook.io/openhuman/test" }))
        .await
        .expect("gitbooks get page");
    assert!(!page.is_error, "{}", page.output());
    assert!(page.output().contains("gitbooks mocked getPage"));

    join.abort();
}

#[test]
fn yuanbao_config_wire_and_splitter_helpers_cover_public_deterministic_paths() {
    assert!(NO_RECONNECT_CLOSE_CODES.contains(&4012));
    assert!(AUTH_FAILED_CODES.contains(&40001));
    assert!(AUTH_RETRYABLE_CODES.contains(&40010));

    let mut cfg = YuanbaoConfig::default();
    assert_eq!(cfg.env, "prod");
    assert_eq!(cfg.bot_version, "0.1.0");
    assert_eq!(cfg.dm_access, "open");
    assert_eq!(cfg.group_access, "allowlist");
    assert!(cfg.group_at_required);
    assert_eq!(cfg.max_message_length, 4500);
    assert_eq!(cfg.max_media_mb, 50);
    assert!(cfg
        .validate()
        .expect_err("default config invalid")
        .to_string()
        .contains("app_key"));
    cfg.env = "pre".into();
    cfg.apply_env_defaults();
    assert_eq!(cfg.api_domain, "https://bot-pre.yuanbao.tencent.com");
    assert_eq!(
        cfg.ws_domain,
        "wss://bot-wss-pre.yuanbao.tencent.com/wss/connection"
    );
    assert!(cfg
        .validate()
        .expect_err("missing token or secret invalid")
        .to_string()
        .contains("app_key"));
    cfg.app_key = "app-key".into();
    assert!(cfg
        .validate()
        .expect_err("missing token or secret invalid")
        .to_string()
        .contains("token"));
    cfg.token = "pre-provisioned-token".into();
    assert!(cfg.validate().is_ok());

    let mut explicit = YuanbaoConfig {
        app_key: "app-key".into(),
        token: "token".into(),
        api_domain: "https://custom-api.example.test".into(),
        ws_domain: "wss://custom-ws.example.test".into(),
        ..YuanbaoConfig::default()
    };
    explicit.apply_env_defaults();
    assert_eq!(explicit.api_domain, "https://custom-api.example.test");
    assert_eq!(explicit.ws_domain, "wss://custom-ws.example.test");
    assert!(explicit.validate().is_ok());

    let seq_a = next_seq_no();
    let seq_b = next_seq_no();
    assert_eq!(seq_b, seq_a + 1);

    let mut varint = Vec::new();
    encode_varint(300, &mut varint);
    assert_eq!(varint, vec![0xac, 0x02]);
    assert_eq!(decode_varint(&varint, 0).expect("decode varint"), (300, 2));
    assert!(decode_varint(&[0x80], 0)
        .expect_err("truncated varint")
        .to_string()
        .contains("truncated varint"));
    assert!(decode_varint(&[0xff; 10], 0)
        .expect_err("overflow varint")
        .to_string()
        .contains("overflow"));

    let mut fields_buf = Vec::new();
    encode_field_varint(1, 42, &mut fields_buf);
    encode_field_string(2, "hello", &mut fields_buf);
    encode_field_bytes(2, b"again", &mut fields_buf);
    fields_buf.push((3 << 3) | 5);
    fields_buf.extend_from_slice(&0x1234_5678_u32.to_le_bytes());
    fields_buf.push((4 << 3) | 1);
    fields_buf.extend_from_slice(&0x0102_0304_0506_0708_u64.to_le_bytes());

    let fields = parse_fields(&fields_buf).expect("parse mixed fields");
    assert_eq!(get_varint(&fields, 1), 42);
    assert_eq!(get_string(&fields, 2), "hello");
    assert_eq!(get_bytes(&fields, 2), b"hello".to_vec());
    assert_eq!(
        get_repeated_bytes(&fields, 2),
        vec![b"hello".to_vec(), b"again".to_vec()]
    );
    assert_eq!(get_varint(&fields, 99), 0);
    assert_eq!(get_string(&fields, 99), "");
    assert!(get_bytes(&fields, 99).is_empty());
    assert!(fields
        .iter()
        .any(|(_, value)| matches!(value, FieldValue::Fixed32(0x1234_5678))));
    assert!(fields
        .iter()
        .any(|(_, value)| matches!(value, FieldValue::Fixed64(0x0102_0304_0506_0708))));
    assert!(parse_fields(&[((9 << 3) | 3) as u8])
        .expect_err("unsupported wire type")
        .to_string()
        .contains("unsupported wire type"));
    assert!(parse_fields(&[((9 << 3) | 2) as u8, 5, b'a'])
        .expect_err("truncated len field")
        .to_string()
        .contains("truncated len field"));

    assert_eq!(split_markdown("short", 100), vec!["short"]);
    let fenced = "intro\n```rust\nfn alpha() {}\nfn beta() {}\n```\noutro\n";
    let chunks = split_markdown(fenced, 32);
    assert!(chunks.len() > 1);
    assert!(chunks.iter().any(|chunk| chunk.contains("```rust")));
    assert!(chunks.iter().all(|chunk| !chunk.trim().is_empty()));
    let hard_split = split_markdown("é".repeat(8).as_str(), 3);
    assert!(hard_split.len() > 1);
    assert!(hard_split.iter().all(|chunk| chunk.len() <= 4));
}

#[test]
fn yuanbao_media_and_proto_helpers_cover_public_roundtrips() {
    assert_eq!(guess_mime_type("PHOTO.JPG"), "image/jpeg");
    assert_eq!(
        guess_mime_type("slides.pptx"),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation"
    );
    assert_eq!(
        guess_mime_type("archive.unknown"),
        "application/octet-stream"
    );
    assert!(is_image("avatar.webp", ""));
    assert!(is_image("no-extension", "image/png"));
    assert!(!is_image("notes.txt", ""));
    assert_eq!(image_format_code("image/jpeg"), 1);
    assert_eq!(image_format_code("image/gif"), 2);
    assert_eq!(image_format_code("image/png"), 3);
    assert_eq!(image_format_code("image/bmp"), 4);
    assert_eq!(image_format_code("image/heic"), 255);

    let png = [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x01, 0x40, 0x00, 0x00, 0x00, 0xF0,
    ];
    let png_dims = parse_image_size(&png).expect("png dims");
    assert_eq!(png_dims.width, 320);
    assert_eq!(png_dims.height, 240);
    let gif_dims = parse_image_size(b"GIF89a\x40\x01\xF0\x00rest").expect("gif dims");
    assert_eq!(gif_dims.width, 320);
    assert_eq!(gif_dims.height, 240);
    let mut webp_vp8x = b"RIFF\x00\x00\x00\x00WEBPVP8X".to_vec();
    webp_vp8x.extend_from_slice(&[0u8; 8]);
    webp_vp8x.extend_from_slice(&[0x3F, 0x01, 0x00, 0xEF, 0x00, 0x00]);
    let webp_dims = parse_image_size(&webp_vp8x).expect("webp dims");
    assert_eq!(webp_dims.width, 320);
    assert_eq!(webp_dims.height, 240);
    assert!(parse_image_size(b"not-an-image").is_none());

    let image_body = build_image_msg_body(
        "https://cdn.example.test/cat.png",
        None,
        Some("cat.png"),
        1024,
        800,
        600,
        "image/png",
    );
    assert_eq!(image_body[0].msg_type, "TIMImageElem");
    assert_eq!(image_body[0].msg_content.uuid.as_deref(), Some("cat.png"));
    assert_eq!(image_body[0].msg_content.image_format, Some(3));
    assert_eq!(
        image_body[0].msg_content.image_info_array[0].url,
        "https://cdn.example.test/cat.png"
    );
    let file_body = build_file_msg_body(
        "https://cdn.example.test/report.pdf",
        "report.pdf",
        Some("file-uuid"),
        2048,
    );
    assert_eq!(file_body[0].msg_type, "TIMFileElem");
    assert_eq!(
        file_body[0].msg_content.file_name.as_deref(),
        Some("report.pdf")
    );
    assert_eq!(file_body[0].msg_content.file_size, Some(2048));

    let frame_buf = encode_conn_msg(
        cmd_type::REQUEST,
        cmd::PING,
        7,
        "msg-7",
        module::CONN_ACCESS,
        b"payload",
    );
    let frame = decode_conn_msg(&frame_buf).expect("decode conn msg");
    assert_eq!(frame.cmd_type, cmd_type::REQUEST);
    assert_eq!(frame.cmd, cmd::PING);
    assert_eq!(frame.seq_no, 7);
    assert_eq!(frame.msg_id, "msg-7");
    assert_eq!(frame.data, b"payload");
    let ping = decode_conn_msg(&encode_ping("ping-1")).expect("decode ping");
    assert_eq!(ping.cmd, cmd::PING);
    let ack = decode_conn_msg(&encode_push_ack(&YuanbaoConnFrame {
        cmd_type: cmd_type::PUSH,
        cmd: "push".into(),
        seq_no: 9,
        msg_id: "push-1".into(),
        need_ack: true,
        status: 0,
        module: module::BIZ_PKG.into(),
        data: Vec::new(),
    }))
    .expect("decode ack");
    assert_eq!(ack.cmd_type, cmd_type::PUSH_ACK);
    assert_eq!(ack.msg_id, "push-1");
    let auth = decode_conn_msg(&encode_auth_bind(
        "biz", "uid", "openclaw", "token", "auth-1", "1.0.0", "linux", "2.0.0", "pre",
    ))
    .expect("decode auth bind");
    assert_eq!(auth.cmd, cmd::AUTH_BIND);
    assert_eq!(auth.module, module::CONN_ACCESS);
    assert!(!auth.data.is_empty());

    let mut auth_rsp = Vec::new();
    encode_field_varint(1, 0, &mut auth_rsp);
    encode_field_string(2, "ok", &mut auth_rsp);
    encode_field_string(3, "connect-1", &mut auth_rsp);
    let auth_rsp = decode_auth_bind_rsp(&auth_rsp).expect("decode auth rsp");
    assert_eq!(auth_rsp.message, "ok");
    assert_eq!(auth_rsp.connect_id, "connect-1");

    let mut push_msg = Vec::new();
    encode_field_string(1, "inbound_message", &mut push_msg);
    encode_field_string(2, module::BIZ_PKG, &mut push_msg);
    encode_field_string(3, "push-msg-1", &mut push_msg);
    encode_field_bytes(4, b"biz-payload", &mut push_msg);
    let decoded_push = decode_push_msg(&push_msg).expect("decode push msg");
    assert_eq!(decoded_push.cmd, "inbound_message");
    assert_eq!(decoded_push.data, b"biz-payload");

    let text_el = YuanbaoMsgBodyElement {
        msg_type: "TIMTextElem".into(),
        msg_content: YuanbaoMsgContent {
            text: Some("hello from proto".into()),
            ..Default::default()
        },
    };
    let mut inbound = Vec::new();
    encode_field_string(1, "C2C.Callback", &mut inbound);
    encode_field_string(2, "sender", &mut inbound);
    encode_field_string(3, "bot", &mut inbound);
    encode_field_string(4, "Alice", &mut inbound);
    encode_field_varint(8, 11, &mut inbound);
    encode_field_varint(10, 1_780_000_000, &mut inbound);
    encode_field_string(12, "msg-11", &mut inbound);
    encode_field_bytes(13, &encode_msg_body_element(&text_el), &mut inbound);
    let mut recall = Vec::new();
    encode_field_varint(1, 10, &mut recall);
    encode_field_string(2, "old-msg", &mut recall);
    encode_field_bytes(17, &recall, &mut inbound);
    let mut log_ext = Vec::new();
    encode_field_string(1, "trace-11", &mut log_ext);
    encode_field_bytes(20, &log_ext, &mut inbound);
    let decoded_inbound = decode_inbound_push(&inbound).expect("decode inbound push");
    assert_eq!(decoded_inbound.callback_command, "C2C.Callback");
    assert_eq!(decoded_inbound.extract_text(), "hello from proto");
    assert_eq!(decoded_inbound.recall_msg_seq_list[0].msg_id, "old-msg");
    assert_eq!(decoded_inbound.trace_id, "trace-11");

    let decoded_json = decode_inbound_json(
        br#"{
            "callback_command": "Group.Callback",
            "from_account": "sender-json",
            "group_code": "group-json",
            "msg_seq": 12,
            "msg_body": [{
                "msg_type": "TIMImageElem",
                "msg_content": {
                    "uuid": "img-json",
                    "image_format": 3,
                    "image_info_array": [{
                        "image_type": 1,
                        "size": 50,
                        "width": 10,
                        "height": 20,
                        "url": "https://cdn.example.test/json.png"
                    }]
                }
            }],
            "recall_msg_seq_list": [{ "msg_seq": 11, "msg_id": "old-json" }],
            "log_ext": { "trace_id": "trace-json" }
        }"#,
    )
    .expect("decode inbound json");
    assert!(decoded_json.is_group());
    assert_eq!(
        decoded_json.extract_image_urls(),
        vec!["https://cdn.example.test/json.png".to_string()]
    );
    assert_eq!(decoded_json.recall_msg_seq_list[0].msg_seq, 11);
    assert_eq!(decoded_json.trace_id, "trace-json");
    assert!(decode_inbound_json(b"[]")
        .expect_err("json root must be object")
        .to_string()
        .contains("json root is not an object"));
}

#[tokio::test]
async fn yuanbao_sign_manager_uses_local_sign_token_backend_and_cache() {
    let (addr, join) = serve_backend().await;
    let api_domain = format!("http://{addr}");

    let signature = compute_signature("nonce", "2026-05-29T10:00:00+08:00", "app-key", "secret");
    assert_eq!(signature.len(), 64);
    assert!(signature.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert_eq!(
        signature,
        compute_signature("nonce", "2026-05-29T10:00:00+08:00", "app-key", "secret")
    );
    let nonce = generate_nonce();
    assert_eq!(nonce.len(), 32);
    assert!(nonce.chars().all(|ch| ch.is_ascii_hexdigit()));
    let timestamp = build_timestamp();
    assert!(timestamp.ends_with("+08:00"));

    let manager = SignManager::new(reqwest::Client::new());
    let entry = manager
        .get_token("app-key", "secret", &api_domain, "pre")
        .await
        .expect("sign manager fetches token");
    assert_eq!(entry.token, "yuanbao-token-e2e");
    assert_eq!(entry.bot_id, "yuanbao-bot-e2e");
    assert_eq!(entry.product, "openhuman");
    assert_eq!(entry.source, "coverage");
    assert!(entry.is_valid());
    assert!(entry.seconds_remaining() > 0);

    let cached = manager
        .cached("app-key")
        .await
        .expect("cached token remains valid");
    assert_eq!(cached.token, entry.token);
    let refreshed = manager
        .force_refresh("app-key", "secret", &api_domain, "")
        .await
        .expect("force refresh fetches token");
    assert_eq!(refreshed.bot_id, "yuanbao-bot-e2e");
    manager.clear_locks().await;
    join.abort();
}
