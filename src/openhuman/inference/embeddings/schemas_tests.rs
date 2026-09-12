use super::*;

#[test]
fn all_schemas_count() {
    assert_eq!(all_controller_schemas().len(), 6);
}

#[test]
fn all_controllers_count() {
    assert_eq!(all_registered_controllers().len(), 6);
}

#[test]
fn schemas_and_controllers_match() {
    let s = all_controller_schemas();
    let c = all_registered_controllers();
    for (schema, ctrl) in s.iter().zip(c.iter()) {
        assert_eq!(schema.function, ctrl.schema.function);
        assert_eq!(schema.namespace, "embeddings");
    }
}

#[test]
fn unknown_function_returns_unknown() {
    let s = schemas("bad");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "embeddings");
}

#[tokio::test]
async fn all_handlers_accept_empty_params_without_panic() {
    // Every handler should return a result (Ok or Err) when called with
    // empty params — it must not panic. Handlers that require params will
    // return an error, which is also acceptable here.
    let controllers = all_registered_controllers();
    for ctrl in controllers {
        let params = serde_json::Map::new();
        // The handler is an async fn pointer; calling it with empty params
        // must not panic. We don't assert Ok because mandatory-param
        // handlers legitimately return Err("invalid params: ...").
        let _result = (ctrl.handler)(params).await;
        // If we reach here the handler did not panic.
    }
}
