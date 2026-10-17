use super::*;

fn config_in(dir: &Path) -> Config {
    let mut config = Config::default();
    config.config_path = dir.join("config.toml");
    config.workspace_dir = dir.join("workspace");
    config.secrets.encrypt = false;
    config
}

fn write_legacy(config: &Config, kind: IndexKind, content: &str) {
    let path = legacy_path(&config.workspace_dir, kind);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[test]
fn a_saved_document_loads_back_and_kinds_are_separate() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    assert_eq!(load(&config, IndexKind::Slang).unwrap(), None);
    save(&config, IndexKind::Slang, "{\"terms\":1}".into()).unwrap();
    assert_eq!(
        load(&config, IndexKind::Slang).unwrap().as_deref(),
        Some("{\"terms\":1}")
    );
    assert_eq!(load(&config, IndexKind::Memes).unwrap(), None);
    assert_eq!(load(&config, IndexKind::Remixed).unwrap(), None);
    save(&config, IndexKind::Slang, "{\"terms\":2}".into()).unwrap();
    assert_eq!(
        load(&config, IndexKind::Slang).unwrap().as_deref(),
        Some("{\"terms\":2}")
    );
}

#[test]
fn the_legacy_remixed_json_is_adopted_once_and_retired() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    write_legacy(&config, IndexKind::Remixed, "[\"m1\",\"m2\"]");

    assert_eq!(
        load(&config, IndexKind::Remixed).unwrap().as_deref(),
        Some("[\"m1\",\"m2\"]")
    );
    let legacy = legacy_path(&config.workspace_dir, IndexKind::Remixed);
    assert!(!legacy.exists(), "the old file is retired");
    assert!(legacy.with_extension("json.migrated").is_file());

    // The module's later saves are the truth; a stray file is not re-adopted.
    save(&config, IndexKind::Remixed, "[\"m1\",\"m2\",\"m3\"]".into()).unwrap();
    write_legacy(&config, IndexKind::Remixed, "[\"stale\"]");
    assert_eq!(
        load(&config, IndexKind::Remixed).unwrap().as_deref(),
        Some("[\"m1\",\"m2\",\"m3\"]")
    );
}

#[test]
fn every_legacy_index_file_is_adopted() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    write_legacy(&config, IndexKind::Slang, "{\"slang\":true}");
    write_legacy(&config, IndexKind::Memes, "{\"memes\":true}");
    assert_eq!(
        load(&config, IndexKind::Slang).unwrap().as_deref(),
        Some("{\"slang\":true}")
    );
    assert_eq!(
        load(&config, IndexKind::Memes).unwrap().as_deref(),
        Some("{\"memes\":true}")
    );
}

#[test]
fn a_stored_document_wins_over_a_legacy_file() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    save(&config, IndexKind::Memes, "{\"new\":1}".into()).unwrap();
    write_legacy(&config, IndexKind::Memes, "{\"old\":1}");
    assert_eq!(
        load(&config, IndexKind::Memes).unwrap().as_deref(),
        Some("{\"new\":1}")
    );
    assert!(legacy_path(&config.workspace_dir, IndexKind::Memes).exists());
}

#[cfg(unix)]
#[test]
fn an_unreadable_legacy_file_is_an_error_not_an_empty_start() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    write_legacy(&config, IndexKind::Remixed, "[\"keep\"]");
    let path = legacy_path(&config.workspace_dir, IndexKind::Remixed);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_to_string(&path).is_ok() {
        // Running as a user who can read anything (root): nothing to assert.
        return;
    }
    assert!(load(&config, IndexKind::Remixed).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        load(&config, IndexKind::Remixed).unwrap().as_deref(),
        Some("[\"keep\"]"),
        "the state is still there once the file can be read"
    );
}

#[test]
fn an_invalid_utf8_legacy_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    let path = legacy_path(&config.workspace_dir, IndexKind::Slang);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
    assert!(load(&config, IndexKind::Slang).is_err());
    assert!(path.exists(), "an unreadable file is left where it is");
}
