//! Raw-line oriented E2E coverage for the tool_registry and approval domains.
//!
//! These tests intentionally mix JSON-RPC calls with the public domain APIs that
//! back those calls. JSON-RPC drives the externally visible controller paths;
//! direct public API calls cover persistence/redaction/provider branches that
//! are otherwise only indirectly reachable from the controllers.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::http::header::AUTHORIZATION;
use reqwest::StatusCode;
use rusqlite::{params, Connection};
use serde_json::{json, Map, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::approval::gate::{
    parse_approval_reply, ApprovalChatContext, ApprovalGate, APPROVAL_CHAT_CONTEXT,
};
use openhuman_core::openhuman::approval::store as approval_store;
use openhuman_core::openhuman::approval::{
    all_approval_controller_schemas, all_approval_registered_controllers, redact_args,
    summarize_action, ApprovalDecision, ExecutionOutcome, GateOutcome, PendingApproval,
};
use openhuman_core::openhuman::config::schema::{
    CapabilityProviderConfig, CapabilityProviderTrustState,
};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::mcp_registry::connections;
use openhuman_core::openhuman::mcp_registry::types::{CommandKind, InstalledServer, Transport};
use openhuman_core::openhuman::security::{live_policy, SecurityPolicy};
use openhuman_core::openhuman::tool_registry::{
    all_tool_registry_controller_schemas, all_tool_registry_registered_controllers,
    capability_provider_by_id, capability_provider_diagnostics, capability_provider_registry,
    denials, get_tool, is_capability_provider_trusted_enabled, list_capability_providers,
    list_tools, normalize_capability_provider_id, registry_entries,
    CapabilityProviderRegistryError,
};

const TEST_RPC_TOKEN: &str = "tool-registry-approval-raw-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
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

struct TestHarness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    rpc_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
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
        let token_dir = std::env::temp_dir().join("openhuman-tool-registry-approval-e2e-auth");
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
    assert_eq!(response.status(), StatusCode::OK, "{method} HTTP status");
    response
        .json::<Value>()
        .await
        .unwrap_or_else(|err| panic!("json for {method}: {err}"))
}

fn ok<'a>(value: &'a Value, context: &str) -> &'a Value {
    if let Some(error) = value.get("error") {
        panic!("{context}: unexpected JSON-RPC error: {error}");
    }
    value
        .get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {value}"))
}

fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    let result = ok(value, context);
    result.get("result").unwrap_or(result)
}

fn error_message<'a>(value: &'a Value, context: &str) -> &'a str {
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error missing message: {value}"))
}

fn provider(
    id: &str,
    display_name: &str,
    trust_state: CapabilityProviderTrustState,
    enabled: bool,
) -> CapabilityProviderConfig {
    CapabilityProviderConfig {
        id: id.to_string(),
        display_name: display_name.to_string(),
        source_uri: Some(format!(" https://example.com/providers/{id} ")),
        source_digest: Some(" sha256:feedface ".to_string()),
        trust_state,
        enabled,
    }
}

fn approval_db_path(config: &Config) -> PathBuf {
    config.workspace_dir.join("approval").join("approval.db")
}

