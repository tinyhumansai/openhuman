//! Raw-line oriented E2E coverage for the tool_registry and approval domains.
//!
//! These tests intentionally mix JSON-RPC calls with the public domain APIs that
//! back those calls. JSON-RPC drives the externally visible controller paths;
//! direct public API calls cover persistence/redaction/provider branches that
//! are otherwise only indirectly reachable from the controllers.

use crate::env_guard::EnvVarGuard;
use crate::rpc_harness::{error_message, payload};
use openhuman_core::core::runtime::DispatchCtx;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::http::header::AUTHORIZATION;
use reqwest::StatusCode;
use serde_json::{json, Map, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::agent::turn_origin::{self, AgentTurnOrigin};
use openhuman_core::config::Config;
use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::mcp::registry::connections;
use openhuman_core::mcp::registry::types::{CommandKind, InstalledServer, Transport};
use openhuman_core::security::approval::gate::{
    ApprovalChatContext, ApprovalGate, ApprovalTurnContext,
};
use openhuman_core::security::approval::{
    all_approval_controller_schemas, all_approval_registered_controllers, ExecutionOutcome,
    GateOutcome,
};
use openhuman_core::security::{live_policy, SecurityPolicy};
use openhuman_core::tools::registry::{registry_entries, registry_entries_for_config};
use openhuman_rpc::server::build_core_http_router;
use tinytools::registry::denials;

const TEST_RPC_TOKEN: &str = "tool-registry-approval-raw-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();
static ENV_LOCK: &OnceLock<tokio::sync::Mutex<()>> = &crate::SHARED_ENV_LOCK;

struct TestHarness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    rpc_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .blocking_lock()
}

async fn env_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

