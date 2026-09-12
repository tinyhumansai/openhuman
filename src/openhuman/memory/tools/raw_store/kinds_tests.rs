use super::*;

#[test]
fn parameters_schema_is_empty_object() {
    let tool = MemoryStoreKindsTool;
    let schema = tool.parameters_schema();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"], json!({}));
}

/// The catalog is the driver's now, so this needs one bound.
///
/// It used to assert against `MemoryKind::ALL` compiled into this crate,
/// which is exactly the host-side copy that had drifted from the engine.
#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the catalog is read from the bound driver, not a compiled-in list"]
async fn execute_returns_the_drivers_storage_kinds() {
    let tool = MemoryStoreKindsTool;
    let result = tool.execute(Value::Null).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
    assert!(
        parsed["kinds"].as_array().is_some_and(|k| !k.is_empty()),
        "a bound driver must report a non-empty catalog"
    );
}
