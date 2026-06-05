//! Focused raw-line coverage for owned domains with local-only dependencies.
//!
//! This suite drives public APIs for the agent, inference, and composio slices
//! against temp directories and loopback Axum servers. It avoids live providers.

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use tempfile::tempdir;

use openhuman_core::openhuman::agent::task_board::{
    board_for_thread, TaskApprovalMode, TaskBoard, TaskBoardCard, TaskBoardStore, TaskCardStatus,
};
use openhuman_core::openhuman::composio::ComposioClient;
use openhuman_core::openhuman::config::{
    CapabilityProviderConfig, CapabilityProviderTrustState, Config, McpServerConfig,
};
use openhuman_core::openhuman::inference::provider::compatible::{
    AuthStyle, OpenAiCompatibleProvider,
};
use openhuman_core::openhuman::inference::provider::{
    ChatMessage, ChatRequest, Provider, ProviderDelta,
};
use openhuman_core::openhuman::integrations::IntegrationClient;
use openhuman_core::openhuman::tool_registry::{
    all_tool_registry_controller_schemas, all_tool_registry_registered_controllers,
    capability_provider_by_id, capability_provider_diagnostics, get_tool,
    is_capability_provider_trusted_enabled, list_capability_providers, list_tools,
    normalize_capability_provider_id,
};
use openhuman_core::openhuman::tool_registry::{
    denials as tool_registry_denials, ops as tool_registry_ops,
};
use openhuman_core::openhuman::tools::ToolSpec;

static OWNED_DOMAIN_ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Default)]
struct ProviderMockState {
    chat_requests: Arc<Mutex<Vec<Value>>>,
    response_requests: Arc<Mutex<Vec<Value>>>,
    auth_headers: Arc<Mutex<Vec<Option<String>>>>,
    user_agents: Arc<Mutex<Vec<Option<String>>>>,
}

#[derive(Clone, Default)]
struct ComposioMockState {
    requests: Arc<Mutex<Vec<(String, String, Option<Value>, Option<String>)>>>,
}

async fn serve_provider_mock() -> (String, ProviderMockState) {
    let state = ProviderMockState::default();
    let app = Router::new()
        .route("/v1/chat/completions", post(provider_chat))
        .route("/v1/responses", post(provider_responses))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind provider mock");
    let addr = listener.local_addr().expect("provider mock addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("provider mock serve");
    });
    (format!("http://{addr}/v1"), state)
}

async fn provider_chat(
    State(state): State<ProviderMockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    state
        .auth_headers
        .lock()
        .expect("auth headers")
        .push(header(&headers, "authorization"));
    state
        .user_agents
        .lock()
        .expect("user agents")
        .push(header(&headers, "user-agent"));
    state
        .chat_requests
        .lock()
        .expect("chat requests")
        .push(body.clone());

    if body.pointer("/stream").and_then(Value::as_bool) == Some(true) {
        return Json(json!({
            "choices": [{
                "message": {
                    "content": "stream fallback body",
                    "reasoning_content": "stream thinking"
                }
            }],
            "usage": {
                "prompt_tokens": 3,
                "completion_tokens": 4,
                "total_tokens": 7,
                "prompt_tokens_details": { "cached_tokens": 2 }
            }
        }))
        .into_response();
    }

    if body.get("tools").is_some() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "unknown parameter: tools" })),
        )
            .into_response();
    }

    if body.pointer("/model").and_then(Value::as_str) == Some("missing-chat") {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "chat completions unavailable" })),
        )
            .into_response();
    }

    Json(json!({
        "choices": [{
            "message": {
                "content": "<think>hidden</think>visible answer",
                "reasoning_content": "model reasoning",
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {
                        "name": "lookup",
                        "arguments": { "query": "openhuman" }
                    }
                }]
            }
        }],
        "openhuman": {
            "usage": {
                "input_tokens": 11,
                "output_tokens": 13,
                "cached_input_tokens": 5
            },
            "billing": { "charged_amount_usd": 0.0123 }
        }
    }))
    .into_response()
}

