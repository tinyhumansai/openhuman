use super::*;
use crate::core::runtime::DispatchCtx;

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

    let value = handle_get_data_paths(DispatchCtx::process_default(), params)
        .await
        .unwrap();
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

// ── local_ai saves drop a pending connect checkpoint (#8172) ─

// A plain save through `config.update_local_ai_settings`, even of the values
// the connect attempt wrote, must drop its checkpoint so a failed check
// cannot restore the older settings over that save.
#[tokio::test]
async fn config_local_ai_save_drops_a_pending_connect_checkpoint() {
    let _env_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard =
        crate::config::test_env::EnvVarGuard::set("OPENHUMAN_WORKSPACE", workspace.path());
    let ctx = DispatchCtx::ambient();
    let patch = crate::config::rpc::LocalAiSettingsPatch {
        provider: Some("lm_studio".into()),
        base_url: Some(Some("http://127.0.0.1:1234/v1".into())),
        ..Default::default()
    };
    let saved = crate::inference::ops::inference_update_local_settings(&ctx, patch, true)
        .await
        .expect("checkpointed save");
    let id = saved.value["checkpoint_id"]
        .as_str()
        .expect("checkpoint_id")
        .to_string();

    let params = Map::from_iter([
        ("provider".to_string(), Value::from("lm_studio")),
        (
            "base_url".to_string(),
            Value::from("http://127.0.0.1:1234/v1"),
        ),
    ]);
    inference::handle_update_local_ai_settings(ctx.clone(), params)
        .await
        .expect("config save");
    let local_ai = || async {
        serde_json::to_value(ctx.load_config().await.expect("config").local_ai).unwrap()
    };
    let after_save = local_ai().await;

    let rollback = crate::inference::ops::inference_rollback_local_settings(&ctx, &id)
        .await
        .expect("rollback");
    assert_eq!(rollback.value["restored"], Value::Bool(false));
    assert_eq!(local_ai().await, after_save);
}
