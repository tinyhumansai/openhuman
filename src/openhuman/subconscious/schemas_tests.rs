use super::*;

#[test]
fn all_schemas_returns_thirteen() {
    // 10 task/escalation schemas + 3 reflection schemas (#623).
    assert_eq!(all_controller_schemas().len(), 13);
}

#[test]
fn all_controllers_returns_thirteen() {
    assert_eq!(all_registered_controllers().len(), 13);
}

#[test]
fn reflection_rpcs_are_registered() {
    let names: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.function)
        .collect();
    assert!(names.contains(&"reflections_list"));
    assert!(names.contains(&"reflections_act"));
    assert!(names.contains(&"reflections_dismiss"));
}

#[test]
fn all_use_subconscious_namespace() {
    for s in all_controller_schemas() {
        assert_eq!(s.namespace, "subconscious");
        assert!(!s.description.is_empty());
    }
}

#[test]
fn schemas_and_controllers_match() {
    let s = all_controller_schemas();
    let c = all_registered_controllers();
    for (schema, ctrl) in s.iter().zip(c.iter()) {
        assert_eq!(schema.function, ctrl.schema.function);
    }
}

#[test]
fn known_functions_resolve() {
    for fn_name in [
        "status",
        "trigger",
        "tasks_list",
        "tasks_add",
        "tasks_update",
        "tasks_remove",
        "log_list",
        "escalations_list",
        "escalations_approve",
        "escalations_dismiss",
    ] {
        let s = schemas(fn_name);
        assert_ne!(s.function, "unknown", "{fn_name} fell through");
    }
}

#[test]
fn unknown_function_returns_unknown() {
    let s = schemas("nonexistent");
    assert_eq!(s.function, "unknown");
}

#[test]
fn status_schema_has_no_inputs() {
    assert!(schemas("status").inputs.is_empty());
}

#[test]
fn trigger_schema_has_no_inputs() {
    assert!(schemas("trigger").inputs.is_empty());
}

#[test]
fn tasks_add_requires_title() {
    let s = schemas("tasks_add");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"title"));
}

#[test]
fn tasks_update_requires_task_id() {
    let s = schemas("tasks_update");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"task_id"));
}

#[test]
fn tasks_remove_requires_task_id() {
    let s = schemas("tasks_remove");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"task_id"));
}

#[test]
fn escalations_approve_requires_escalation_id() {
    let s = schemas("escalations_approve");
    assert!(s
        .inputs
        .iter()
        .any(|f| f.name == "escalation_id" && f.required));
}

#[test]
fn escalations_dismiss_requires_escalation_id() {
    let s = schemas("escalations_dismiss");
    assert!(s
        .inputs
        .iter()
        .any(|f| f.name == "escalation_id" && f.required));
}

#[test]
fn log_list_has_optional_inputs() {
    let s = schemas("log_list");
    for input in &s.inputs {
        assert!(
            !input.required,
            "log_list input '{}' should be optional",
            input.name
        );
    }
}

#[test]
fn tasks_list_has_optional_enabled_only() {
    let s = schemas("tasks_list");
    let enabled = s.inputs.iter().find(|f| f.name == "enabled_only");
    assert!(enabled.is_some_and(|f| !f.required));
}

// ── Field helpers ──────────────────────────────────────────────

#[test]
fn field_helper_is_required() {
    let f = field("name", TypeSchema::String, "desc");
    assert!(f.required);
}

#[test]
fn field_req_helper_is_required() {
    let f = field_req("name", TypeSchema::String, "desc");
    assert!(f.required);
}

#[test]
fn field_opt_helper_is_not_required() {
    let f = field_opt("name", TypeSchema::String, "desc");
    assert!(!f.required);
}

// ── Error chain preservation ───────────────────────────────────
//
// The RPC handlers in this module bridge `anyhow::Result` (from
// `store::with_connection` and the wrapped rusqlite errors) into the
// JSON-RPC `Result<Value, String>` boundary via `map_err(|e| ...)`.
//
// **Critical for observability**: plain `e.to_string()` on an
// `anyhow::Error` returns ONLY the outermost context. For a
// `with_connection` failure the outer wrap is
// `"failed to run subconscious schema DDL"` — the underlying rusqlite
// root (the actual SQLite error code + message) is dropped. That
// stringified message is what `jsonrpc::invoke_method_inner` later
// passes to `report_error_or_expected`, which in turn captures it in
// Sentry. Without the chain, Sentry events for TAURI-RUST-A only
// surface the generic wrapper text and the rusqlite root cause is
// permanently invisible.
//
// All `map_err` sites in `schemas.rs` use `format!("{e:#}")` (anyhow's
// alternate Display walks the cause chain inline joined by `": "`) so
// the rusqlite root reaches Sentry. These guard tests pin the format
// so future contributors don't silently regress to `e.to_string()`.
#[test]
fn anyhow_alternate_display_walks_chain() {
    use anyhow::Context;

    let inner = anyhow::anyhow!("database is locked").context("execute_batch failed");
    let outer: anyhow::Result<()> = Err(inner).context("failed to run subconscious schema DDL");

    let err = outer.unwrap_err();

    // Plain to_string() — the broken (pre-fix) shape. Only outer
    // wrapper reaches the caller, root cause lost.
    let lossy = err.to_string();
    assert_eq!(lossy, "failed to run subconscious schema DDL");
    assert!(
        !lossy.contains("database is locked"),
        "plain Display must drop the root cause — if this changes the chain-formatter \
         is no longer load-bearing, revisit observability assumptions"
    );

    // Alternate Display — what schemas.rs map_err now produces. Every
    // layer joined by ": " so the rusqlite root reaches Sentry.
    let full = format!("{err:#}");
    assert!(
        full.contains("failed to run subconscious schema DDL"),
        "chain-formatted message must include outer wrapper, got: {full}"
    );
    assert!(
        full.contains("execute_batch failed"),
        "chain-formatted message must include middle context, got: {full}"
    );
    assert!(
        full.contains("database is locked"),
        "chain-formatted message must include the rusqlite root, got: {full}"
    );
}

#[test]
fn anyhow_alternate_display_includes_rusqlite_error_chain() {
    use anyhow::Context;

    // Simulate the exact shape produced by `with_connection`:
    // a real rusqlite Error wrapped in `with_context(...)`.
    let raw = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error {
            code: rusqlite::ErrorCode::DatabaseBusy,
            extended_code: 5,
        },
        Some("database is locked".into()),
    );
    let wrapped: anyhow::Result<()> =
        Err(anyhow::Error::from(raw)).context("failed to run subconscious schema DDL");

    let err = wrapped.unwrap_err();
    let chained = format!("{err:#}");

    // Outer wrapper preserved.
    assert!(chained.contains("failed to run subconscious schema DDL"));
    // rusqlite-rendered root preserved — this is the signal Sentry
    // needs to distinguish a DDL lock-race from a corruption / disk-full
    // / permission failure. Without it, all four fingerprint identically.
    assert!(
        chained.contains("database is locked"),
        "rusqlite root must appear in chain-formatted message, got: {chained}"
    );
}
