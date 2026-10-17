use super::*;

fn config_with(url: Option<&str>) -> Config {
    let mut config = Config::default();
    config.storage.url = url.map(str::to_string);
    config
}

#[test]
fn nothing_configured_is_the_default() {
    assert_eq!(mode_from(None, &config_with(None)), StorageMode::Default);
    assert_eq!(
        mode_from(Some("  ".into()), &config_with(Some(""))),
        StorageMode::Default
    );
}

#[test]
fn the_environment_beats_the_file_beats_the_default() {
    let config = config_with(Some("sqlite:/from/config"));
    assert_eq!(
        mode_from(Some("memory".into()), &config),
        StorageMode::Url("memory".into())
    );
    assert_eq!(
        mode_from(None, &config),
        StorageMode::Url("sqlite:/from/config".into())
    );
    assert_eq!(
        mode_from(Some(" ".into()), &config_with(Some(" memory "))),
        StorageMode::Url("memory".into()),
        "a blank override falls through to the file"
    );
}

#[test]
fn the_retired_classic_opt_out_reads_as_unset() {
    assert_eq!(
        mode_from(Some("classic".into()), &config_with(None)),
        StorageMode::Default
    );
    assert_eq!(
        mode_from(None, &config_with(Some("Legacy"))),
        StorageMode::Default
    );
    assert_eq!(
        mode_from(Some("classic".into()), &config_with(Some("memory"))),
        StorageMode::Url("memory".into()),
        "a retired value in the environment falls through to the file"
    );
}

#[test]
fn the_default_url_is_sqlite_under_the_workspace_storage_directory() {
    let workspace = Path::new("/work/space");
    assert_eq!(default_url(workspace), "sqlite:/work/space/storage");
    assert_eq!(
        effective_url(&StorageMode::Default, workspace),
        "sqlite:/work/space/storage"
    );
    assert_eq!(
        effective_url(&StorageMode::Url("memory".into()), workspace),
        "memory"
    );
}

#[test]
fn the_default_differs_per_workspace() {
    assert_ne!(
        default_url(Path::new("/users/a/workspace")),
        default_url(Path::new("/users/b/workspace"))
    );
}

#[tokio::test]
async fn the_default_url_opens_in_directory_mode() {
    let dir = tempfile::tempdir().unwrap();
    let backend = crate::storage::open(&default_url(dir.path()))
        .await
        .unwrap();
    assert_eq!(backend.driver(), "sqlite");
    // Directory mode: a named database is its own file in the directory.
    backend.database("approvals").unwrap();
    assert!(dir.path().join(DEFAULT_DIR).join("approvals.db").exists());
}
