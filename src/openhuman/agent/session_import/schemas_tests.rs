use super::*;

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

    let result = handle_session_import_run(params).await.expect("handler ok");
    assert_eq!(result["dry_run"], Value::Bool(true));
    assert_eq!(result["scanned"], 0);
}

#[tokio::test]
async fn handler_rejects_invalid_params() {
    let mut params = Map::new();
    params.insert("only".into(), Value::Bool(true)); // wrong type
    let err = handle_session_import_run(params).await.unwrap_err();
    assert!(err.contains("invalid params"), "{err}");
}
