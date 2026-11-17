use super::*;

// ---- memory_diff removal ---------------------------------------------------

/// The `memory_diff` controllers were deleted along with the `memory-git`
/// feature, and must stay gone — while the rest of the memory surface stays.
///
/// `memory` is asserted present in the same test on purpose: the removal took
/// the git ledger, not the memory domain. Splitting that into a separate test
/// would let one pass while the other silently regressed. This replaces the
/// `{registered_when_feature_on,absent_when_feature_off}` pair that pinned the
/// gate while it existed.
#[test]
fn memory_diff_controllers_are_gone_and_memory_survives() {
    let namespaces: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.namespace)
        .collect();
    assert!(
        !namespaces.contains(&"memory_diff"),
        "`memory_diff` was removed and must not be registered, got: {namespaces:?}"
    );
    assert!(
        namespaces.contains(&"memory"),
        "removing the git ledger must not remove the memory domain"
    );
}

// ---- session_db removal (#6082) --------------------------------------------

/// The six read-only `session_db` controllers were removed in #6082: they
/// queried a session index that nothing in `src/` ever writes (permanently
/// empty in production, no frontend consumer). The three `run_ledger`
/// controllers live in the same module and read a table that *is* written
/// (from `web_chat::progress` and `core::observability::progress_tracing`), so they
/// must stay fully registered.
///
/// `run_ledger` is asserted present in the same test on purpose: the removal
/// took the dead read surface, not the run-ledger domain. Splitting that into a
/// separate test would let one pass while the other silently regressed.
#[test]
fn session_db_controllers_are_gone_and_run_ledger_survives() {
    let methods: Vec<String> = all_controller_schemas()
        .iter()
        .map(rpc_method_name)
        .collect();

    for removed in [
        "openhuman.session_db_list",
        "openhuman.session_db_get",
        "openhuman.session_db_search",
        "openhuman.session_db_get_messages",
        "openhuman.session_db_get_tool_calls",
        "openhuman.session_db_get_children",
    ] {
        assert!(
            !methods.contains(&removed.to_string()),
            "removed session_db controller `{removed}` must be absent \
             (unknown-method over /rpc, omitted from /schema), got: {methods:?}"
        );
    }

    for kept in [
        "openhuman.run_ledger_list",
        "openhuman.run_ledger_get",
        "openhuman.run_ledger_events",
    ] {
        assert!(
            methods.contains(&kept.to_string()),
            "run_ledger controller `{kept}` must stay registered — removing the \
             dead session_db read surface must not touch the run ledger"
        );
    }

    let namespaces: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.namespace)
        .collect();
    assert!(
        !namespaces.contains(&"session_db"),
        "the `session_db` namespace was removed and must not be registered, got: {namespaces:?}"
    );
}
