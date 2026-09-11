use super::*;

// This file used to register a stub `ComposioProvider` under the engine's
// provider registry so the third test below could prove
// `connection_identity` short-circuits *before* calling `fetch_user_profile`
// when the toolkit has no active connection. tinymemory v1.13.4 deleted
// `ComposioProvider`/the registry outright with no replacement (see
// `identity.rs`'s module docs) — `connection_identity` now calls the
// `tinyconnectors` module's `GetUserProfile` directly rather than a
// registered provider trait object, so there is no stub to register. None of
// the three tests below ever reached that call anyway (all three assert the
// short-circuit paths — empty toolkit, and "not in active integrations" — so
// they are unaffected by the migration and are unchanged except for dropping
// the now-nonexistent stub setup.

fn fresh_config_in_workspace(tmp: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.config_path = tmp.join("config.toml");
    config.workspace_dir = tmp.join("workspace");
    config.secrets.encrypt = false;
    config
}

#[tokio::test]
async fn empty_toolkit_short_circuits_to_none() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = fresh_config_in_workspace(tmp.path());
    assert!(connection_identity(&config, "").await.is_none());
    assert!(connection_identity(&config, "   ").await.is_none());
}

#[tokio::test]
async fn unknown_toolkit_returns_none_without_provider_call() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = fresh_config_in_workspace(tmp.path());
    // Toolkit slug with no active connection.
    assert!(connection_identity(&config, "not-a-real-toolkit-xyz")
        .await
        .is_none());
}

#[tokio::test]
async fn no_active_connection_short_circuits_before_provider_call() {
    // No connections exist for the toolkit → identity helper should return
    // None without calling `GetUserProfile`.
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = fresh_config_in_workspace(tmp.path());
    // Default config has no Composio auth → fetch_connected_integrations
    // returns an empty vec, so the toolkit is not "in active".
    let username = connection_identity(&config, "stub-no-active").await;
    assert!(username.is_none(), "must short-circuit when not connected");
}
