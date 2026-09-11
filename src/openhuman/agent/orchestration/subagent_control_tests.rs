use super::*;

#[test]
fn registered_controllers_match_schemas() {
    let schemas = all_controller_schemas();
    let registered = all_registered_controllers();
    assert_eq!(schemas.len(), registered.len());
    assert_eq!(schemas.len(), 2);
    assert_eq!(schema_for("subagent_cancel").namespace, "subagent");
    assert_eq!(schema_for("subagent_cancel").function, "cancel");
    assert_eq!(schema_for("subagent_steer").namespace, "subagent");
    assert_eq!(schema_for("subagent_steer").function, "steer");
}

#[test]
fn require_str_rejects_blank_and_missing() {
    let mut params = Map::new();
    assert!(require_str(&params, "taskId").is_err());
    params.insert("taskId".into(), json!("   "));
    assert!(require_str(&params, "taskId").is_err());
    params.insert("taskId".into(), json!("sub-1"));
    assert_eq!(require_str(&params, "taskId").unwrap(), "sub-1");
    // Whitespace-padded ids are trimmed so they match the registry key.
    params.insert("taskId".into(), json!("  sub-1  "));
    assert_eq!(require_str(&params, "taskId").unwrap(), "sub-1");
}

#[tokio::test]
async fn cancel_unknown_task_is_a_noop_false() {
    let _lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut params = Map::new();
    params.insert("taskId".into(), json!("sub-does-not-exist"));
    let out = handle_subagent_cancel(params).await.expect("handler ok");
    // RpcOutcome wraps the payload under `data`.
    let cancelled = out
        .get("data")
        .and_then(|d| d.get("cancelled"))
        .or_else(|| out.get("cancelled"))
        .and_then(Value::as_bool);
    assert_eq!(cancelled, Some(false));
}

#[tokio::test]
async fn steer_unknown_task_is_a_noop_false() {
    let _lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut params = Map::new();
    params.insert("taskId".into(), json!("sub-does-not-exist"));
    params.insert("message".into(), json!("redirect"));
    let out = handle_subagent_steer(params).await.expect("handler ok");
    let data = out.get("data").unwrap_or(&out);
    assert_eq!(data.get("steered").and_then(Value::as_bool), Some(false));
    assert_eq!(data.get("reason").and_then(Value::as_str), Some("unknown"));
}
