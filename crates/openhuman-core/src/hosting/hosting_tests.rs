//! Tests for OpenHuman's hosting module adapter and its tool contract.

use super::*;
use crate::config::Config;
use std::sync::{Arc, Mutex};

fn config_with(workspace: &std::path::Path, enabled: bool, api_key: &str) -> Config {
    let mut config = Config::default();
    config.workspace_dir = workspace.to_path_buf();
    config.components.hosting = enabled;
    config.hosting.api_key = api_key.to_string();
    config
}

/// A recording adapter fixture for OpenHuman's exact member, tuple, and
/// confidentiality intent. The TinyBus transport is covered separately by
/// the generic client fixtures and the released-module verifier.
fn account_fixture(
    workspace_dir: PathBuf,
    calls: Arc<Mutex<Vec<(String, serde_json::Value, bool)>>>,
) -> Account {
    let mut config = Config::default();
    config.workspace_dir = workspace_dir;
    config.hosting.api_key = "fixture-secret".to_owned();
    let account = Account::from_config(&config)
        .expect("configuration")
        .expect("account");
    let calls_out = Arc::clone(&calls);
    let fixture = Arc::new(
        move |member: &str, args: serde_json::Value, confidential: bool| {
            calls_out
                .lock()
                .unwrap()
                .push((member.to_owned(), args.clone(), confidential));
            if member == tinyhosts_bus::METHODS[1] {
                return Ok(serde_json::json!(["vercel"]).to_string());
            }
            let request = args
                .as_array()
                .and_then(|args| args.first())
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("fixture expected one JSON argument"))?;
            let request: serde_json::Value = serde_json::from_str(request)?;
            let response = match request.get("operation").and_then(serde_json::Value::as_str) {
                Some("prepare_bundle") => serde_json::json!({"result":"prepared_bundle","value":{
                    "contract_version":[1,2],
                    "bundle":[{"path":"index.html","contents":"aGVsbG8="}],
                    "file_count":1,"total_bytes":5,"skipped_entries":0,"scanned_entries":1
                }}),
                Some("launch") => serde_json::json!({"result":"launch","value":{
                    "site":{"id":"site-1","name":"demo"},"created_site":true,
                    "database":null,"database_env_keys":[],"domains":[],
                    "deployment":{"id":"deploy-1","site":"demo","status":"building","url":"https://demo.example"}
                }}),
                Some("deployment") => serde_json::json!({"result":"deployment","value":{
                    "id":"deploy-1","site":"demo","status":
                        if request["id"] == "ready-1" { "ready" } else { "failed" }
                }}),
                Some("promote") => serde_json::json!({"result":"done"}),
                Some("set_env") => serde_json::json!({"result":"done"}),
                Some("list_sites") => serde_json::json!({"result":"sites","value":[]}),
                _ => return Err(anyhow::anyhow!("unexpected fixture operation")),
            };
            Ok(response.to_string())
        },
    );
    Account {
        call_fixture: Some(fixture),
        ..account
    }
}

#[test]
fn hosting_off_yields_no_account() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), false, "token");

    assert!(Account::from_config(&config)
        .expect("resolution does not fail")
        .is_none());
}

#[test]
fn a_configured_key_yields_an_account() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), true, "token");

    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account, since a key is configured");

    assert_eq!(account.provider, "vercel");
    assert_eq!(account.workspace_dir(), workspace.path());
}

#[test]
fn an_unknown_provider_is_an_error_rather_than_a_silent_skip() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let mut config = config_with(workspace.path(), true, "token");
    config.hosting.provider = "heroku".to_string();

    let error = Account::from_config(&config).expect_err("an unknown provider fails");

    assert!(
        error.to_string().contains("heroku"),
        "the error should name the provider: {error}"
    );
}

#[test]
fn an_account_reports_itself_without_its_credential() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), true, "super-secret");

    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account");

    assert!(
        !format!("{account:?}").contains("super-secret"),
        "the credential must never be rendered"
    );
}

