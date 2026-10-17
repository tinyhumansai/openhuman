use super::*;
use crate::core::runtime::DispatchCtx;

#[test]
fn all_controller_schemas_lists_registered_functions() {
    let schemas = all_controller_schemas();
    assert_eq!(schemas.len(), 3);
    // The six read-only `session_db` controllers were removed in #6082; only the
    // durable run-ledger read surface remains here.
    assert!(schemas
        .iter()
        .all(|schema| schema.namespace == "run_ledger"));
}

/// The removed `session_db` surface must stay removed: none of the six read
/// controllers may reappear in this module's aggregators (#6082).
#[test]
fn session_db_controllers_are_absent() {
    let removed = [
        "list",
        "get",
        "search",
        "get_messages",
        "get_tool_calls",
        "get_children",
    ];
    let schemas = all_controller_schemas();
    assert!(
        schemas
            .iter()
            .all(|schema| schema.namespace != "session_db"),
        "the removed `session_db` namespace must not be registered"
    );
    let registered = all_registered_controllers();
    for name in removed {
        assert!(
            registered
                .iter()
                .all(|rc| rc.schema.namespace != "session_db" || rc.schema.function != name),
            "removed session_db controller '{name}' must not be registered"
        );
    }
}

#[test]
fn all_registered_controllers_match_schemas() {
    let registered = all_registered_controllers();
    let schemas = all_controller_schemas();
    assert_eq!(registered.len(), schemas.len());

    let schema_fns: Vec<&str> = schemas.iter().map(|s| s.function).collect();
    for rc in &registered {
        assert!(
            schema_fns.contains(&rc.schema.function),
            "registered controller '{}' not in schema list",
            rc.schema.function
        );
    }
}

#[test]
fn schema_for_run_ledger_events_requires_run_id() {
    let s = schema_for("run_ledger_events");
    assert_eq!(s.namespace, "run_ledger");
    assert_eq!(s.function, "events");
    assert!(s
        .inputs
        .iter()
        .any(|input| input.name == "runId" && input.required));
}

#[test]
fn new_correlation_id_is_eight_hex_chars() {
    let cid = new_correlation_id();
    assert_eq!(cid.len(), 8);
    assert!(cid.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn an_unknown_function_gets_the_error_schema() {
    let s = schema_for("no_such_function");
    assert_eq!(s.namespace, "run_ledger");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.outputs[0].name, "error");
}

/// Runs `handler` with the configured workspace pointed at a scratch
/// directory, so the ledger it reads is empty and nothing touches the
/// operator's home.
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
async fn listing_an_empty_ledger_returns_no_runs() {
    let value = with_scratch_workspace(|| {
        handle_run_ledger_list(DispatchCtx::process_default(), Map::new())
    })
    .await
    .expect("list answers on an empty ledger");
    let text = value.to_string();
    assert_eq!(value["runs"], serde_json::json!([]), "{text}");
    assert_eq!(value["count"], 0, "{text}");
}

#[tokio::test]
async fn listing_rejects_malformed_params() {
    let err = with_scratch_workspace(|| {
        handle_run_ledger_list(
            DispatchCtx::process_default(),
            params(serde_json::json!({ "limit": "many" })),
        )
    })
    .await
    .expect_err("a non-numeric limit is refused");
    assert!(err.contains("invalid run ledger list params"), "{err}");
}

#[tokio::test]
async fn getting_an_unknown_run_answers_null() {
    let value = with_scratch_workspace(|| {
        handle_run_ledger_get(
            DispatchCtx::process_default(),
            params(serde_json::json!({ "id": "no-such-run" })),
        )
    })
    .await
    .expect("get answers for an unknown id");
    assert!(value["run"].is_null(), "{value}");
}

#[tokio::test]
async fn getting_a_run_requires_an_id() {
    let err = with_scratch_workspace(|| {
        handle_run_ledger_get(DispatchCtx::process_default(), Map::new())
    })
    .await
    .expect_err("id is required");
    assert!(err.contains("missing required param: id"), "{err}");
}

#[tokio::test]
async fn events_of_an_unknown_run_are_empty() {
    let value = with_scratch_workspace(|| {
        handle_run_ledger_events(
            DispatchCtx::process_default(),
            params(serde_json::json!({ "runId": "no-such-run" })),
        )
    })
    .await
    .expect("events answer for an unknown run");
    assert_eq!(value["events"], serde_json::json!([]), "{value}");
    assert_eq!(value["count"], 0, "{value}");
}

#[tokio::test]
async fn events_reject_malformed_params() {
    let err = with_scratch_workspace(|| {
        handle_run_ledger_events(DispatchCtx::process_default(), Map::new())
    })
    .await
    .expect_err("runId is required");
    assert!(err.contains("invalid run ledger events params"), "{err}");
}
