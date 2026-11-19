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

// ── update_computer_settings captcha provider ────────────────

#[tokio::test]
async fn update_computer_settings_saves_the_captcha_provider() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.config_path = dir.path().join("config.toml");
    config.workspace_dir = dir.path().join("workspace");
    let ctx = DispatchCtx::new(crate::core::runtime::CoreContext::for_inspection(config));
    let mut params = Map::new();
    params.insert("captcha_solver".into(), Value::Bool(true));
    params.insert("captcha_provider".into(), Value::String("surplus".into()));
    handle_update_computer_settings(ctx, params).await.unwrap();
    let saved = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(saved.contains("captcha_provider = \"surplus\""), "{saved}");

    let schema = super::super::schema_defs::schemas("update_computer_settings");
    let field = schema
        .inputs
        .iter()
        .find(|field| field.name == "captcha_provider")
        .expect("captcha_provider schema input");
    assert!(field.comment.contains("type_safe") && field.comment.contains("surplus"));
}