#[test]
fn an_account_exposes_every_hosting_tool() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), true, "token");
    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account");

    let names: Vec<String> = account
        .tools()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();

    assert_eq!(
        names,
        [
            "hosting_launch_site",
            "hosting_deployment_status",
            "hosting_list_deployments",
            "hosting_deployment_logs",
            "hosting_rollback",
            "hosting_list_sites",
            "hosting_set_env",
            "hosting_add_domain",
            "hosting_domain_status",
            "hosting_analytics",
        ]
    );
    for tool in account.tools() {
        assert!(!tool.description().is_empty());
        assert_eq!(tool.parameters_schema()["type"], "object");
    }
}

#[test]
fn tool_declarations_match_the_released_module_contract() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let account = Account::from_config(&config_with(workspace.path(), true, "token"))
        .expect("resolution")
        .expect("account");
    let tools = account.tools();
    let declarations: Vec<tinyhosts_bus::ToolDeclaration> =
        serde_json::from_str(tinyhosts_bus::TOOL_DECLARATIONS_JSON).expect("contract json");
    assert_eq!(tools.len(), declarations.len());
    for (tool, declaration) in tools.iter().zip(declarations) {
        assert_eq!(tool.name(), declaration.name);
        assert_eq!(tool.description(), declaration.description);
        assert_eq!(tool.parameters_schema(), declaration.parameters_schema);
        assert_eq!(tool.external_effect(), declaration.external_effect);
        assert_eq!(
            tool.permission_level(),
            match declaration.permission_level {
                tinyhosts_bus::PermissionLevel::ReadOnly => tinytools::PermissionLevel::ReadOnly,
                tinyhosts_bus::PermissionLevel::Write => tinytools::PermissionLevel::Write,
            }
        );
    }
}

#[test]
fn the_module_registry_pins_the_released_hosts_artifact_set() {
    let record = crate::modules::registry::find("tinyhosts").expect("registered module");
    assert_eq!(record.version, "0.4.1");
    assert_eq!(record.assets.len(), 11);
    assert!(record.assets.iter().all(|asset| {
        asset.archive.starts_with("tinyhosts-0.4.1-") && asset.sha256.len() == 64
    }));
    assert_eq!(
        record.asset_for("ubuntu-24.04-x86_64").unwrap().sha256,
        "7e8a72b7951db71cc98c94fed0908660a58bd3f2cbf23103d953e28e1315d4d5"
    );
}

#[cfg(feature = "modules")]
#[tokio::test]
#[ignore = "requires a TinyHosts library: node scripts/modules/build.mjs tinyhosts, then OPENHUMAN_MODULE_SOURCE_DIR"]
async fn released_artifact_serves_providers_and_confidential_local_preparation() {
    use base64::Engine as _;

    let artifact = crate::modules::local_artifact("tinyhosts").expect(
        "build tinyhosts with scripts/modules/build.mjs and set OPENHUMAN_MODULE_SOURCE_DIR",
    );
    let workspace = tempfile::tempdir().expect("workspace");
    let site = workspace.path().join("site");
    std::fs::create_dir(&site).expect("site directory");
    let html = b"<!doctype html><title>module fixture</title>";
    std::fs::write(site.join("index.html"), html).expect("fixture HTML");

    let mut config = config_with(workspace.path(), true, "fixture-api-key");
    config.modules.allow_download = false;
    config
        .modules
        .overrides
        .push(crate::config::schema::ModuleOverride::with_path(
            "tinyhosts",
            artifact.to_string_lossy().into_owned(),
        ));
    let account = Account::connect_with_config("vercel", "fixture-api-key", None, config)
        .expect("account configuration");
    let workspace = workspace
        .path()
        .canonicalize()
        .expect("canonical workspace");
    let operation: tinyhosts_bus::rpc::Operation = tinyhosts_bus::rpc::Operation::PrepareBundle {
        directory: tinyhosts_bus::preparation::AuthorizedDirectory {
            workspace: workspace.to_string_lossy().into_owned(),
            path: "site".to_owned(),
            max_bytes: tinyhosts_bus::preparation::MAX_PREPARATION_BYTES,
            max_files: tinyhosts_bus::preparation::MAX_PREPARATION_FILES,
        },
    };
    let tinyhosts_bus::rpc::Outcome::PreparedBundle(prepared) = account
        .execute(operation)
        .await
        .expect("released module Providers and PrepareBundle calls")
    else {
        panic!("typed prepared bundle");
    };

    assert_eq!(prepared.contract_version, (1, 2));
    assert_eq!(prepared.file_count, 1);
    assert_eq!(prepared.total_bytes, html.len() as u64);
    assert_eq!(prepared.bundle.len(), 1);
    assert_eq!(prepared.bundle[0].path, "index.html");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&prepared.bundle[0].contents)
            .expect("base64 file content"),
        html,
        "the native preparation operation returns only the local fixture bytes"
    );
}

