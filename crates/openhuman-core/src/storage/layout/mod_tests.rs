use super::*;
use crate::config::Config;

#[test]
fn a_profile_config_yields_its_target() {
    let root = std::path::Path::new("/r/.openhuman");
    let config = Config {
        config_path: root.join("users").join("abc").join("config.toml"),
        ..Config::default()
    };
    let target = Target::from_config(&config).unwrap();
    assert_eq!(target, Target::new(root, "abc"));
    assert_eq!(target.workspace(), root.join("users/abc/workspace"));
    assert_eq!(target.files(), root.join("users/abc/files"));
    assert_eq!(target.storage_db(), root.join("users/abc/storage.db"));
    assert_eq!(target.backups(), root.join("backups"));
}

#[test]
fn a_config_outside_users_has_no_layout() {
    let config = Config {
        config_path: std::path::PathBuf::from("/somewhere/else/config.toml"),
        ..Config::default()
    };
    assert!(Target::from_config(&config).is_none());
}