async fn provider_responses(
    State(state): State<ProviderMockState>,
    Json(body): Json<Value>,
) -> Response {
    state
        .response_requests
        .lock()
        .expect("response requests")
        .push(body);
    Json(json!({
        "output_text": "responses fallback answer",
        "output": []
    }))
    .into_response()
}

async fn serve_composio_mock() -> (String, ComposioMockState) {
    let state = ComposioMockState::default();
    let app = Router::new()
        .route(
            "/agent-integrations/composio/toolkits",
            get(composio_toolkits),
        )
        .route("/agent-integrations/composio/tools", get(composio_tools))
        .route(
            "/agent-integrations/composio/authorize",
            post(composio_authorize),
        )
        .route(
            "/agent-integrations/composio/connections/{id}",
            delete(composio_delete_connection),
        )
        .route(
            "/agent-integrations/composio/triggers/available",
            get(composio_available_triggers),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind composio mock");
    let addr = listener.local_addr().expect("composio mock addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("composio mock serve");
    });
    (format!("http://{addr}"), state)
}

async fn composio_toolkits(State(state): State<ComposioMockState>, headers: HeaderMap) -> Response {
    record_composio(
        &state,
        "GET",
        "/agent-integrations/composio/toolkits",
        None,
        &headers,
    );
    Json(json!({ "success": true, "data": { "toolkits": ["gmail", "github"] } })).into_response()
}

async fn composio_tools(
    State(state): State<ComposioMockState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    record_composio(
        &state,
        "GET",
        uri.path_and_query().unwrap().as_str(),
        None,
        &headers,
    );
    Json(json!({
        "success": true,
        "data": {
            "tools": [{
                "type": "function",
                "function": {
                    "name": "GMAIL_SEND_EMAIL",
                    "description": "Send email",
                    "parameters": { "type": "object" }
                }
            }]
        }
    }))
    .into_response()
}

async fn composio_authorize(
    State(state): State<ComposioMockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    record_composio(
        &state,
        "POST",
        "/agent-integrations/composio/authorize",
        Some(body),
        &headers,
    );
    Json(json!({
        "success": true,
        "data": {
            "connectUrl": "https://example.test/connect",
            "connectionId": "conn_123"
        }
    }))
    .into_response()
}

async fn composio_delete_connection(
    State(state): State<ComposioMockState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    record_composio(
        &state,
        "DELETE",
        &format!("/agent-integrations/composio/connections/{id}"),
        None,
        &headers,
    );
    Json(json!({
        "success": true,
        "data": { "deleted": true, "memory_chunks_deleted": 2 }
    }))
    .into_response()
}

async fn composio_available_triggers(
    State(state): State<ComposioMockState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    record_composio(
        &state,
        "GET",
        uri.path_and_query().unwrap().as_str(),
        None,
        &headers,
    );
    Json(json!({
        "success": true,
        "data": {
            "triggers": [{
                "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                "scope": "static",
                "defaultConfig": { "labelIds": ["INBOX"] },
                "requiredConfigKeys": ["labelIds"]
            }]
        }
    }))
    .into_response()
}

fn record_composio(
    state: &ComposioMockState,
    method: &str,
    path: &str,
    body: Option<Value>,
    headers: &HeaderMap,
) {
    state.requests.lock().expect("composio requests").push((
        method.to_string(),
        path.to_string(),
        body,
        header(headers, "authorization"),
    ));
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
}

fn owned_domain_config(workspace_root: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.workspace_dir = workspace_root.join("workspace");
    config.config_path = workspace_root.join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
    config
}