#[tokio::test]
async fn providers_and_launch_use_the_declared_bus_members_and_confidential_execute() {
    let workspace = tempfile::tempdir().expect("workspace");
    std::fs::create_dir(workspace.path().join("app")).unwrap();
    std::fs::write(workspace.path().join("app/index.html"), "hello").unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let account = account_fixture(workspace.path().to_path_buf(), Arc::clone(&calls));
    let tools = account.tools();
    let launch = tools
        .iter()
        .find(|tool| tool.name() == "hosting_launch_site")
        .unwrap();
    let result = launch
        .execute(serde_json::json!({"site":"demo","path":"app"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result
        .markdown_formatted
        .as_deref()
        .unwrap()
        .contains("deploy-1"));

    let calls = calls.lock().unwrap();
    assert_eq!(
        calls.len(),
        3,
        "Providers and two Execute operations are expected"
    );
    assert_eq!(
        calls[0],
        ("Providers".to_owned(), serde_json::Value::Null, false)
    );
    assert_eq!(calls[1].0, "Execute");
    assert!(calls[1].2, "PrepareBundle uses confidential Execute");
    assert_eq!(calls[2].0, "Execute");
    assert!(
        calls[2].2,
        "Launch carries credentials and must be confidential"
    );
    let prepared: serde_json::Value =
        serde_json::from_str(calls[1].1[0].as_str().unwrap()).unwrap();
    assert_eq!(prepared["operation"], "prepare_bundle");
    assert_eq!(prepared["directory"]["path"], "app");
    assert!(prepared["credentials"]["api_key"].as_str().unwrap() == "fixture-secret");
    let launched: serde_json::Value =
        serde_json::from_str(calls[2].1[0].as_str().unwrap()).unwrap();
    assert_eq!(launched["operation"], "launch");
    assert_eq!(launched["plan"]["bundle"][0]["path"], "index.html");
}

#[tokio::test]
async fn rollback_promotes_only_a_ready_deployment() {
    let workspace = tempfile::tempdir().expect("workspace");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let account = account_fixture(workspace.path().to_path_buf(), Arc::clone(&calls));
    let tools = account.tools();
    let rollback = tools
        .iter()
        .find(|tool| tool.name() == "hosting_rollback")
        .unwrap();
    let refused = rollback
        .execute(serde_json::json!({"site":"demo","deployment_id":"bad-1"}))
        .await
        .unwrap();
    assert!(refused.is_error);
    assert!(refused.content[0].render().contains("Failed"));
    let done = rollback
        .execute(serde_json::json!({"site":"demo","deployment_id":"ready-1"}))
        .await
        .unwrap();
    assert!(!done.is_error);
    let operations: Vec<String> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call.0 == "Execute")
        .map(|call| {
            let request: serde_json::Value =
                serde_json::from_str(call.1[0].as_str().unwrap()).unwrap();
            request["operation"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(operations, ["deployment", "deployment", "promote"]);
}

#[tokio::test]
async fn set_env_renders_the_module_summary_for_the_given_keys() {
    let workspace = tempfile::tempdir().expect("workspace");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let account = account_fixture(workspace.path().to_path_buf(), calls);
    let tools = account.tools();
    let set_env = tools
        .iter()
        .find(|tool| tool.name() == "hosting_set_env")
        .unwrap();
    let result = set_env
        .execute(serde_json::json!({"site":"demo","env":{"A":"1"}}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content[0].render().contains("Set A on demo."));
}

#[tokio::test]
async fn an_environment_credential_is_left_for_the_module_to_read() {
    let workspace = tempfile::tempdir().expect("workspace");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut account = account_fixture(workspace.path().to_path_buf(), Arc::clone(&calls));
    account.env_credential = true;
    let tools = account.tools();
    let list = tools
        .iter()
        .find(|tool| tool.name() == "hosting_list_sites")
        .unwrap();
    list.execute(serde_json::json!({})).await.unwrap();
    let calls = calls.lock().unwrap();
    let request: serde_json::Value = serde_json::from_str(calls[1].1[0].as_str().unwrap()).unwrap();
    assert!(request.get("credentials").is_none());
}

#[tokio::test]
async fn disabled_module_loader_does_not_fall_back_to_a_linked_provider() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut config = config_with(workspace.path(), true, "fixture-secret");
    config.modules.enabled = false;
    let account = Account::from_config(&config).unwrap().unwrap();
    let tools = account.tools();
    let list = tools
        .iter()
        .find(|tool| tool.name() == "hosting_list_sites")
        .unwrap();
    let result = list.execute(serde_json::json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.content[0]
        .render()
        .starts_with("MODULE_CALL_REPORTED:"));
}

#[tokio::test]
async fn connect_with_config_preserves_embedding_module_policy() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.modules.enabled = false;
    let account = Account::connect_with_config("vercel", "fixture-secret", None, config)
        .expect("account uses the supplied host configuration");
    assert_eq!(account.workspace_dir(), workspace.path());
    let list = account
        .tools()
        .into_iter()
        .find(|tool| tool.name() == "hosting_list_sites")
        .unwrap();
    let result = list.execute(serde_json::json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.content[0]
        .render()
        .starts_with("MODULE_CALL_REPORTED:"));
}

#[tokio::test]
async fn launch_rejects_a_forbidden_workspace_path_before_any_module_call() {
    let workspace = tempfile::tempdir().expect("workspace");
    std::fs::create_dir(workspace.path().join(".ssh")).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let account = account_fixture(workspace.path().to_path_buf(), Arc::clone(&calls));
    let launch = account
        .tools()
        .into_iter()
        .find(|tool| tool.name() == "hosting_launch_site")
        .unwrap();

    let result = launch
        .execute(serde_json::json!({"site":"demo", "path":".ssh"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn confidential_execute_failure_is_not_retried_as_an_ordinary_call() {
    let workspace = tempfile::tempdir().expect("workspace");
    let config = config_with(workspace.path(), true, "never-render-this-secret");
    let account = Account::from_config(&config).unwrap().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let calls_out = Arc::clone(&calls);
    let fixture = Arc::new(
        move |member: &str, args: serde_json::Value, confidential: bool| {
            calls_out
                .lock()
                .unwrap()
                .push((member.to_owned(), args, confidential));
            if member == tinyhosts_bus::METHODS[1] {
                Ok(serde_json::json!(["vercel"]).to_string())
            } else {
                Err(anyhow::anyhow!(
                    "MODULE_CALL_REPORTED: module transport failed"
                ))
            }
        },
    );
    let account = Account {
        call_fixture: Some(fixture),
        ..account
    };
    let list = account
        .tools()
        .into_iter()
        .find(|tool| tool.name() == "hosting_list_sites")
        .unwrap();

    let result = list.execute(serde_json::json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.content[0]
        .render()
        .contains("MODULE_CALL_REPORTED: module execution failed"));
    assert!(!result.content[0]
        .render()
        .contains("never-render-this-secret"));
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2, "one Providers call and one failed Execute");
    assert_eq!(calls[0].0, "Providers");
    assert_eq!(calls[1].0, "Execute");
    assert!(
        calls[1].2,
        "the failed credential-bearing call remains confidential"
    );
}

#[cfg(feature = "crash-reporting")]
#[test]
fn an_operation_with_the_wrong_typed_outcome_is_reported_once_without_reply_data() {
    let workspace = tempfile::tempdir().expect("workspace");
    let config = config_with(workspace.path(), true, "never-render-this-secret");
    let account = Account::from_config(&config).unwrap().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let calls_out = Arc::clone(&calls);
    let fixture = Arc::new(
        move |member: &str, args: serde_json::Value, confidential: bool| {
            calls_out
                .lock()
                .unwrap()
                .push((member.to_owned(), args.clone(), confidential));
            if member == tinyhosts_bus::METHODS[1] {
                return Ok(serde_json::json!(["vercel"]).to_string());
            }
            Ok(serde_json::json!({"result":"launch","value":{
                "site":{"id":"private-id","name":"private-site"},
                "created_site":true,
                "database":null,
                "database_env_keys":[],
                "domains":[],
                "deployment":{"id":"private-deployment","site":"private-site","status":"building"}
            }})
            .to_string())
        },
    );
    let account = Account {
        call_fixture: Some(fixture),
        ..account
    };
    let list = account
        .tools()
        .into_iter()
        .find(|tool| tool.name() == "hosting_list_sites")
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let events = sentry::test::with_captured_events(|| {
        runtime.block_on(async {
            let result = list.execute(serde_json::json!({})).await.unwrap();
            assert!(result.is_error);
            assert!(result.content[0]
                .render()
                .contains("MODULE_CALL_REPORTED: module execution failed"));
            assert!(!result.content[0].render().contains("private-site"));
        });
    });
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(
        events[0].tags.get("reason_code").map(String::as_str),
        Some("module_fault")
    );
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls[1].2);
}

#[test]
fn workspace_resolution_refuses_absolute_traversal_and_symlink_escape() {
    let workspace = tempfile::tempdir().expect("workspace");
    let outside = tempfile::tempdir().expect("outside");
    std::fs::create_dir(workspace.path().join("app")).unwrap();
    std::fs::create_dir(workspace.path().join(".ssh")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), workspace.path().join("escape")).unwrap();
    assert!(resolve_in_workspace(workspace.path(), "app").is_ok());
    assert!(resolve_in_workspace(workspace.path(), "../outside").is_err());
    assert!(resolve_in_workspace(workspace.path(), ".ssh").is_err());
    assert!(resolve_in_workspace(workspace.path(), outside.path().to_str().unwrap()).is_err());
    #[cfg(unix)]
    assert!(resolve_in_workspace(workspace.path(), "escape").is_err());
}

#[test]
fn only_the_tools_that_change_the_world_carry_an_external_effect() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let config = config_with(workspace.path(), true, "token");
    let account = Account::from_config(&config)
        .expect("resolution does not fail")
        .expect("an account");

    for tool in account.tools() {
        let expected = matches!(
            tool.name(),
            "hosting_launch_site" | "hosting_set_env" | "hosting_add_domain" | "hosting_rollback"
        );
        assert_eq!(
            tool.external_effect(),
            expected,
            "{} has the wrong external effect",
            tool.name()
        );
    }
}

#[test]
fn a_disabled_hosting_component_registers_no_tools_despite_a_credential() {
    let workspace = tempfile::tempdir().expect("workspace");
    let config = config_with(workspace.path(), false, "token");
    assert!(Account::from_config(&config).unwrap().is_none());
    let config = config_with(workspace.path(), true, "token");
    assert!(Account::from_config(&config).unwrap().is_some());
}