fn pending(
    request_id: &str,
    _session_id: &str,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
) -> PendingApproval {
    PendingApproval::new(
        request_id,
        "tools.composio_execute",
        "tools.composio_execute(action=execute, 42 bytes)",
        json!({ "action": "execute", "tool_slug": "GMAIL_SEND_EMAIL" }),
        expires_at,
    )
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
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_registry_rpc_diagnostics_include_denials_and_provider_errors() {
    let _lock = env_lock();
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

#[test]
fn tool_registry_public_api_lists_gets_and_validates_ids() {
    let listed = list_tools()
        .into_cli_compatible_json()
        .expect("list_tools json");
    let tools = listed
        .get("tools")
        .and_then(Value::as_array)
        .expect("listed tools");
    let first_tool_id = tools
        .first()
        .and_then(|tool| tool.get("tool_id"))
        .and_then(Value::as_str)
        .expect("first tool id");

    let found = get_tool(first_tool_id)
        .expect("get first tool")
        .into_cli_compatible_json()
        .expect("get_tool json");
    assert_eq!(
        found.get("tool_id").and_then(Value::as_str),
        Some(first_tool_id)
    );
    assert!(get_tool("   ")
        .expect_err("blank id should fail")
        .contains("non-empty"));
    assert!(get_tool("missing.tool")
        .expect_err("missing id should fail")
        .contains("missing.tool"));
}

#[test]
fn capability_provider_public_api_normalizes_lookup_and_error_branches() {
    let config = Config {
        capability_providers: vec![
            provider(
                " Team Tools ",
                "  ",
                CapabilityProviderTrustState::Trusted,
                true,
            ),
            provider(
                "draft_tools",
                "Draft Tools",
                CapabilityProviderTrustState::Untrusted,
                true,
            ),
            provider(
                "disabled.tools",
                "Disabled Tools",
                CapabilityProviderTrustState::Trusted,
                false,
            ),
        ],
        ..Config::default()
    };

    assert_eq!(
        normalize_capability_provider_id(" Team Tools "),
        Ok("team-tools".to_string())
    );
    assert!(normalize_capability_provider_id("!!!").is_err());
    assert!(normalize_capability_provider_id(&"x".repeat(120)).is_err());

    let registry = capability_provider_registry(&config).expect("provider registry");
    let listed = registry.list();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[2].id, "team-tools");
    assert_eq!(
        listed[2].display_name, "team-tools",
        "empty display_name should fall back to normalized id"
    );
    assert_eq!(
        listed[2].source_uri.as_deref(),
        Some("https://example.com/providers/ Team Tools")
    );
    assert!(registry.get("TEAM TOOLS").is_some());
    assert!(registry.get("!!!").is_none());
    assert!(registry.is_trusted_enabled("team tools"));
    assert!(!registry.is_trusted_enabled("draft_tools"));
    assert!(!registry.is_trusted_enabled("disabled.tools"));

    assert_eq!(list_capability_providers(&config).unwrap().len(), 3);
    let diagnostics = capability_provider_diagnostics(&config);
    assert_eq!(diagnostics.total_providers, 3);
    assert_eq!(diagnostics.enabled_providers, 2);
    assert_eq!(diagnostics.trusted_providers, 2);
    assert_eq!(diagnostics.trusted_enabled_providers, 1);
    assert!(diagnostics.registry_errors.is_empty());
    assert_eq!(
        capability_provider_by_id(&config, "team tools")
            .unwrap()
            .expect("team provider")
            .id,
        "team-tools"
    );
    assert!(is_capability_provider_trusted_enabled(
        &config,
        "team tools"
    ));

    let duplicate_config = Config {
        capability_providers: vec![
            provider(
                "Team Tools",
                "Team Tools",
                CapabilityProviderTrustState::Trusted,
                true,
            ),
            provider(
                "team-tools",
                "Team Tools",
                CapabilityProviderTrustState::Trusted,
                true,
            ),
        ],
        ..Config::default()
    };
    assert!(list_capability_providers(&duplicate_config).is_err());
    let diagnostics = capability_provider_diagnostics(&duplicate_config);
    assert_eq!(diagnostics.total_providers, 2);
    assert!(diagnostics.registry_errors[0].contains("duplicate"));

    let invalid_config = Config {
        capability_providers: vec![provider(
            "!!!",
            "Invalid Tools",
            CapabilityProviderTrustState::Trusted,
            true,
        )],
        ..Config::default()
    };
    let invalid_err = list_capability_providers(&invalid_config).expect_err("invalid provider id");
    assert_eq!(
        invalid_err.to_string(),
        CapabilityProviderRegistryError::InvalidId {
            raw: "!!!".to_string()
        }
        .to_string()
    );
    assert!(!is_capability_provider_trusted_enabled(
        &invalid_config,
        "invalid"
    ));
    let invalid_diagnostics = capability_provider_diagnostics(&invalid_config);
    assert_eq!(invalid_diagnostics.total_providers, 1);
    assert_eq!(invalid_diagnostics.enabled_providers, 0);
    assert!(invalid_diagnostics.registry_errors[0].contains("invalid provider id"));
}

#[test]
fn tool_registry_diagnostics_for_config_reports_audit_success_and_policy_shape() {
    let dir = tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };

    let diagnostics =
        openhuman_core::openhuman::tool_registry::ops::diagnostics_for_config(&config)
            .into_cli_compatible_json()
            .expect("diagnostics json");
    assert!(diagnostics
        .get("total_tools")
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 0));
    assert_eq!(
        diagnostics.pointer("/mcp_write_audit/enabled"),
        Some(&json!(true))
    );
    assert_eq!(
        diagnostics.pointer("/mcp_write_audit/last_error"),
        Some(&Value::Null)
    );
    assert!(diagnostics
        .pointer("/mcp_write_audit/recent_rows")
        .and_then(Value::as_u64)
        .is_some());
    assert_eq!(
        diagnostics.pointer("/posture/autonomy_level"),
        Some(&json!("supervised"))
    );
    assert!(diagnostics
        .pointer("/policy_surfaces")
        .and_then(Value::as_array)
        .expect("policy surfaces")
        .iter()
        .any(|surface| surface.as_str() == Some("tool_registry.diagnostics")));
    assert_eq!(
        diagnostics.pointer("/mcp_allowlists/server_count"),
        Some(&json!(0))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn tool_registry_entries_fall_back_on_current_thread_runtime() {
    let entries = registry_entries();
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
    let server = test_mcp_server();
    let tools = connections::connect(&config, &server)
        .await
        .expect("connect test mcp server");
    assert_eq!(tools.first().map(|tool| tool.name.as_str()), Some("echo"));

    let entries = registry_entries();
    let client_entry = entries
        .iter()
        .find(|entry| entry.tool_id == format!("mcp-client::{}::echo", server.server_id))
        .expect("connected mcp client entry");
    assert_eq!(client_entry.name, "echo");
    assert_eq!(client_entry.route["protocol"], json!("mcp-client"));
    assert_eq!(client_entry.route["server_id"], json!(server.server_id));
    assert!(client_entry.tags.iter().any(|tag| tag == "mcp_client"));

    assert!(connections::disconnect(&server.server_id).await);
}

#[tokio::test]
async fn tool_registry_schema_handlers_validate_and_return_payloads() {
    let schemas = all_tool_registry_controller_schemas();
    assert_eq!(
        schemas
            .iter()
            .map(|schema| schema.function)
            .collect::<Vec<_>>(),
        vec!["list", "get", "diagnostics"]
    );
    let controllers = all_tool_registry_registered_controllers();
    assert_eq!(controllers.len(), schemas.len());

    let list_handler = controllers
        .iter()
        .find(|controller| controller.schema.function == "list")
        .expect("list controller")
        .handler;
    let list_value = list_handler(Map::new()).await.expect("list handler");
    let tools = list_value
        .get("tools")
        .and_then(Value::as_array)
        .expect("tools array");
    assert!(tools
        .iter()
        .any(|tool| tool.get("tool_id").and_then(Value::as_str) == Some("memory.search")));

    let get_handler = controllers
        .iter()
        .find(|controller| controller.schema.function == "get")
        .expect("get controller")
        .handler;
    assert!(get_handler(Map::new())
        .await
        .expect_err("missing tool_id")
        .contains("non-empty string"));
    let mut numeric_tool_id = Map::new();
    numeric_tool_id.insert("tool_id".to_string(), json!(42));
    assert!(get_handler(numeric_tool_id)
        .await
        .expect_err("numeric tool_id")
        .contains("non-empty string"));
    let mut blank_tool_id = Map::new();
    blank_tool_id.insert("tool_id".to_string(), json!("   "));
    assert!(get_handler(blank_tool_id)
        .await
        .expect_err("blank tool_id")
        .contains("non-empty string"));
    let mut valid_tool_id = Map::new();
    valid_tool_id.insert("tool_id".to_string(), json!("tools.web_search"));
    let tool_value = get_handler(valid_tool_id).await.expect("get handler");
    assert_eq!(
        tool_value.get("tool_id").and_then(Value::as_str),
        Some("tools.web_search")
    );

    let diagnostics_handler = controllers
        .iter()
        .find(|controller| controller.schema.function == "diagnostics")
        .expect("diagnostics controller")
        .handler;
    let diagnostics_value = diagnostics_handler(Map::new())
        .await
        .expect("diagnostics handler");
    assert!(diagnostics_value
        .get("total_tools")
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 0));
}