#[tokio::test]
async fn openai_compatible_provider_covers_auth_temperature_tool_fallback_and_responses() {
    let (base_url, state) = serve_provider_mock().await;
    let provider = OpenAiCompatibleProvider::new_with_user_agent(
        "owned-mock",
        &base_url,
        Some("secret-token"),
        AuthStyle::Bearer,
        "OpenHumanOwnedCoverage/1.0",
    )
    .with_temperature_unsupported_models(vec!["gpt-5*".to_string()]);

    let tool = ToolSpec {
        name: "lookup".to_string(),
        description: "Lookup a record".to_string(),
        parameters: json!({ "type": "object" }),
    };
    let messages = vec![ChatMessage::system("system"), ChatMessage::user("hello")];
    let fallback_response = provider
        .chat(
            ChatRequest {
                messages: &messages,
                tools: Some(&[tool.clone(), tool]),
                stream: None,
            },
            "gpt-5-mini",
            0.6,
        )
        .await
        .expect("provider chat with tool fallback");
    assert!(
        fallback_response
            .text
            .as_deref()
            .is_some_and(|text| text.contains("\"tool_calls\"")),
        "tool-schema fallback should return the history-path text payload"
    );
    assert!(fallback_response.tool_calls.is_empty());

    let response = provider
        .chat(
            ChatRequest {
                messages: &messages,
                tools: None,
                stream: None,
            },
            "gpt-5-mini",
            0.6,
        )
        .await
        .expect("provider native chat");
    assert_eq!(response.text.as_deref(), Some("visible answer"));
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "lookup");
    assert_eq!(response.tool_calls[0].arguments, r#"{"query":"openhuman"}"#);
    let usage = response.usage.expect("usage");
    assert_eq!(usage.input_tokens, 11);
    assert_eq!(usage.output_tokens, 13);
    assert_eq!(usage.cached_input_tokens, 5);
    assert_eq!(usage.charged_amount_usd, 0.0123);

    let chat_requests = state.chat_requests.lock().expect("chat requests").clone();
    assert!(
        chat_requests.len() >= 2,
        "tool rejection should force a retry without native tools"
    );
    assert_eq!(
        chat_requests[0].pointer("/tools/0/function/name"),
        Some(&json!("lookup"))
    );
    assert!(chat_requests[0].get("temperature").is_none());
    assert!(chat_requests[1].get("tools").is_none());

    let auth_headers = state.auth_headers.lock().expect("auth headers").clone();
    assert!(
        auth_headers
            .iter()
            .any(|header| header.as_deref() == Some("Bearer secret-token")),
        "bearer auth should be sent"
    );
    let user_agents = state.user_agents.lock().expect("user agents").clone();
    assert!(
        user_agents
            .iter()
            .any(|header| header.as_deref() == Some("OpenHumanOwnedCoverage/1.0")),
        "custom user-agent should be sent"
    );

    let fallback_text = provider
        .chat_with_history(&[ChatMessage::user("fallback please")], "missing-chat", 0.4)
        .await
        .expect("responses fallback");
    assert_eq!(fallback_text, "responses fallback answer");
    assert_eq!(
        state.response_requests.lock().expect("response requests")[0].pointer("/input/0/content"),
        Some(&json!([{"text": "fallback please", "type": "input_text"}]))
    );
}

#[tokio::test]
async fn openai_compatible_provider_streaming_json_fallback_aggregates_response() {
    let (base_url, _state) = serve_provider_mock().await;
    let provider = OpenAiCompatibleProvider::new("owned-mock", &base_url, None, AuthStyle::None);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderDelta>(4);
    let messages = vec![ChatMessage::user("stream please")];

    let response = provider
        .chat(
            ChatRequest {
                messages: &messages,
                tools: None,
                stream: Some(&tx),
            },
            "stream-model",
            0.7,
        )
        .await
        .expect("streaming JSON fallback");

    assert_eq!(response.text.as_deref(), Some("stream fallback body"));
    assert_eq!(
        response.reasoning_content.as_deref(),
        Some("stream thinking")
    );
    assert!(
        rx.try_recv().is_err(),
        "non-SSE fallback should not emit deltas"
    );
}

