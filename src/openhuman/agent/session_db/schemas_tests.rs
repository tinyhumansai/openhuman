use super::*;

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