#[tokio::test]
async fn tool_registry_diagnostics_reports_config_and_audit_store_failures() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let workspace_file = tmp.path().join("workspace-file");
    std::fs::write(&workspace_file, "not a directory").expect("workspace sentinel");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace_file);

    let err = openhuman_core::openhuman::tool_registry::ops::diagnostics()
        .await
        .expect_err("workspace file should prevent config load");
    assert!(err.contains("failed to load config for tool registry diagnostics"));

    let broken_audit_config = Config {
        workspace_dir: workspace_file,
        ..Config::default()
    };
    let diagnostics =
        openhuman_core::openhuman::tool_registry::ops::diagnostics_for_config(&broken_audit_config);
    assert!(diagnostics.value.mcp_write_audit.enabled);
    assert_eq!(diagnostics.value.mcp_write_audit.recent_rows, None);
    assert!(diagnostics
        .value
        .mcp_write_audit
        .last_error
        .as_deref()
        .is_some_and(|error| !error.is_empty()));
}

#[test]
fn approval_redaction_and_store_cover_shape_expiry_migration_and_audit_branches() {
    let dir = tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };

    let raw_args = json!({
        "action": "execute",
        "tool_slug": "GMAIL_SEND_EMAIL",
        "integration": "gmail",
        "body": "Hello from /Users/alice/private.txt",
        "recipients": ["a@example.com", "b@example.com"],
        "metadata": {
            "Subject": "Confidential subject",
            "token": "sk-secret",
            "auth": true,
            "message": 42,
            "password": null,
            "user": { "id": "user-123", "name": "Alice" },
            "attempts": 3,
            "confirmed": true,
            "nullable": null,
            "safe_path": "C:\\Users\\bob\\Desktop\\report.txt",
            "safe_list": [
                "open /Users/frank/Desktop/report.txt",
                { "content": "nested secret" }
            ]
        }
    });
    let redacted = redact_args(&raw_args);
    assert_eq!(redacted["action"], json!("execute"));
    assert_eq!(redacted["body"], json!("<redacted: string (35 chars)>"));
    assert_eq!(redacted["recipients"], json!("<redacted: array (2 items)>"));
    assert_eq!(
        redacted.pointer("/metadata/Subject"),
        Some(&json!("<redacted: string (20 chars)>"))
    );
    assert_eq!(
        redacted.pointer("/metadata/attempts"),
        Some(&json!(3)),
        "non-sensitive numeric fields should pass through"
    );
    assert_eq!(
        redacted.pointer("/metadata/auth"),
        Some(&json!("<redacted: bool>"))
    );
    assert_eq!(
        redacted.pointer("/metadata/message"),
        Some(&json!("<redacted: number>"))
    );
    assert_eq!(redacted.pointer("/metadata/password"), Some(&Value::Null));
    assert_eq!(
        redacted.pointer("/metadata/user"),
        Some(&json!("<redacted: object (2 keys)>"))
    );
    assert_eq!(
        redacted.pointer("/metadata/safe_list/0"),
        Some(&json!("open <HOME>/Desktop/report.txt"))
    );
    assert_eq!(
        redacted.pointer("/metadata/safe_list/1/content"),
        Some(&json!("<redacted: string (13 chars)>"))
    );
    assert_eq!(
        redact_args(&json!(
            "open /home/carol/report.md and C:\\Users\\dave\\x.txt"
        )),
        json!("open <HOME>/report.md and <HOME>\\x.txt")
    );
    assert_eq!(redact_args(&json!("/Users/erin")), json!("<HOME>"));
    let summary = summarize_action("tools.composio_execute", &raw_args);
    assert!(summary.contains("action=execute"));
    assert!(summary.contains("tool_slug=GMAIL_SEND_EMAIL"));
    assert!(summary.contains("integration=gmail"));
    let summary_without_safe_fields = summarize_action("tools.empty", &json!(["opaque"]));
    assert!(summary_without_safe_fields.starts_with("tools.empty ("));

    approval_store::insert_pending(
        &config,
        &pending(
            "expired",
            "session-a",
            Some(chrono::Utc::now() - chrono::Duration::minutes(5)),
        ),
        "session-a",
    )
    .expect("insert expired");
    approval_store::insert_pending(
        &config,
        &pending(
            "active",
            "session-a",
            Some(chrono::Utc::now() + chrono::Duration::minutes(5)),
        ),
        "session-a",
    )
    .expect("insert active");
    approval_store::insert_pending(
        &config,
        &pending("other-session", "session-b", None),
        "session-b",
    )
    .expect("insert no-ttl");

    let rows = approval_store::list_pending(&config).expect("list pending");
    let ids = rows
        .iter()
        .map(|row| row.request_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["active", "other-session"]);
    assert_eq!(
        approval_store::get_decision(&config, "expired").expect("expired decision"),
        Some(ApprovalDecision::Deny)
    );

    let removed = approval_store::purge_session(&config, "session-b").expect("purge session");
    assert_eq!(removed, 1);
    assert_eq!(
        approval_store::purge_session(&config, "missing-session").unwrap(),
        0
    );

    let decided = approval_store::decide(&config, "active", ApprovalDecision::ApproveOnce)
        .expect("decide active")
        .expect("active row");
    assert_eq!(decided.request_id, "active");
    assert!(!approval_store::record_execution(
        &config,
        "missing",
        ExecutionOutcome::Aborted,
        Some("not found"),
    )
    .expect("unknown record execution"));
    assert!(approval_store::record_execution(
        &config,
        "active",
        ExecutionOutcome::Failure,
        Some("upstream Authorization: Bearer sk-live-abcdefghijklmnopqrstuvwxyz failed"),
    )
    .expect("record failed execution"));
    assert!(!approval_store::record_execution(
        &config,
        "active",
        ExecutionOutcome::Success,
        Some("late rewrite"),
    )
    .expect("idempotent execution"));

    let audit = approval_store::list_recent_decisions(&config, 0).expect("recent decisions");
    assert_eq!(audit.len(), 1, "zero limit should clamp to one");
    assert_eq!(audit[0].request_id, "active");
    assert_eq!(audit[0].decision, ApprovalDecision::ApproveOnce);

    let db_path = approval_db_path(&config);
    let conn = Connection::open(&db_path).expect("open approval db");
    conn.execute(
        "INSERT INTO pending_approvals
            (request_id, tool_name, action_summary, args_redacted, session_id, created_at,
             decided_at, decision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            "corrupt-json",
            "tools.web_search",
            "corrupt args",
            "{not valid json",
            "session-a",
            chrono::Utc::now().to_rfc3339(),
            chrono::Utc::now().to_rfc3339(),
            "deny",
        ],
    )
    .expect("insert corrupt audit row");
    drop(conn);
    let audit = approval_store::list_recent_decisions(&config, 10).expect("audit with corrupt row");
    let corrupt = audit
        .iter()
        .find(|row| row.request_id == "corrupt-json")
        .expect("corrupt audit row");
    assert_eq!(
        corrupt.args_redacted,
        json!({ "_error": "args_redacted not valid JSON" })
    );

    let legacy_dir = tempdir().expect("legacy tempdir");
    let legacy_config = Config {
        workspace_dir: legacy_dir.path().to_path_buf(),
        ..Config::default()
    };
    let legacy_db = approval_db_path(&legacy_config);
    std::fs::create_dir_all(legacy_db.parent().expect("legacy db parent"))
        .expect("create legacy db dir");
    let legacy_conn = Connection::open(&legacy_db).expect("open legacy db");
    legacy_conn
        .execute_batch(
            "CREATE TABLE pending_approvals (
                request_id      TEXT PRIMARY KEY,
                tool_name       TEXT NOT NULL,
                action_summary  TEXT NOT NULL,
                args_redacted   TEXT NOT NULL,
                session_id      TEXT NOT NULL,
                created_at      TEXT NOT NULL,
                expires_at      TEXT,
                decided_at      TEXT,
                decision        TEXT
            );",
        )
        .expect("create legacy schema");
    legacy_conn
        .execute(
            "INSERT INTO pending_approvals
                (request_id, tool_name, action_summary, args_redacted, session_id, created_at)
             VALUES ('legacy', 'tools.web_search', 'legacy', '{}', 'legacy-session', ?1)",
            params![chrono::Utc::now().to_rfc3339()],
        )
        .expect("insert legacy row");
    drop(legacy_conn);

    assert_eq!(
        approval_store::list_pending(&legacy_config)
            .expect("migrated pending")
            .len(),
        1
    );
    approval_store::decide(&legacy_config, "legacy", ApprovalDecision::ApproveOnce)
        .expect("decide legacy");
    assert!(approval_store::record_execution(
        &legacy_config,
        "legacy",
        ExecutionOutcome::Success,
        None,
    )
    .expect("record execution after migration"));
}

