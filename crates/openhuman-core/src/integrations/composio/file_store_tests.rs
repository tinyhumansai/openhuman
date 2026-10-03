use super::*;
use std::collections::BTreeMap;

#[tokio::test]
async fn missing_file_loads_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let loaded: BTreeMap<String, u32> = load(&dir.path().join("absent.json")).await.unwrap();
    assert!(loaded.is_empty());
}

#[tokio::test]
async fn save_then_load_round_trips_and_creates_the_directory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("state.json");
    let mut value = BTreeMap::new();
    value.insert("gmail".to_string(), 3u32);
    save(&path, &value).await.unwrap();
    let loaded: BTreeMap<String, u32> = load(&path).await.unwrap();
    assert_eq!(loaded, value);
    assert!(!path.with_extension("json.tmp").exists());
}

#[tokio::test]
async fn corrupt_file_is_an_error_not_a_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    std::fs::write(&path, b"{not json").unwrap();
    let loaded: Result<BTreeMap<String, u32>, String> = load(&path).await;
    assert!(loaded.is_err());
}

#[test]
fn paths_live_under_the_workspace_integrations_dir() {
    let config = Config {
        workspace_dir: std::path::PathBuf::from("/ws"),
        ..Config::default()
    };
    assert_eq!(
        path(&config, IDENTITIES_FILE),
        std::path::PathBuf::from("/ws/integrations/composio_identities.json")
    );
    assert_eq!(
        path(&config, USER_SCOPES_FILE),
        std::path::PathBuf::from("/ws/integrations/composio_user_scopes.json")
    );
}
