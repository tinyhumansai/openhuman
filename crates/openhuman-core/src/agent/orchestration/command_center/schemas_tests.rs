use super::*;
use crate::core::runtime::DispatchCtx;

#[test]
fn registered_controllers_match_schemas() {
    let schemas = all_controller_schemas();
    let registered = all_registered_controllers();
    assert_eq!(schemas.len(), registered.len());
    assert_eq!(schemas.len(), 2);
    assert!(schemas.iter().all(|s| s.namespace == "agent_work"));
    assert_eq!(schema_for("agent_work_list").function, "list");
    assert_eq!(schema_for("agent_work_control").function, "control");
}

#[test]
fn unknown_agent_has_no_display_name() {
    assert_eq!(resolve_display_name("__definitely_not_an_agent__"), None);
}

#[test]
fn an_unknown_function_gets_the_unknown_schema() {
    let s = schema_for("no_such_function");
    assert_eq!(s.namespace, "agent_work");
    assert_eq!(s.function, "unknown");
}

#[test]
fn require_and_optional_strings_ignore_blank_values() {
    let mut params = Map::new();
    params.insert("present".into(), Value::String("x".into()));
    params.insert("blank".into(), Value::String("   ".into()));
    assert_eq!(require_str(&params, "present").unwrap(), "x");
    assert_eq!(
        require_str(&params, "blank").unwrap_err(),
        "missing required param: blank"
    );
    assert_eq!(
        require_str(&params, "absent").unwrap_err(),
        "missing required param: absent"
    );
    assert_eq!(opt_str(&params, "present").as_deref(), Some("x"));
    assert_eq!(opt_str(&params, "blank"), None);
    assert_eq!(opt_str(&params, "absent"), None);
}

async fn with_scratch_workspace<F, Fut>(handler: F) -> Result<Value, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    let tmp = tempfile::tempdir().expect("tempdir");
    let _env = crate::config::test_env::EnvVarGuard::workspace_async(tmp.path()).await;
    handler().await
}

fn params(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

#[tokio::test]
async fn listing_work_on_an_empty_ledger_answers() {
    let v = with_scratch_workspace(|| {
        handle_agent_work_list(
            DispatchCtx::process_default(),
            params(serde_json::json!({ "limit": 5 })),
        )
    })
    .await
    .expect("an empty ledger lists as an empty view");
    assert_eq!(v["total"], 0, "{v}");
    assert!(
        v["groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|g| g["rows"] == serde_json::json!([])),
        "{v}"
    );
}

#[tokio::test]
async fn control_requires_a_run_id_and_action() {
    let err = with_scratch_workspace(|| {
        handle_agent_work_control(DispatchCtx::process_default(), Map::new())
    })
    .await
    .unwrap_err();
    assert_eq!(err, "missing required param: runId");
    let err = with_scratch_workspace(|| {
        handle_agent_work_control(
            DispatchCtx::process_default(),
            params(serde_json::json!({ "runId": "r1" })),
        )
    })
    .await
    .unwrap_err();
    assert_eq!(err, "missing required param: action");
}

#[tokio::test]
async fn control_refuses_an_unknown_action() {
    let err = with_scratch_workspace(|| {
        handle_agent_work_control(
            DispatchCtx::process_default(),
            params(serde_json::json!({ "runId": "r1", "action": "explode" })),
        )
    })
    .await
    .unwrap_err();
    assert_eq!(err, "unknown control action: explode");
}

#[tokio::test]
async fn control_of_an_unknown_run_is_an_error_not_a_panic() {
    let err = with_scratch_workspace(|| {
        handle_agent_work_control(
            DispatchCtx::process_default(),
            params(
                serde_json::json!({ "runId": "no-such-run", "action": "stop", "reason": "test" }),
            ),
        )
    })
    .await
    .unwrap_err();
    assert!(!err.is_empty());
}