#[tokio::test]
async fn composio_client_round_trips_backend_paths_and_payload_normalization() {
    let (base_url, state) = serve_composio_mock().await;
    let client = ComposioClient::new(Arc::new(IntegrationClient::new(
        format!("{base_url}/openai/v1/chat/completions"),
        "jwt-token".to_string(),
    )));

    let toolkits = client.list_toolkits().await.expect("toolkits");
    assert_eq!(toolkits.toolkits, vec!["gmail", "github"]);

    let tools = client
        .list_tools(
            Some(&[
                " gmail ".to_string(),
                "".to_string(),
                "github repo".to_string(),
            ]),
            Some(&[" mail ".to_string()]),
        )
        .await
        .expect("tools");
    assert_eq!(tools.tools[0].function.name, "GMAIL_SEND_EMAIL");

    let authorized = client
        .authorize(
            " gmail ",
            Some(json!({
                "oauth_scopes": "profile https://www.googleapis.com/auth/gmail.readonly"
            })),
        )
        .await
        .expect("authorize");
    assert_eq!(authorized.connection_id, "conn_123");

    let triggers = client
        .list_available_triggers(" gmail ", Some("conn 123"))
        .await
        .expect("available triggers");
    assert_eq!(triggers.triggers[0].slug, "GMAIL_NEW_GMAIL_MESSAGE");

    let deleted = client
        .delete_connection(" conn_123 ")
        .await
        .expect("delete connection");
    assert!(deleted.deleted);
    assert_eq!(deleted.memory_chunks_deleted, 2);

    let requests = state.requests.lock().expect("composio requests").clone();
    assert_eq!(requests[0].3.as_deref(), Some("Bearer jwt-token"));
    assert!(
        requests.iter().any(|(_, path, _, _)| path
            == "/agent-integrations/composio/tools?toolkits=gmail,github%20repo&tags=mail"),
        "list_tools should trim blanks and URL-encode query values: {requests:?}"
    );
    let authorize_body = requests
        .iter()
        .find(|(method, path, _, _)| {
            method == "POST" && path == "/agent-integrations/composio/authorize"
        })
        .and_then(|(_, _, body, _)| body.clone())
        .expect("authorize body");
    assert_eq!(authorize_body.get("toolkit"), Some(&json!("gmail")));
    assert_eq!(
        authorize_body.pointer("/oauth_scopes/0"),
        Some(&json!("profile"))
    );
    assert!(authorize_body["oauth_scopes"]
        .as_array()
        .expect("oauth scopes")
        .iter()
        .any(|scope| scope == "https://www.googleapis.com/auth/gmail.readonly"));
    assert!(
        requests.iter().any(|(method, path, _, _)| {
            method == "DELETE" && path == "/agent-integrations/composio/connections/conn_123"
        }),
        "delete_connection should use DELETE route: {requests:?}"
    );
}

