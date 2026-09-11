use super::*;

// ── get_data_paths user scoping (#4950) ─────────────────────

// The Clear App Data flow passes `user_id` so the reset targets the
// signed-in user's `users/<id>` slice even though the active-user marker
// was already removed by the preceding sign-out. Verify the handler parses
// the param and scopes the resolved current dir accordingly.
#[tokio::test]
async fn handle_get_data_paths_scopes_to_explicit_user_id() {
    let mut params = Map::new();
    params.insert(
        "user_id".to_string(),
        Value::String("clear-me-4950".to_string()),
    );

    let value = handle_get_data_paths(params).await.unwrap();
    // `get_data_paths_for_user` attaches a log, so the outcome is wrapped as
    // `{ "result": <paths>, "logs": [...] }`.
    let current = value
        .pointer("/result/current_openhuman_dir")
        .and_then(Value::as_str)
        .expect("current_openhuman_dir present");
    assert!(
        current.replace('\\', "/").ends_with("users/clear-me-4950"),
        "current dir must be scoped to the explicit user id, got {current}"
    );
}

// ── platform slug validation (finding #6) ───────────────────