#[test]
fn approval_reply_parser_accepts_explicit_yes_no_only() {
    for decision in [
        ApprovalDecision::ApproveOnce,
        ApprovalDecision::ApproveAlwaysForTool,
        ApprovalDecision::Deny,
    ] {
        assert_eq!(
            ApprovalDecision::from_str(decision.as_str()),
            Some(decision)
        );
    }
    assert_eq!(ApprovalDecision::from_str("maybe"), None);
    assert!(ApprovalDecision::ApproveOnce.is_approve());
    assert!(ApprovalDecision::ApproveAlwaysForTool.is_approve());
    assert!(!ApprovalDecision::Deny.is_approve());

    for outcome in [
        ExecutionOutcome::Success,
        ExecutionOutcome::Failure,
        ExecutionOutcome::Aborted,
    ] {
        assert_eq!(ExecutionOutcome::from_str(outcome.as_str()), Some(outcome));
    }
    assert_eq!(ExecutionOutcome::from_str("partial"), None);
    assert_eq!(
        serde_json::to_string(&ExecutionOutcome::Aborted).expect("serialize outcome"),
        "\"aborted\""
    );

    assert_eq!(
        parse_approval_reply(" yes "),
        Some(ApprovalDecision::ApproveOnce)
    );
    assert_eq!(
        parse_approval_reply("APPROVED"),
        Some(ApprovalDecision::ApproveOnce)
    );
    assert_eq!(parse_approval_reply("n"), Some(ApprovalDecision::Deny));
    assert_eq!(parse_approval_reply("denied"), Some(ApprovalDecision::Deny));
    assert_eq!(parse_approval_reply("maybe later"), None);
}

