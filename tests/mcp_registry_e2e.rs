//! Native-module integration tests for the MCP registry host adapter.
//!
//! These tests use TinyBus' normal loader with the verified TinyMCP artifact
//! staged on `OPENHUMAN_MODULE_PATH`; no implementation crate is linked into
//! this host test. These cases avoid launching untrusted stdio servers because
//! Linux TinyBox does not yet provide the required isolated networking.

#![cfg(feature = "mcp")]

#[path = "support/env_guard.rs"]
mod env_guard;

use openhuman_core::config::{schema::ModuleOverride, Config};
use serde_json::{json, Map, Value};
static STORAGE_SETUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static TEST_MASTER_KEY: std::sync::Once = std::sync::Once::new();

async fn fresh_workspace_config() -> (tempfile::TempDir, Config) {
    TEST_MASTER_KEY.call_once(|| {
        // This target owns a process-wide module callback and storage slot.
        // Give the module host the same configured secret key as production.
        std::env::set_var("OPENHUMAN_KEYRING_MASTER_KEY", "55".repeat(32));
        std::env::remove_var("OPENHUMAN_KEYRING_MASTER_KEY_FILE");
    });
    env_guard::init_keyring_workspace();
    let tmp = tempfile::tempdir().expect("tempdir");
    let storage_url = format!("sqlite:{}", tmp.path().join("storage").display());
    let backend = openhuman_core::storage::open(&storage_url)
        .await
        .expect("open isolated module storage backend");
    openhuman_core::storage::install(backend);
    let artifact = openhuman_core::modules::local_artifact("tinymcp")
        .expect("build tinymcp with scripts/modules/build.mjs and set OPENHUMAN_MODULE_SOURCE_DIR");
    let artifact = artifact
        .canonicalize()
        .expect("TinyMCP artifact path must exist");
    let cfg = Config {
        workspace_dir: tmp.path().to_path_buf(),
        modules: openhuman_core::config::schema::ModulesConfig {
            enabled: true,
            allow_download: false,
            overrides: vec![ModuleOverride::with_path(
                "tinymcp",
                artifact.to_string_lossy().into_owned(),
            )],
            ..Default::default()
        },
        ..Config::default()
    };
    (tmp, cfg)
}

async fn install_server(config: &Config, name: &str, server: Value) -> String {
    let mut servers = Map::new();
    servers.insert(name.to_owned(), server);
    let document = json!({
        "mcpServers": servers
    });
    let applied =
        openhuman_core::mcp::registry::config_ops::mcp_clients_config_set(config, document).await;
    if let Err(error) = &applied {
        let module_status = openhuman_core::modules::ops::list(config)
            .into_iter()
            .find(|module| module.id == "tinymcp");
        panic!("apply mcp.json through the module: {error}; status: {module_status:?}");
    }
    applied.expect("apply mcp.json through the module");
    let status = openhuman_core::mcp::registry::ops::mcp_clients_status(config)
        .await
        .expect("list the module-owned installs");
    status.value["servers"]
        .as_array()
        .and_then(|servers| servers.iter().find(|entry| entry["qualified_name"] == name))
        .and_then(|entry| entry["server_id"].as_str())
        .map(str::to_owned)
        .expect("applied server appears in module status")
}

fn stdio_server() -> Value {
    json!({
        "command": env!("CARGO_BIN_EXE_test-mcp-stub"),
        "args": [],
        "enabled": false
    })
}

fn http_server(url: &str) -> Value {
    json!({ "url": url, "enabled": false })
}

#[tokio::test]
async fn disabled_server_accepts_environment_update_without_reconnecting() {
    use std::collections::HashMap;

    let _storage_lock = STORAGE_SETUP_LOCK.lock().await;
    let (_tmp, config) = fresh_workspace_config().await;
    let server_id = install_server(&config, "@openhuman-test/echo", stdio_server()).await;
    let mut env = HashMap::new();
    env.insert("API_KEY".to_owned(), "test-only-secret".to_owned());

    let result =
        openhuman_core::mcp::registry::ops::mcp_clients_update_env(&config, server_id.clone(), env)
            .await
            .expect("confidential disabled-server update succeeds");
    assert_eq!(result.value["status"], "disabled");
    assert_eq!(result.value["env_keys"], json!(["API_KEY"]));
    assert!(
        !openhuman_core::mcp::registry::connections::is_connected_for_config(&config, &server_id)
            .await
    );
}

/// One workspace only. Un-tenanted workspaces in one process share TinyMCP's
/// installs by design (the host binds module storage per tenant profile, not
/// per workspace), so a second workspace here would boot-connect this server
/// and record its own 401. Per-tenant isolation is covered by
/// `modules::storage_host` `module_scopes_are_distinct_per_module_and_per_tenant`
/// and `a_module_scope_is_the_explicit_contexts_profile_across_tasks`.
#[tokio::test]
async fn auth_failure_surfaces_401_reason_as_a_typed_hint() {
    use openhuman_core::mcp::registry::{connections, ops};
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let _storage_lock = STORAGE_SETUP_LOCK.lock().await;
    let upstream = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(401))
        .mount(&upstream)
        .await;

    let (_tmp, config) = fresh_workspace_config().await;
    let server_id = install_server(
        &config,
        "@openhuman-test/remote",
        http_server(&upstream.uri()),
    )
    .await;

    ops::mcp_clients_set_enabled(&config, server_id.clone(), true)
        .await
        .expect("enable remote server before exercising its 401 response");

    assert!(
        ops::mcp_clients_connect(&config, server_id.clone())
            .await
            .is_err(),
        "the module must report a 401 as a failed connection"
    );
    assert_eq!(
        connections::auth_hint_for_config(&config, &server_id).await,
        Some("credential_required")
    );
    assert_eq!(
        connections::last_error_for_config(&config, &server_id).await,
        None,
        "401 response text stays hidden; the typed auth hint carries the user-facing reason"
    );
}
