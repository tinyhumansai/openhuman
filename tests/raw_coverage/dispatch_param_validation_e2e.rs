//! What a caller actually observes when a dispatched call carries bad params.
//!
//! `core::all::validate_params` is the single pre-dispatch gate for every
//! registered controller, so for an *absent*, *unknown* or *wrong-typed* param a
//! handler's own "missing required param" string never reaches a caller — the
//! gate refuses first, in different wording.
//!
//! It is not a total shield. The gate accepts an explicit JSON `null` for any
//! declared type: the required check tests key *presence*, and its type check
//! returns early on null. So a dispatched `{"class": null}` runs the handler and
//! the handler's own refusal is what comes back. That is why #6073 keeps the
//! handler-side checks instead of deleting them — on the null path they are the
//! only guard.
//!
//! Most suites in this directory invoke controllers directly
//! (`(controller.handler)(params).await`), skipping validation altogether, so it
//! is easy to write a test that looks like it covers the dispatch path and does
//! not. These tests pin the observable behaviour on both sides of that line.

use serde_json::{json, Map};

use openhuman_core::core::all::{
    all_registered_controllers, rpc_method_name, schema_for_rpc_method,
};
use openhuman_core::core::dispatch::dispatch;
use openhuman_core::core::types::AppState;

fn state() -> AppState {
    AppState {
        core_version: "dispatch-param-validation-e2e".to_string(),
    }
}

/// The refusal a caller sees carries the *schema's* comment, not the handler's
/// wording — which is what makes the two impossible to mistake for each other.
#[tokio::test]
async fn missing_required_param_is_refused_with_the_schema_comment() {
    let schema = schema_for_rpc_method("openhuman.run_ledger_get")
        .expect("run_ledger_get is registered unconditionally");
    let comment = schema
        .inputs
        .iter()
        .find(|field| field.name == "id")
        .expect("`id` input")
        .comment;

    let err = dispatch(state(), "openhuman.run_ledger_get", json!({}))
        .await
        .expect_err("`id` is required");

    assert_eq!(err, format!("missing required param 'id': {comment}"));

    // `handle_run_ledger_get` refuses with `missing required param: id`. If that
    // ever starts matching, validation has moved into the handler and the shape
    // of what callers see has changed with it.
    assert!(!err.contains("missing required param: id"), "got: {err}");
}

/// A param that no schema declares is refused before dispatch. No handler
/// anywhere implements an unknown-param check, so this message can only come
/// from `validate_params` — it is the proof that the gate ran.
#[tokio::test]
async fn unknown_param_is_refused_by_the_gate_alone() {
    let err = dispatch(
        state(),
        "openhuman.memory_engines_list",
        json!({ "nonsense_param": 1 }),
    )
    .await
    .expect_err("unknown params are refused");

    assert_eq!(err, "unknown param 'nonsense_param' for memory.engines_list");
}

/// Declared types are enforced at the same gate, so a handler's own
/// `serde_json::from_value` error is not what a caller gets either.
#[tokio::test]
async fn mistyped_param_is_refused_with_the_declared_type() {
    let err = dispatch(
        state(),
        "openhuman.session_import_run",
        json!({ "dry_run": "yes" }),
    )
    .await
    .expect_err("`dry_run` is declared bool");

    assert_eq!(
        err,
        "invalid type for param 'dry_run' in session_import.run: expected bool, got string"
    );
    assert!(!err.contains("invalid params:"), "got: {err}");
}

/// For an *absent* param the two refusals are different strings, and the
/// dispatcher's is the one a caller gets. The handler's own check still fires —
/// it just takes a direct invocation to see it, the way the suites in this
/// directory call controllers.
#[tokio::test]
async fn handler_refusal_differs_from_the_dispatcher_refusal_for_an_absent_param() {
    let dispatched = dispatch(state(), "openhuman.memory_recall", json!({}))
        .await
        .expect_err("`question` is required");
    assert!(
        dispatched.starts_with("missing required param 'question': "),
        "got: {dispatched}"
    );

    let controllers = all_registered_controllers();
    let controller = controllers
        .iter()
        .find(|controller| rpc_method_name(&controller.schema) == "openhuman.memory_recall")
        .expect("memory_recall is registered unconditionally");
    let direct = (controller.handler)(Map::new())
        .await
        .expect_err("the handler refuses too");

    assert!(
        direct.contains("INVALID_REQUEST") && direct.contains("question"),
        "the handler refuses with the memory taxonomy's INVALID_REQUEST: {direct}"
    );
    assert_ne!(
        dispatched, direct,
        "the two refusals must stay distinguishable: a test written against the \
         handler's wording is testing the handler, not the dispatch path (#6073)"
    );
}

/// An explicit `null` satisfies the gate and reaches the handler, so here the
/// handler's own refusal *is* what the caller sees. This is the case that makes
/// the handler-side checks load-bearing rather than redundant, and the reason
/// #6073 documents them instead of removing them.
#[tokio::test]
async fn explicit_null_passes_the_gate_and_the_handler_refuses_instead() {
    let err = dispatch(
        state(),
        "openhuman.memory_recall",
        json!({ "question": null }),
    )
    .await
    .expect_err("`question` is null, so the handler refuses");

    // The handler's wording (the memory error taxonomy), not the dispatcher's
    // — the mirror image of
    // `missing_required_param_is_refused_with_the_schema_comment`.
    assert!(err.contains("INVALID_REQUEST"), "got: {err}");
    assert!(
        !err.starts_with("missing required param 'question'"),
        "the gate must NOT have refused this: {err}"
    );
}

/// A field backed by a `u32` declares its real range, so a value that is a
/// valid `u64` but not a valid `u32` is refused at the gate with the bound
/// named — rather than passing validation and failing (or, before #6137, being
/// silently clamped to "unlimited") inside the handler.
#[tokio::test]
async fn out_of_range_integer_is_refused_with_the_bound_named() {
    let err = dispatch(
        state(),
        "openhuman.config_update_autonomy_settings",
        json!({ "max_actions_per_hour": 4_294_967_296u64 }),
    )
    .await
    .expect_err("4294967296 does not fit the u32 field");
    assert_eq!(
        err,
        "invalid type for param 'max_actions_per_hour' in config.update_autonomy_settings: \
         expected unsigned integer <= 4294967295, got 4294967296"
    );

    let err = dispatch(
        state(),
        "openhuman.config_update_autonomy_settings",
        json!({ "max_actions_per_hour": 0 }),
    )
    .await
    .expect_err("0 is below the declared minimum");
    assert_eq!(
        err,
        "invalid type for param 'max_actions_per_hour' in config.update_autonomy_settings: \
         expected unsigned integer >= 1, got 0"
    );
}