#[tokio::test]
async fn approval_schema_handlers_validate_params_and_surface_empty_gate_state() {
    let schemas = all_approval_controller_schemas();
    assert_eq!(
        schemas
            .iter()
            .map(|schema| schema.function)
            .collect::<Vec<_>>(),
        vec!["list_pending", "list_recent_decisions", "decide"]
    );
    let unknown = openhuman_core::openhuman::approval::schemas::schemas("missing");
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
    let list_value = list_handler(Map::new()).await.expect("list pending value");
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
    assert!(recent_handler(invalid_limit)
        .await
        .expect_err("string limit")
        .contains("expected unsigned integer"));
    for invalid in [json!(true), json!([]), json!({ "limit": 10 })] {
        let mut invalid_limit = Map::new();
        invalid_limit.insert("limit".to_string(), invalid);
        assert!(recent_handler(invalid_limit)
            .await
            .expect_err("non-numeric limit")
            .contains("expected unsigned integer"));
    }
    let mut negative_limit = Map::new();
    negative_limit.insert("limit".to_string(), json!(-1));
    assert!(recent_handler(negative_limit)
        .await
        .expect_err("negative limit")
        .contains("expected unsigned integer"));
    let mut null_limit = Map::new();
    null_limit.insert("limit".to_string(), Value::Null);
    let recent_value = recent_handler(null_limit)
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
    assert!(decide_handler(Map::new())
        .await
        .expect_err("missing request id")
        .contains("missing required param 'request_id'"));
    let mut numeric_request = Map::new();
    numeric_request.insert("request_id".to_string(), json!(42));
    numeric_request.insert("decision".to_string(), json!("deny"));
    assert!(decide_handler(numeric_request)
        .await
        .expect_err("numeric request id")
        .contains("expected string"));
    for invalid in [
        Value::Null,
        json!(false),
        json!([]),
        json!({ "id": "missing" }),
    ] {
        let mut invalid_request = Map::new();
        invalid_request.insert("request_id".to_string(), invalid);
        invalid_request.insert("decision".to_string(), json!("deny"));
        assert!(decide_handler(invalid_request)
            .await
            .expect_err("non-string request id")
            .contains("expected string"));
    }
    let mut numeric_decision = Map::new();
    numeric_decision.insert("request_id".to_string(), json!("missing"));
    numeric_decision.insert("decision".to_string(), json!(42));
    assert!(decide_handler(numeric_decision)
        .await
        .expect_err("numeric decision")
        .contains("expected string"));
    let mut invalid_decision = Map::new();
    invalid_decision.insert("request_id".to_string(), json!("missing"));
    invalid_decision.insert("decision".to_string(), json!("maybe"));
    assert!(decide_handler(invalid_decision)
        .await
        .expect_err("invalid decision")
        .contains("approve_once|approve_always_for_tool|deny"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_rpc_decision_paths_persist_always_allow_and_recent_audit() {
    let _lock = env_lock();
    let harness = setup("").await;
    let config = Config::load_or_init()
        .await
        .expect("load config for approval gate");
    let gate = ApprovalGate::init_global(config.clone(), "approval-raw-e2e-session");
    let gate_for_task = gate.clone();

    let approval_task = tokio::spawn(async move {
        APPROVAL_CHAT_CONTEXT
            .scope(
                ApprovalChatContext {
                    thread_id: "approval-raw-thread".to_string(),
                    client_id: "approval-raw-client".to_string(),
                },
                async move {
                    gate_for_task
                        .intercept_audited(
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
        openhuman_core::openhuman::approval::GateOutcome::Allow
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

    let no_chat = gate
        .intercept_audited(
            "tools.web_search",
            "tools.web_search(query=coverage)",
            json!({ "query": "coverage" }),
        )
        .await;
    assert!(matches!(
        no_chat.0,
        openhuman_core::openhuman::approval::GateOutcome::Allow
    ));
    assert_eq!(
        no_chat.1, None,
        "non-chat calls should not create approval rows"
    );
    assert!(matches!(
        gate.intercept(
            "tools.web_search",
            "tools.web_search(query=legacy)",
            json!({ "query": "legacy" }),
        )
        .await,
        GateOutcome::Allow
    ));

    let auto_approved = gate
        .intercept_audited(
            "tools.composio_execute",
            "tools.composio_execute(action=execute)",
            json!({ "action": "execute" }),
        )
        .await;
    assert!(matches!(
        auto_approved.0,
        openhuman_core::openhuman::approval::GateOutcome::Allow
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
    );
    let live_policy_auto_approved = APPROVAL_CHAT_CONTEXT
        .scope(
            ApprovalChatContext {
                thread_id: "approval-live-policy-thread".to_string(),
                client_id: "approval-live-policy-client".to_string(),
            },
            gate.intercept_audited(
                "tools.live_policy_allowed",
                "tools.live_policy_allowed(action=coverage)",
                json!({ "action": "coverage" }),
            ),
        )
        .await;
    assert!(matches!(live_policy_auto_approved.0, GateOutcome::Allow));
    assert_eq!(live_policy_auto_approved.1, None);
    assert!(gate
        .pending_for_thread("approval-live-policy-thread")
        .is_none());

    let gate_for_deny_task = gate.clone();
    let deny_task = tokio::spawn(async move {
        APPROVAL_CHAT_CONTEXT
            .scope(
                ApprovalChatContext {
                    thread_id: "approval-deny-thread".to_string(),
                    client_id: "approval-deny-client".to_string(),
                },
                async move {
                    gate_for_deny_task
                        .intercept_audited(
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
        openhuman_core::openhuman::approval::GateOutcome::Deny { reason } => {
            assert!(reason.contains("User denied"));
        }
        other => panic!("expected deny outcome, got {other:?}"),
    }
    assert_eq!(deny_approved_id, None);
    assert!(gate.pending_for_thread("approval-deny-thread").is_none());
    assert_eq!(gate.session_id(), "approval-raw-e2e-session");

    let second_init = ApprovalGate::init_global(Config::default(), "ignored-second-session");
    assert_eq!(second_init.session_id(), "approval-raw-e2e-session");

    let approval_dir = config.workspace_dir.join("approval");
    if approval_dir.exists() {
        std::fs::remove_dir_all(&approval_dir).expect("remove approval dir before failure branch");
    }
    std::fs::write(&approval_dir, "not a directory").expect("replace approval dir with file");

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

    let persist_failure = APPROVAL_CHAT_CONTEXT
        .scope(
            ApprovalChatContext {
                thread_id: "approval-persist-failure-thread".to_string(),
                client_id: "approval-persist-failure-client".to_string(),
            },
            gate.intercept_audited(
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
