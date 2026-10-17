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
    // Directory mode: the directory holds the database file.
    assert!(dir.path().join(DEFAULT_DIR).exists());
}

fn v2_layout(root: &Path) -> crate::config::schema::ProfileLayout {
    crate::config::schema::ProfileLayout::resolve(root, "alice")
}

#[test]
fn the_default_url_per_layout_version() {
    use crate::config::schema::profile_layout::{LayoutVersion, ProfileLayout};
    let dir = std::path::PathBuf::from("/srv/oh/users/alice");
    let v1 = ProfileLayout::with_version(dir.clone(), LayoutVersion::V1);
    let v2 = ProfileLayout::with_version(dir, LayoutVersion::V2);
    assert_eq!(
        default_url_for(&v1),
        "sqlite:/srv/oh/users/alice/workspace/storage"
    );
    assert_eq!(
        default_url_for(&v2),
        "sqlite:/srv/oh/users/alice/storage.db"
    );
}

#[test]
fn a_v2_workspace_defaults_to_its_profiles_storage_db_and_a_v1_one_is_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = v2_layout(tmp.path());
    assert_eq!(
        default_url(&layout.workspace_dir),
        format!("sqlite:{}", layout.dir.join("storage.db").display())
    );
    // A `files` directory with no marker beside it is just a V1 workspace.
    assert_eq!(
        default_url(Path::new("/plain/files")),
        "sqlite:/plain/files/storage"
    );
}

#[test]
fn an_explicit_url_wins_over_the_v2_default() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = v2_layout(tmp.path());
    let mut config = config_with(Some("memory"));
    config.workspace_dir = layout.workspace_dir.clone();
    assert_eq!(
        crate::storage::url_from(None, &config).as_deref(),
        Some("memory")
    );
    assert_eq!(
        crate::storage::url_from(Some("sqlite:/elsewhere.db".into()), &config).as_deref(),
        Some("sqlite:/elsewhere.db")
    );
    config.storage.url = None;
    assert_eq!(
        crate::storage::url_from(None, &config),
        Some(default_url_for(&layout))
    );
}

#[tokio::test]
async fn the_v2_default_opens_one_storage_db_file_beside_config_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = v2_layout(tmp.path());
    prepare_default_dir(&layout.workspace_dir).unwrap();
    let backend = crate::storage::open(&default_url(&layout.workspace_dir))
        .await
        .unwrap();
    assert_eq!(backend.driver(), "sqlite");
    assert!(
        layout.storage_db.is_file(),
        "{} should be a database file",
        layout.storage_db.display()
    );
    assert!(!layout.workspace_dir.join(DEFAULT_DIR).exists());
}
