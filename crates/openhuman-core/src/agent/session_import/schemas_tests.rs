use super::*;
use crate::core::runtime::DispatchCtx;

#[test]
fn controller_schema_inventory_is_stable() {
    let schemas = all_session_import_controller_schemas();
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].namespace, "session_import");
    assert_eq!(schemas[0].function, "run");

    let controllers = all_session_import_registered_controllers();
    assert_eq!(controllers.len(), 1);
    assert_eq!(controllers[0].schema.function, "run");
}

#[tokio::test]
async fn handler_runs_dry_against_explicit_workspace() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut params = Map::new();
    params.insert("dry_run".into(), Value::Bool(true));
    params.insert(
        "workspace".into(),
        Value::String(tmp.path().to_string_lossy().to_string()),
    );

    let result = handle_session_import_run(DispatchCtx::process_default(), params)
        .await
        .expect("handler ok");
    assert_eq!(result["dry_run"], Value::Bool(true));
    assert_eq!(result["sources"]["sessions_db"], Value::Bool(false));
    assert_eq!(result["imports"], json!([]));
}

#[tokio::test]
async fn dry_run_reports_the_legacy_sources_a_workspace_holds() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("session_raw")).unwrap();
    let mut params = Map::new();
    params.insert("dry_run".into(), Value::Bool(true));
    params.insert(
        "workspace".into(),
        Value::String(tmp.path().to_string_lossy().to_string()),
    );

    let result = handle_session_import_run(DispatchCtx::process_default(), params)
        .await
        .expect("handler ok");
    assert_eq!(result["sources"]["session_raw"], Value::Bool(true));
    assert_eq!(result["sources"]["turn_states"], Value::Bool(false));
}

#[tokio::test]
async fn handler_rejects_invalid_params() {
    let mut params = Map::new();
    params.insert("dry_run".into(), Value::String("yes".into())); // wrong type
    let err = handle_session_import_run(DispatchCtx::process_default(), params)
        .await
        .unwrap_err();
    assert!(err.contains("invalid params"), "{err}");
}

#[test]
fn an_unknown_function_gets_the_error_schema() {
    let schema = session_import_schemas("no_such_function");
    assert_eq!(schema.namespace, "session_import");
    assert_eq!(schema.function, "unknown");
    assert_eq!(schema.outputs.len(), 1);
    assert_eq!(schema.outputs[0].name, "error");
}

/// Runs the import against `workspace` (or the configured one when `None`)
/// with the configured workspace pointed at a scratch directory, so nothing
/// touches the operator's home.
async fn run_import(workspace: Option<&std::path::Path>) -> Value {
    let configured = tempfile::TempDir::new().unwrap();
    let _env = crate::config::test_env::EnvVarGuard::workspace_async(configured.path()).await;
    let mut params = Map::new();
    if let Some(workspace) = workspace {
        params.insert(
            "workspace".into(),
            Value::String(workspace.to_string_lossy().to_string()),
        );
    }
    let result = handle_session_import_run(DispatchCtx::process_default(), params).await;
    result.expect("the import handler answers even with nothing to import")
}

#[tokio::test]
async fn a_real_run_on_an_empty_configured_workspace_reports_each_domain() {
    let result = run_import(None).await;
    assert_eq!(result["dry_run"], Value::Bool(false));
    assert!(result["imports"].is_array());
    assert!(result["sources"].is_object());
}

#[tokio::test]
async fn a_real_run_on_an_overridden_workspace_opens_that_workspaces_backend() {
    let other = tempfile::TempDir::new().unwrap();
    let result = run_import(Some(other.path())).await;
    assert_eq!(result["dry_run"], Value::Bool(false));
    assert_eq!(
        result["workspace"],
        Value::String(other.path().to_string_lossy().to_string())
    );
    let imports = result["imports"].as_array().expect("imports array");
    assert!(
        imports.iter().all(|import| import.get("domain").is_some()),
        "{imports:?}"
    );
}
