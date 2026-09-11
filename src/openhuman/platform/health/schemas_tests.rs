use super::*;

#[test]
fn all_schemas_returns_two() {
    assert_eq!(all_controller_schemas().len(), 2);
}

#[test]
fn all_controllers_returns_two() {
    assert_eq!(all_registered_controllers().len(), 2);
}

#[test]
fn snapshot_schema() {
    let s = schemas("snapshot");
    assert_eq!(s.namespace, "health");
    assert_eq!(s.function, "snapshot");
    assert!(s.inputs.is_empty());
    assert!(!s.outputs.is_empty());
}

#[test]
fn system_info_schema() {
    let s = schemas("system_info");
    assert_eq!(s.namespace, "health");
    assert_eq!(s.function, "system_info");
    assert!(s.inputs.is_empty());
    // version, os, arch, pid
    assert_eq!(s.outputs.len(), 4);
}

#[test]
fn unknown_function_returns_unknown() {
    let s = schemas("bad");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "health");
}

#[test]
fn schemas_and_controllers_match() {
    let s = all_controller_schemas();
    let c = all_registered_controllers();
    assert_eq!(s.len(), c.len());
    for (schema, controller) in s.iter().zip(c.iter()) {
        assert_eq!(schema.function, controller.schema.function);
    }
}

#[tokio::test]
async fn handle_snapshot_returns_json_object() {
    let result = handle_snapshot(Map::new()).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_object());
}

#[tokio::test]
async fn handle_system_info_returns_json_object() {
    let result = handle_system_info(Map::new()).await;
    assert!(result.is_ok());
    let json = result.unwrap();
    assert!(json.is_object());
    assert!(json["version"].as_str().is_some());
    assert!(json["os"].as_str().is_some());
    assert!(json["arch"].as_str().is_some());
    assert!(json["pid"].as_u64().is_some());
}

#[test]
fn to_json_helper() {
    let outcome = RpcOutcome::single_log(serde_json::json!({"ok": true}), "log");
    assert!(to_json(outcome).is_ok());
}

/// Every declared output type must match the JSON the handler actually emits.
///
/// `pid` drifted exactly this way (#6074): the schema said `TypeSchema::String`
/// while `SystemInfo.pid` is a `u32` that serializes as a JSON number, so the
/// generated frontend types and the model-facing tool `output_schema` both
/// advertised a type the wire never carried. Output fields are not validated at
/// dispatch, so nothing at runtime would have caught it — checking the
/// declaration against a live response is what keeps the two from separating
/// again.
#[tokio::test]
async fn system_info_declared_output_types_match_emitted_json() {
    let schema = schemas("system_info");
    let json = handle_system_info(Map::new())
        .await
        .expect("system_info handler succeeds");

    assert_eq!(schema.outputs.len(), 4, "version, os, arch, pid");
    for field in &schema.outputs {
        let value = json
            .get(field.name)
            .unwrap_or_else(|| panic!("`{}` missing from the response", field.name));
        let ok = match field.ty {
            TypeSchema::String => value.is_string(),
            TypeSchema::U64 => value.is_u64(),
            // Deliberately exhaustive-by-panic: a field declared as some other
            // type has no check here yet, and silently passing would defeat the
            // point of this test.
            ref other => panic!("`{}` declares {other:?}; add a check for it", field.name),
        };
        assert!(
            ok,
            "`{}` declares {:?} but the handler emitted {value}",
            field.name, field.ty
        );
    }
}