#[tokio::test]
async fn agent_task_board_store_normalizes_persists_and_surfaces_errors() {
    let dir = tempdir().expect("tempdir");
    let store = TaskBoardStore::new(dir.path().to_path_buf());

    assert_eq!(TaskCardStatus::InProgress.as_str(), "in_progress");
    assert_eq!(TaskApprovalMode::NotRequired.as_str(), "not_required");
    assert!(store.get(" missing ").expect("get missing").is_none());
    assert!(store
        .get("   ")
        .expect_err("blank id")
        .contains("thread_id"));

    let saved = store
        .put(TaskBoard {
            thread_id: " thread-owned ".to_string(),
            cards: vec![
                TaskBoardCard {
                    id: " ".to_string(),
                    title: "  Draft owned coverage  ".to_string(),
                    status: TaskCardStatus::Blocked,
                    objective: Some("  Raise raw coverage  ".to_string()),
                    plan: vec![
                        " inspect ".to_string(),
                        " ".to_string(),
                        " test ".to_string(),
                    ],
                    assigned_agent: Some(" agent ".to_string()),
                    allowed_tools: vec![" cargo ".to_string(), "".to_string()],
                    approval_mode: Some(TaskApprovalMode::Required),
                    acceptance_criteria: vec![" tests pass ".to_string()],
                    evidence: vec![" coverage measured ".to_string()],
                    notes: Some(" waiting ".to_string()),
                    session_thread_id: Some("  task-sess-owned  ".to_string()),
                    blocker: None,
                    source_metadata: None,
                    order: 99,
                    updated_at: String::new(),
                },
                TaskBoardCard {
                    id: "drop-me".to_string(),
                    title: "   ".to_string(),
                    status: TaskCardStatus::Todo,
                    objective: None,
                    plan: Vec::new(),
                    assigned_agent: None,
                    allowed_tools: Vec::new(),
                    approval_mode: None,
                    acceptance_criteria: Vec::new(),
                    evidence: Vec::new(),
                    notes: None,
                    session_thread_id: None,
                    blocker: None,
                    source_metadata: None,
                    order: 99,
                    updated_at: String::new(),
                },
            ],
            updated_at: String::new(),
        })
        .expect("put task board");

    assert_eq!(saved.thread_id, "thread-owned");
    assert_eq!(saved.cards.len(), 1);
    assert!(saved.cards[0].id.starts_with("task-"));
    assert_eq!(saved.cards[0].title, "Draft owned coverage");
    assert_eq!(saved.cards[0].plan, vec!["inspect", "test"]);
    assert_eq!(saved.cards[0].blocker.as_deref(), Some("waiting"));
    // A padded session_thread_id is trimmed (not dropped) on persist.
    assert_eq!(
        saved.cards[0].session_thread_id.as_deref(),
        Some("task-sess-owned")
    );
    assert_eq!(saved.cards[0].order, 0);

    let loaded = board_for_thread(dir.path(), " thread-owned ")
        .expect("board_for_thread")
        .cards;
    assert_eq!(loaded[0].approval_mode, Some(TaskApprovalMode::Required));
    // …and the normalized value survives a reload from disk.
    assert_eq!(
        loaded[0].session_thread_id.as_deref(),
        Some("task-sess-owned")
    );

    assert!(store.delete("thread-owned").expect("delete present"));
    assert!(!store.delete("thread-owned").expect("delete missing"));

    let missing = board_for_thread(dir.path(), "thread-owned").expect("missing board");
    assert!(missing.cards.is_empty());
}