fn ensure_rpc_auth() {
    crate::tinyhumans_boot::boot();
    AUTH_INIT.get_or_init(|| {
        std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN);
        let token_dir = std::env::temp_dir().join("openhuman-tool-registry-approval-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
}

/// The bearer this process actually validates.
///
/// `core::auth::RPC_TOKEN` is a process-global `OnceLock` and `init_rpc_token`
/// returns early once it is set — deliberately, so a second call cannot 401 live
/// clients. Since `tests/raw_coverage/` is one aggregated binary, only the first
/// suite to reach `ensure_rpc_auth` pins its own `TEST_RPC_TOKEN`; every other
/// suite sending its literal gets a 401 and trips its own `assert_eq!` (#6112).
/// Ask the auth module what it settled on instead of assuming we won the race.
fn rpc_bearer() -> &'static str {
    ensure_rpc_auth();
    openhuman_core::core::auth::get_rpc_token()
        .expect("ensure_rpc_auth initialises the token subsystem on the line above")
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

fn write_config(openhuman_dir: &Path, capability_providers: &str) {
    std::fs::create_dir_all(openhuman_dir).expect("create .openhuman");
    let cfg = format!(
        r#"api_url = "http://127.0.0.1:9"
default_model = "e2e-model"
default_temperature = 0.2

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
level = "supervised"
workspace_only = false
max_actions_per_hour = 17
require_approval_for_medium_risk = false
block_high_risk_commands = false
auto_approve = []

[mcp_client]
enabled = true

[[mcp_client.servers]]
name = "filesystem"
command = "node"
args = ["server.js"]
enabled = true
allowed_tools = ["read_file", "list_directory"]
disallowed_tools = ["write_file"]

{capability_providers}
"#
    );
    std::fs::write(openhuman_dir.join("config.toml"), cfg).expect("write config.toml");
}

async fn setup(capability_providers: &str) -> TestHarness {
    crate::tinyhumans_boot::boot();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let workspace = home.join("openhuman-workspace");
    write_config(&workspace, capability_providers);
    write_config(&home.join(".openhuman"), capability_providers);

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];

    let (addr, rpc_join) = serve_rpc().await;
    TestHarness {
        _tmp: tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        rpc_join,
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
        .header(AUTHORIZATION, format!("Bearer {}", rpc_bearer()))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .unwrap_or_else(|err| panic!("POST {url} {method}: {err}"));
    assert_eq!(response.status(), StatusCode::OK, "{method} HTTP status");
    response
        .json::<Value>()
        .await
        .unwrap_or_else(|err| panic!("json for {method}: {err}"))
}

fn test_mcp_server() -> InstalledServer {
    InstalledServer {
        server_id: format!("tool-registry-test-{}", uuid::Uuid::new_v4()),
        qualified_name: "@openhuman-test/echo".to_string(),
        display_name: "Test Echo".to_string(),
        description: Some("Stub MCP server used by tool registry coverage tests.".to_string()),
        icon_url: None,
        command_kind: CommandKind::Binary,
        command: env!("CARGO_BIN_EXE_test-mcp-stub").to_string(),
        args: Vec::new(),
        env_keys: Vec::new(),
        config: None,
        installed_at: 0,
        last_connected_at: None,
        transport: Transport::Stdio,
        enabled: true,
    }
}

async fn install_test_mcp_server(config: &Config, server: &InstalledServer) -> InstalledServer {
    openhuman_core::mcp::registry::config_ops::mcp_clients_config_set(
        config,
        json!({
            "mcpServers": {
                server.qualified_name.clone(): {
                    "command": server.command.clone(),
                    "args": server.args.clone(),
                    "enabled": false,
                }
            }
        }),
    )
    .await
    .expect("install test MCP server through the module-owned config document");
    let status = openhuman_core::mcp::registry::ops::mcp_clients_status(config)
        .await
        .expect("read module-owned MCP server status");
    let server_id = status.value["servers"]
        .as_array()
        .and_then(|servers| {
            servers
                .iter()
                .find(|entry| entry["qualified_name"] == server.qualified_name)
        })
        .and_then(|entry| entry["server_id"].as_str())
        .expect("the installed server has a module-owned id")
        .to_owned();
    let mut installed = server.clone();
    installed.server_id = server_id;
    installed.enabled = false;
    installed
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_registry_rpc_diagnostics_include_denials_and_provider_errors() {
    let _lock = env_lock_async().await;
    let harness = setup(
        r#"
[[capability_providers]]
id = "Acme Tools"
display_name = "Acme Tools"
trust_state = "trusted"
enabled = true

[[capability_providers]]
id = "acme-tools"
display_name = "Duplicate Acme"
trust_state = "trusted"
enabled = true
"#,
    )
    .await;

    denials::record("   ", "policy", "blocked", "ignored blank tool");
    denials::record(
        "tools.secret",
        "external-write",
        "denied",
        "blocked Authorization: Bearer sk-secret-abcdefghijklmnopqrstuvwxyz",
    );
    denials::record("tools.long", "", "", &"x".repeat(320));

    let diagnostics = rpc(
        &harness.rpc_base,
        10,
        "openhuman.tool_registry_diagnostics",
        json!({}),
    )
    .await;
    let diagnostics = payload(&diagnostics, "tool_registry_diagnostics");

    assert!(
        diagnostics
            .get("total_tools")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0),
        "registry should expose tools: {diagnostics}"
    );
    assert_eq!(
        diagnostics
            .pointer("/mcp_allowlists/enabled")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        diagnostics
            .pointer("/mcp_allowlists/servers/0/allowed_tools_count")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert!(diagnostics
        .pointer("/possible_write_surfaces")
        .and_then(Value::as_array)
        .expect("write surfaces")
        .iter()
        .any(|tool| tool.as_str() == Some("tools.composio_execute")));

    let recent_denials = diagnostics
        .get("recent_denials")
        .and_then(Value::as_array)
        .expect("recent denials array");
    assert!(recent_denials
        .iter()
        .any(|row| row.get("reason").and_then(Value::as_str)
            == Some("[redacted: sensitive content]")));
    assert!(recent_denials.iter().any(|row| {
        row.get("policy").and_then(Value::as_str) == Some("unknown")
            && row.get("action").and_then(Value::as_str) == Some("blocked")
            && row
                .get("reason")
                .and_then(Value::as_str)
                .is_some_and(|reason| reason.ends_with('…'))
    }));

    assert_eq!(
        diagnostics
            .pointer("/capability_providers/total_providers")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert!(diagnostics
        .pointer("/capability_providers/registry_errors/0")
        .and_then(Value::as_str)
        .is_some_and(|err| err.contains("duplicate provider id after normalization")));

    let list = rpc(
        &harness.rpc_base,
        11,
        "openhuman.tool_registry_list",
        json!({}),
    )
    .await;
    let listed_tools = payload(&list, "tool_registry_list")
        .get("tools")
        .and_then(Value::as_array)
        .expect("tool registry list");
    let first_tool_id = listed_tools
        .first()
        .and_then(|tool| tool.get("tool_id"))
        .and_then(Value::as_str)
        .expect("first tool id")
        .to_string();

    let found = rpc(
        &harness.rpc_base,
        12,
        "openhuman.tool_registry_get",
        json!({ "tool_id": format!("  {first_tool_id}  ") }),
    )
    .await;
    assert_eq!(
        payload(&found, "tool_registry_get success")
            .get("tool_id")
            .and_then(Value::as_str),
        Some(first_tool_id.as_str())
    );

    let empty = rpc(
        &harness.rpc_base,
        13,
        "openhuman.tool_registry_get",
        json!({ "tool_id": "   " }),
    )
    .await;
    assert!(error_message(&empty, "empty tool id").contains("non-empty"));

    let missing = rpc(
        &harness.rpc_base,
        14,
        "openhuman.tool_registry_get",
        json!({ "tool_id": "missing.tool" }),
    )
    .await;
    assert!(error_message(&missing, "missing tool").contains("missing.tool"));

    harness.rpc_join.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn tool_registry_entries_fall_back_on_current_thread_runtime() {
    let entries = registry_entries(&openhuman_core::core::runtime::DispatchCtx::host_default());
    assert!(entries
        .iter()
        .any(|entry| entry.tool_id == "tools.web_search"));
    assert!(entries
        .iter()
        .all(|entry| !entry.tool_id.starts_with("mcp-client::")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_registry_entries_include_connected_mcp_client_tools() {
    let tmp = tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..Config::default()
    };
    let server = install_test_mcp_server(&config, &test_mcp_server()).await;
    openhuman_core::mcp::registry::ops::mcp_clients_set_enabled(
        &config,
        server.server_id.clone(),
        true,
    )
    .await
    .expect("enable test MCP server");
    let tools = connections::connect(&config, &server)
        .await
        .expect("connect test mcp server");
    assert_eq!(tools.first().map(|tool| tool.name.as_str()), Some("echo"));

    // A second workspace, so that scoping is what the assertions below actually
    // test. With one workspace open, `registry_entries()` and the config-scoped
    // form agree, and this case would keep passing if the forwarding regressed.
    let other_tmp = tempdir().expect("second tempdir");
    let other_config = Config {
        workspace_dir: other_tmp.path().to_path_buf(),
        ..Config::default()
    };
    let other_server = install_test_mcp_server(&other_config, &test_mcp_server()).await;
    openhuman_core::mcp::registry::ops::mcp_clients_set_enabled(
        &other_config,
        other_server.server_id.clone(),
        true,
    )
    .await
    .expect("enable second test MCP server");
    connections::connect(&other_config, &other_server)
        .await
        .expect("connect second test mcp server");

    // Config-scoped, not ambient: this case connects through
    // `host::for_config(&config)`, keyed by its own tempdir. `registry_entries()`
    // resolves through the process default instead, which returns a lone host
    // but `None` once another case in this binary has opened a second one — so
    // the ambient form reports nothing connected here purely because of who
    // else ran first.
    let entries = registry_entries_for_config(&config);
    let client_entry = entries
        .iter()
        .find(|entry| entry.tool_id == format!("mcp-client::{}::echo", server.server_id))
        .expect("connected mcp client entry");
    assert_eq!(client_entry.name, "echo");
    assert_eq!(client_entry.route["protocol"], json!("mcp-client"));
    assert_eq!(client_entry.route["server_id"], json!(server.server_id));
    assert!(client_entry.tags.iter().any(|tag| tag == "mcp_client"));

    // The other workspace's server must NOT leak in. This is the assertion that
    // fails if a config-scoped lookup falls back to the process default.
    assert!(
        !entries
            .iter()
            .any(|entry| entry.tool_id == format!("mcp-client::{}::echo", other_server.server_id)),
        "entries for one workspace must not include another workspace's server"
    );

    // Symmetrically, from the second workspace's side.
    let other_entries = registry_entries_for_config(&other_config);
    assert!(other_entries
        .iter()
        .any(|entry| entry.tool_id == format!("mcp-client::{}::echo", other_server.server_id)));
    assert!(!other_entries
        .iter()
        .any(|entry| entry.tool_id == format!("mcp-client::{}::echo", server.server_id)));

    // Config-scoped for the same reason as the lookup above: this connection
    // lives in the host keyed by `config`'s workspace, and the by-id form
    // resolves through the process default.
    assert!(connections::disconnect_for_config(&config, &server.server_id).await);
    assert!(connections::disconnect_for_config(&other_config, &other_server.server_id).await);
}

#[tokio::test]
async fn tool_registry_diagnostics_reports_config_and_audit_store_failures() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let workspace_file = tmp.path().join("workspace-file");
    std::fs::write(&workspace_file, "not a directory").expect("workspace sentinel");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace_file);

    let err = openhuman_core::tools::registry::ops::diagnostics()
        .await
        .expect_err("workspace file should prevent config load");
    assert!(err.contains("failed to load config for tool registry diagnostics"));

    // The write audit lives in the TinyMCP module (605ca6e841), which picks its
    // storage provider once per process at load, so a broken workspace no
    // longer reaches it. Fail it at the module seam instead; storage failures
    // inside the audit are covered by tinymcp's `storage/failing.rs`.
    let mut broken_audit_config = Config {
        workspace_dir: workspace_file,
        ..Config::default()
    };
    broken_audit_config.modules.enabled = false;
    let diagnostics =
        openhuman_core::tools::registry::ops::diagnostics_for_config(&broken_audit_config).await;
    assert!(diagnostics.value.mcp_write_audit.enabled);
    assert_eq!(diagnostics.value.mcp_write_audit.recent_rows, None);
    assert!(diagnostics
        .value
        .mcp_write_audit
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("MCP module unavailable")));
}

#[tokio::test]
async fn approval_schema_handlers_validate_params_and_surface_empty_gate_state() {
    let schemas = all_approval_controller_schemas();
    assert_eq!(
        schemas
            .iter()
            .map(|schema| schema.function)
            .collect::<Vec<_>>(),
        vec![
            "list_pending",
            "list_recent_decisions",
            "decide",
            "get_gate_state",
            "preauthorize_flow"
        ]
    );
    let unknown = openhuman_core::security::approval::schemas::schemas("missing");
    assert_eq!(unknown.namespace, "approval");
    assert_eq!(unknown.function, "unknown");
    assert_eq!(unknown.outputs[0].name, "error");

    let controllers = all_approval_registered_controllers();
    assert_eq!(controllers.len(), schemas.len());

    let list_handler = controllers
        .iter()
        .find(|controller| controller.schema.function == "list_pending")
        .expect("list pending controller")
        .handler;
    let list_value = list_handler(DispatchCtx::process_default(), Map::new())
        .await
        .expect("list pending value");
    assert!(list_value
        .get("result")
        .or(Some(&list_value))
        .and_then(Value::as_array)
        .is_some());

    let recent_handler = controllers
        .iter()
        .find(|controller| controller.schema.function == "list_recent_decisions")
        .expect("recent decisions controller")
        .handler;
    let mut invalid_limit = Map::new();
    invalid_limit.insert("limit".to_string(), json!("ten"));
    assert!(
        recent_handler(DispatchCtx::process_default(), invalid_limit)
            .await
            .expect_err("string limit")
            .contains("expected unsigned integer")
    );
    for invalid in [json!(true), json!([]), json!({ "limit": 10 })] {
        let mut invalid_limit = Map::new();
        invalid_limit.insert("limit".to_string(), invalid);
        assert!(
            recent_handler(DispatchCtx::process_default(), invalid_limit)
                .await
                .expect_err("non-numeric limit")
                .contains("expected unsigned integer")
        );
    }
    let mut negative_limit = Map::new();
    negative_limit.insert("limit".to_string(), json!(-1));
    assert!(
        recent_handler(DispatchCtx::process_default(), negative_limit)
            .await
            .expect_err("negative limit")
            .contains("expected unsigned integer")
    );
    let mut null_limit = Map::new();
    null_limit.insert("limit".to_string(), Value::Null);
    let recent_value = recent_handler(DispatchCtx::process_default(), null_limit)
        .await
        .expect("null limit should use default");
    assert!(recent_value
        .get("result")
        .or(Some(&recent_value))
        .and_then(Value::as_array)
        .is_some());

    let decide_handler = controllers
        .iter()
        .find(|controller| controller.schema.function == "decide")
        .expect("decide controller")
        .handler;
    assert!(decide_handler(DispatchCtx::process_default(), Map::new())
        .await
        .expect_err("missing request id")
        .contains("missing required param 'request_id'"));
    let mut numeric_request = Map::new();
    numeric_request.insert("request_id".to_string(), json!(42));
    numeric_request.insert("decision".to_string(), json!("deny"));
    assert!(
        decide_handler(DispatchCtx::process_default(), numeric_request)
            .await
            .expect_err("numeric request id")
            .contains("expected string")
    );
    for invalid in [
        Value::Null,
        json!(false),
        json!([]),
        json!({ "id": "missing" }),
    ] {
        let mut invalid_request = Map::new();
        invalid_request.insert("request_id".to_string(), invalid);
        invalid_request.insert("decision".to_string(), json!("deny"));
        assert!(
            decide_handler(DispatchCtx::process_default(), invalid_request)
                .await
                .expect_err("non-string request id")
                .contains("expected string")
        );
    }
    let mut numeric_decision = Map::new();
    numeric_decision.insert("request_id".to_string(), json!("missing"));
    numeric_decision.insert("decision".to_string(), json!(42));
    assert!(
        decide_handler(DispatchCtx::process_default(), numeric_decision)
            .await
            .expect_err("numeric decision")
            .contains("expected string")
    );
    let mut invalid_decision = Map::new();
    invalid_decision.insert("request_id".to_string(), json!("missing"));
    invalid_decision.insert("decision".to_string(), json!("maybe"));
    assert!(
        decide_handler(DispatchCtx::process_default(), invalid_decision)
            .await
            .expect_err("invalid decision")
            .contains("approve_once|approve_always_for_tool|approve_always_for_flow|deny")
    );

    let preauthorize_handler = controllers
        .iter()
        .find(|controller| controller.schema.function == "preauthorize_flow")
        .expect("preauthorize flow controller")
        .handler;
    assert!(
        preauthorize_handler(DispatchCtx::process_default(), Map::new())
            .await
            .expect_err("missing flow id")
            .contains("missing required param 'flow_id'")
    );
    let mut missing_tools = Map::new();
    missing_tools.insert("flow_id".to_string(), json!("flow-1"));
    assert!(
        preauthorize_handler(DispatchCtx::process_default(), missing_tools)
            .await
            .expect_err("missing tool names")
            .contains("missing required param 'tool_names'")
    );
    let mut non_array_tools = Map::new();
    non_array_tools.insert("flow_id".to_string(), json!("flow-1"));
    non_array_tools.insert("tool_names".to_string(), json!("slack_post"));
    assert!(
        preauthorize_handler(DispatchCtx::process_default(), non_array_tools)
            .await
            .expect_err("non-array tool names")
            .contains("expected array of strings")
    );
    let mut mixed_tools = Map::new();
    mixed_tools.insert("flow_id".to_string(), json!("flow-1"));
    mixed_tools.insert("tool_names".to_string(), json!(["ok", 42]));
    assert!(
        preauthorize_handler(DispatchCtx::process_default(), mixed_tools)
            .await
            .expect_err("non-string tool name entry")
            .contains("expected string")
    );
    // Success-path behavior (gate-absent tolerance, idempotent grants, audit
    // rows) is pinned by the unit tests in `approval::rpc`/`approval::store`;
    // asserting it here would be order-dependent on whether a sibling test
    // already installed the process-global gate.
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_rpc_decision_paths_persist_always_allow_and_recent_audit() {
    let _lock = env_lock_async().await;
    let harness = setup("").await;
    let config = Config::load_or_init()
        .await
        .expect("load config for approval gate");
    let test_session_id = format!("session-{}", uuid::Uuid::new_v4());
    let gate = ApprovalGate::init_global(config.clone(), test_session_id.clone());
    let gate_for_task = gate.clone();

    let approval_task = tokio::spawn(async move {
        // Scope a WebChat origin alongside the chat context — the gate now
        // requires an origin label or it fails closed on `Unknown`.
        turn_origin::with_origin(
            AgentTurnOrigin::WebChat {
                thread_id: "approval-raw-thread".to_string(),
                client_id: "approval-raw-client".to_string(),
                request_id: None,
            },
            async move {
                let turn = ApprovalTurnContext::chat(ApprovalChatContext {
                    thread_id: "approval-raw-thread".to_string(),
                    client_id: "approval-raw-client".to_string(),
                    // No turn in scope in this fixture; the field is documented as
                    // carried only when the caller has one (`gate.rs:91-95`).
                    request_id: None,
                });
                gate_for_task
                    .intercept_audited(
                        &turn,
                        "tools.composio_execute",
                        "tools.composio_execute(action=execute, 123 bytes)",
                        json!({
                            "action": "execute",
                            "tool_slug": "GMAIL_SEND_EMAIL",
                            "body": "<redacted: string (500 chars)>"
                        }),
                    )
                    .await
            },
        )
        .await
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    let request_id = loop {
        let pending = rpc(
            &harness.rpc_base,
            20,
            "openhuman.approval_list_pending",
            json!({}),
        )
        .await;
        let rows = payload(&pending, "approval_list_pending")
            .as_array()
            .expect("pending rows");
        if let Some(row) = rows.iter().find(|row| {
            row.get("tool_name").and_then(Value::as_str) == Some("tools.composio_execute")
        }) {
            break row
                .get("request_id")
                .and_then(Value::as_str)
                .expect("request id")
                .to_string();
        }
        assert!(Instant::now() < deadline, "pending approval did not appear");
        tokio::time::sleep(Duration::from_millis(25)).await;
    };

    assert_eq!(
        gate.pending_for_thread("approval-raw-thread").as_deref(),
        Some(request_id.as_str())
    );

    let invalid = rpc(
        &harness.rpc_base,
        21,
        "openhuman.approval_decide",
        json!({ "request_id": request_id, "decision": "maybe" }),
    )
    .await;
    assert!(error_message(&invalid, "invalid decision").contains("invalid 'decision'"));

    let decide = rpc(
        &harness.rpc_base,
        22,
        "openhuman.approval_decide",
        json!({
            "request_id": request_id,
            "decision": "approve_always_for_tool"
        }),
    )
    .await;
    assert_eq!(
        payload(&decide, "approval_decide")
            .get("tool_name")
            .and_then(Value::as_str),
        Some("tools.composio_execute")
    );

    let (outcome, approved_id) = approval_task.await.expect("approval task");
    assert!(matches!(
        outcome,
        openhuman_core::security::approval::GateOutcome::Allow
    ));
    assert_eq!(approved_id.as_deref(), Some(request_id.as_str()));
    gate.record_execution(
        &request_id,
        ExecutionOutcome::Aborted,
        Some("aborted after approval"),
    );
    gate.record_execution(
        "missing-gate-row",
        ExecutionOutcome::Failure,
        Some("missing row"),
    );
    assert!(gate.pending_for_thread("approval-raw-thread").is_none());

    let duplicate_decide = rpc(
        &harness.rpc_base,
        23,
        "openhuman.approval_decide",
        json!({ "request_id": request_id, "decision": "deny" }),
    )
    .await;
    assert!(error_message(&duplicate_decide, "duplicate decide").contains("no pending approval"));

    let recent = rpc(
        &harness.rpc_base,
        24,
        "openhuman.approval_list_recent_decisions",
        json!({ "limit": 1 }),
    )
    .await;
    let rows = payload(&recent, "approval_list_recent_decisions")
        .as_array()
        .expect("recent decisions");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("decision").and_then(Value::as_str),
        Some("approve_always_for_tool")
    );

    let config_after = Config::load_or_init()
        .await
        .expect("reload config after always allow");
    assert!(
        config_after
            .autonomy
            .auto_approve
            .iter()
            .any(|tool| tool == "tools.composio_execute"),
        "approve_always_for_tool should persist an auto-approve entry"
    );

    // Bare call with neither a chat context nor an AgentTurnOrigin scope:
    // the gate now treats this as `Unknown` and fails closed (refuses to
    // execute an external_effect tool from an unlabelled call site). The
    // earlier "non-chat ⇒ Allow" behaviour leaked trusted execution to any
    // caller that forgot to scope a label.
    let no_chat = gate
        .intercept_audited(
            &Default::default(),
            "tools.web_search",
            "tools.web_search(query=coverage)",
            json!({ "query": "coverage" }),
        )
        .await;
    match &no_chat.0 {
        openhuman_core::security::approval::GateOutcome::Deny { reason } => {
            assert!(
                reason.contains("origin label"),
                "unlabelled call should be denied for missing origin: {reason}"
            );
        }
        other => panic!("expected Deny for unlabelled call, got {other:?}"),
    }
    assert_eq!(
        no_chat.1, None,
        "denied calls should not create approval rows"
    );
    assert!(matches!(
        gate.intercept(
            &Default::default(),
            "tools.web_search",
            "tools.web_search(query=legacy)",
            json!({ "query": "legacy" }),
        )
        .await,
        GateOutcome::Deny { .. }
    ));

    // Always-allowed tools should bypass approval even when an origin is
    // scoped — the auto_approve allowlist short-circuit runs before the
    // origin branch. Install a live policy with the persisted entry so the
    // gate sees the latest auto_approve set (the gate's boot-time config
    // snapshot predates the approve_always_for_tool decision we just made).
    live_policy::install(
        Arc::new(SecurityPolicy {
            workspace_dir: config.workspace_dir.clone(),
            auto_approve: vec!["tools.composio_execute".to_string()],
            ..SecurityPolicy::default()
        }),
        config.workspace_dir.clone(),
        config.workspace_dir.clone(),
    );
    let auto_approved = turn_origin::with_origin(
        AgentTurnOrigin::WebChat {
            thread_id: "approval-auto-thread".to_string(),
            client_id: "approval-auto-client".to_string(),
            request_id: None,
        },
        gate.intercept_audited(
            &Default::default(),
            "tools.composio_execute",
            "tools.composio_execute(action=execute)",
            json!({ "action": "execute" }),
        ),
    )
    .await;
    assert!(matches!(
        auto_approved.0,
        openhuman_core::security::approval::GateOutcome::Allow
    ));
    assert_eq!(
        auto_approved.1, None,
        "always-allowed tools should bypass persisted approvals"
    );

    live_policy::install(
        Arc::new(SecurityPolicy {
            workspace_dir: config.workspace_dir.clone(),
            auto_approve: vec!["tools.live_policy_allowed".to_string()],
            ..SecurityPolicy::default()
        }),
        config.workspace_dir.clone(),
        config.workspace_dir.clone(),
    );
    let live_policy_auto_approved = gate
        .intercept_audited(
            &ApprovalTurnContext::chat(ApprovalChatContext {
                thread_id: "approval-live-policy-thread".to_string(),
                client_id: "approval-live-policy-client".to_string(),
                // No turn in scope in this fixture; the field is documented as
                // carried only when the caller has one (`gate.rs:91-95`).
                request_id: None,
            }),
            "tools.live_policy_allowed",
            "tools.live_policy_allowed(action=coverage)",
            json!({ "action": "coverage" }),
        )
        .await;
    assert!(matches!(live_policy_auto_approved.0, GateOutcome::Allow));
    assert_eq!(live_policy_auto_approved.1, None);
    assert!(gate
        .pending_for_thread("approval-live-policy-thread")
        .is_none());

    let gate_for_deny_task = gate.clone();
    let deny_task = tokio::spawn(async move {
        turn_origin::with_origin(
            AgentTurnOrigin::WebChat {
                thread_id: "approval-deny-thread".to_string(),
                client_id: "approval-deny-client".to_string(),
                request_id: None,
            },
            async move {
                let turn = ApprovalTurnContext::chat(ApprovalChatContext {
                    thread_id: "approval-deny-thread".to_string(),
                    client_id: "approval-deny-client".to_string(),
                    // No turn in scope in this fixture; the field is documented as
                    // carried only when the caller has one (`gate.rs:91-95`).
                    request_id: None,
                });
                gate_for_deny_task
                    .intercept_audited(
                        &turn,
                        "tools.web_search",
                        "tools.web_search(query=deny)",
                        json!({ "query": "deny" }),
                    )
                    .await
            },
        )
        .await
    });

    let deny_request_id = loop {
        let pending = rpc(
            &harness.rpc_base,
            25,
            "openhuman.approval_list_pending",
            json!({}),
        )
        .await;
        let rows = payload(&pending, "approval_list_pending deny")
            .as_array()
            .expect("pending rows for deny");
        if let Some(row) = rows
            .iter()
            .find(|row| row.get("tool_name").and_then(Value::as_str) == Some("tools.web_search"))
        {
            break row
                .get("request_id")
                .and_then(Value::as_str)
                .expect("deny request id")
                .to_string();
        }
        assert!(
            Instant::now() < deadline,
            "pending deny approval did not appear"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    };

    let deny = rpc(
        &harness.rpc_base,
        26,
        "openhuman.approval_decide",
        json!({ "request_id": deny_request_id, "decision": "deny" }),
    )
    .await;
    assert_eq!(
        payload(&deny, "approval_decide deny")
            .get("request_id")
            .and_then(Value::as_str),
        Some(deny_request_id.as_str())
    );
    let (deny_outcome, deny_approved_id) = deny_task.await.expect("deny task");
    match deny_outcome {
        openhuman_core::security::approval::GateOutcome::Deny { reason } => {
            assert!(reason.contains("User denied"));
        }
        other => panic!("expected deny outcome, got {other:?}"),
    }
    assert_eq!(deny_approved_id, None);
    assert!(gate.pending_for_thread("approval-deny-thread").is_none());
    assert!(gate.session_id().starts_with("session-"));

    let second_init = ApprovalGate::init_global(Config::default(), "session-ignored-second");
    assert_eq!(second_init.session_id(), gate.session_id());

    // Approvals are documents in the workspace's default storage database
    // (`sqlite:<workspace>/storage`, 5995128210). Drop the cached handle and
    // block that directory, so the store's next open fails.
    openhuman_core::storage::local::forget(&config.workspace_dir);
    let storage_dir = config.workspace_dir.join("storage");
    if storage_dir.exists() {
        std::fs::remove_dir_all(&storage_dir).expect("remove storage dir before failure branch");
    }
    std::fs::write(&storage_dir, "not a directory").expect("replace storage dir with file");

    gate.record_execution(
        &request_id,
        ExecutionOutcome::Success,
        Some("store path is blocked"),
    );

    let list_failure = rpc(
        &harness.rpc_base,
        27,
        "openhuman.approval_list_pending",
        json!({}),
    )
    .await;
    assert!(list_failure.get("error").is_some());

    let recent_failure = rpc(
        &harness.rpc_base,
        28,
        "openhuman.approval_list_recent_decisions",
        json!({}),
    )
    .await;
    assert!(recent_failure.get("error").is_some());

    let decide_failure = rpc(
        &harness.rpc_base,
        29,
        "openhuman.approval_decide",
        json!({ "request_id": "blocked-store", "decision": "deny" }),
    )
    .await;
    assert!(decide_failure.get("error").is_some());

    let persist_failure = turn_origin::with_origin(
        AgentTurnOrigin::WebChat {
            thread_id: "approval-persist-failure-thread".to_string(),
            client_id: "approval-persist-failure-client".to_string(),
            request_id: None,
        },
        gate.intercept_audited(
            &ApprovalTurnContext::chat(ApprovalChatContext {
                thread_id: "approval-persist-failure-thread".to_string(),
                client_id: "approval-persist-failure-client".to_string(),
                // No turn in scope in this fixture; the field is documented as
                // carried only when the caller has one (`gate.rs:91-95`).
                request_id: None,
            }),
            "tools.persistence_failure",
            "tools.persistence_failure(action=coverage)",
            json!({ "action": "coverage" }),
        ),
    )
    .await;
    match persist_failure.0 {
        GateOutcome::Deny { reason } => {
            assert!(reason.contains("Approval gate could not persist the request"));
        }
        other => panic!("expected persistence failure deny, got {other:?}"),
    }
    assert_eq!(persist_failure.1, None);
    assert!(gate
        .pending_for_thread("approval-persist-failure-thread")
        .is_none());

    harness.rpc_join.abort();
}