#[test]
fn tool_registry_public_apis_cover_entries_diagnostics_and_provider_policy() {
    let dir = tempdir().expect("tempdir");
    let mut config = owned_domain_config(dir.path());
    config.mcp_client.enabled = true;
    config.mcp_client.servers = vec![
        McpServerConfig {
            name: "filesystem".to_string(),
            enabled: true,
            allowed_tools: vec!["read_file".to_string(), "write_file".to_string()],
            disallowed_tools: vec!["delete_file".to_string()],
            ..McpServerConfig::default()
        },
        McpServerConfig {
            name: "disabled".to_string(),
            enabled: false,
            ..McpServerConfig::default()
        },
    ];
    config.capability_providers = vec![
        CapabilityProviderConfig {
            id: " Acme Tools ".to_string(),
            display_name: "  Acme Tooling  ".to_string(),
            source_uri: Some(" https://example.test/acme.json ".to_string()),
            source_digest: Some(" sha256:abc ".to_string()),
            trust_state: CapabilityProviderTrustState::Trusted,
            enabled: true,
        },
        CapabilityProviderConfig {
            id: "zeta.provider".to_string(),
            display_name: "   ".to_string(),
            source_uri: Some(" ".to_string()),
            source_digest: None,
            trust_state: CapabilityProviderTrustState::Untrusted,
            enabled: false,
        },
    ];

    assert_eq!(
        normalize_capability_provider_id(" Acme Tools "),
        Ok("acme-tools".to_string())
    );
    assert!(normalize_capability_provider_id("   ").is_err());
    assert!(normalize_capability_provider_id(&"x".repeat(120)).is_err());

    let providers = list_capability_providers(&config).expect("providers");
    assert_eq!(
        providers
            .iter()
            .map(|provider| provider.id.as_str())
            .collect::<Vec<_>>(),
        vec!["acme-tools", "zeta.provider"]
    );
    assert_eq!(providers[0].display_name, "Acme Tooling");
    assert_eq!(
        providers[0].source_uri.as_deref(),
        Some("https://example.test/acme.json")
    );
    assert_eq!(providers[1].display_name, "zeta.provider");
    assert!(providers[1].source_uri.is_none());

    let acme = capability_provider_by_id(&config, " acme tools ")
        .expect("provider lookup")
        .expect("acme provider");
    assert_eq!(acme.id, "acme-tools");
    assert!(is_capability_provider_trusted_enabled(
        &config,
        "ACME TOOLS"
    ));
    assert!(!is_capability_provider_trusted_enabled(
        &config,
        "zeta.provider"
    ));
    assert!(capability_provider_by_id(&config, "missing")
        .expect("missing provider lookup")
        .is_none());

    let diagnostics = tool_registry_ops::diagnostics_for_config(&config).value;
    assert!(diagnostics.total_tools > 0);
    assert!(diagnostics.enabled_tools > 0);
    assert!(diagnostics.json_rpc_tools > 0);
    assert!(diagnostics
        .possible_write_surfaces
        .iter()
        .any(|tool_id| tool_id.contains("execute") || tool_id.contains("write")));
    assert!(diagnostics
        .policy_surfaces
        .iter()
        .any(|tool_id| tool_id == "tool_registry.diagnostics"));
    assert_eq!(diagnostics.mcp_allowlists.server_count, 2);
    assert_eq!(diagnostics.mcp_allowlists.enabled_server_count, 1);
    assert!(diagnostics.mcp_allowlists.servers[0].has_allowlist);
    assert!(diagnostics.mcp_allowlists.servers[0].has_denylist);
    assert_eq!(diagnostics.capability_providers.total_providers, 2);
    assert_eq!(diagnostics.capability_providers.enabled_providers, 1);
    assert_eq!(
        diagnostics.capability_providers.trusted_enabled_providers,
        1
    );

    let mut duplicate_config = owned_domain_config(dir.path());
    duplicate_config.capability_providers = vec![
        CapabilityProviderConfig {
            id: "Acme Tools".to_string(),
            ..CapabilityProviderConfig::default()
        },
        CapabilityProviderConfig {
            id: "acme-tools".to_string(),
            ..CapabilityProviderConfig::default()
        },
    ];
    let duplicate_diagnostics = capability_provider_diagnostics(&duplicate_config);
    assert_eq!(duplicate_diagnostics.total_providers, 2);
    assert!(duplicate_diagnostics.registry_errors[0].contains("duplicate"));

    tool_registry_denials::record(
        "tools.write_file",
        "approval_required",
        "write",
        "medium risk",
    );
    let denials = tool_registry_denials::list(1);
    assert_eq!(denials.len(), 1);
    assert_eq!(denials[0].tool_name, "tools.write_file");
    tool_registry_denials::record("", "policy", "blocked", "ignored");
    tool_registry_denials::record(" tools.blank ", "", "", "");
    tool_registry_denials::record("tools.secret", "policy", "blocked", "Bearer secret");
    tool_registry_denials::record("tools.long", "policy", "blocked", &"a".repeat(500));
    let recent_denials = tool_registry_denials::list(4);
    assert_eq!(recent_denials[0].tool_name, "tools.long");
    assert_eq!(recent_denials[0].reason.chars().count(), 241);
    assert_eq!(recent_denials[1].reason, "[redacted: sensitive content]");
    assert_eq!(recent_denials[2].policy, "unknown");
    assert_eq!(recent_denials[2].action, "blocked");
    assert_eq!(recent_denials[2].reason, "<empty>");

    let registry = list_tools().value.tools;
    assert!(registry.iter().any(|entry| {
        entry.tool_id == "tools.web_search"
            && entry.tags.iter().any(|tag| tag == "retrieval")
            && entry.route.pointer("/protocol").and_then(Value::as_str) == Some("json_rpc")
    }));
    assert!(registry.iter().any(|entry| {
        entry.tool_id.contains("memory") && entry.tags.iter().any(|tag| tag == "memory")
    }));

    let web_search = get_tool(" tools.web_search ")
        .expect("get web search")
        .value;
    assert_eq!(web_search.title, "Web Search");
    assert_eq!(
        web_search
            .input_schema
            .pointer("/additionalProperties")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert!(get_tool("   ")
        .expect_err("blank tool id")
        .contains("tool_id"));
    assert!(get_tool("missing.tool")
        .expect_err("missing tool")
        .contains("tool not found"));
}

#[tokio::test]
async fn tool_registry_controller_handlers_cover_list_get_and_validation_paths() {
    let schemas = all_tool_registry_controller_schemas();
    assert_eq!(
        schemas
            .iter()
            .map(|schema| schema.function)
            .collect::<Vec<_>>(),
        vec!["list", "get", "diagnostics"]
    );

    let controllers = all_tool_registry_registered_controllers();
    assert_eq!(controllers.len(), 3);

    let list_handler = controllers
        .iter()
        .find(|controller| controller.schema.function == "list")
        .expect("list controller")
        .handler;
    let list_value = list_handler(Map::new()).await.expect("list value");
    assert!(list_value
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| !tools.is_empty()));

    let get_handler = controllers
        .iter()
        .find(|controller| controller.schema.function == "get")
        .expect("get controller")
        .handler;
    let mut params = Map::new();
    params.insert("tool_id".to_string(), json!("tools.web_search"));
    let get_value = get_handler(params).await.expect("get value");
    assert_eq!(
        get_value.get("tool_id").and_then(Value::as_str),
        Some("tools.web_search")
    );

    let mut blank_params = Map::new();
    blank_params.insert("tool_id".to_string(), json!("  "));
    assert!(get_handler(blank_params)
        .await
        .expect_err("blank tool id")
        .contains("non-empty string"));

    let mut typed_params = Map::new();
    typed_params.insert("tool_id".to_string(), json!(42));
    assert!(get_handler(typed_params)
        .await
        .expect_err("numeric tool id")
        .contains("non-empty string"));

    let diagnostics_handler = controllers
        .iter()
        .find(|controller| controller.schema.function == "diagnostics")
        .expect("diagnostics controller")
        .handler;
    let dir = tempdir().expect("tempdir");
    let previous_workspace = {
        let _env_guard = OWNED_DOMAIN_ENV_LOCK.lock().expect("env lock");
        let previous_workspace = std::env::var_os("OPENHUMAN_WORKSPACE");
        std::env::set_var("OPENHUMAN_WORKSPACE", dir.path());
        previous_workspace
    };
    let diagnostics_value = diagnostics_handler(Map::new())
        .await
        .expect("diagnostics value");
    {
        let _env_guard = OWNED_DOMAIN_ENV_LOCK.lock().expect("env lock");
        match previous_workspace {
            Some(value) => std::env::set_var("OPENHUMAN_WORKSPACE", value),
            None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
        }
    }
    assert!(diagnostics_value
        .get("total_tools")
        .or_else(|| diagnostics_value.pointer("/diagnostics/total_tools"))
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 0));
}
