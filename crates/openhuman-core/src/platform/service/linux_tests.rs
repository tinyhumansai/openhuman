use super::*;
use crate::config::Config;

#[test]
fn linux_service_file_is_in_systemd_user_unit_dir() {
    let config = Config::default();
    let path = linux_service_file(&config).unwrap();
    let expected = common::systemd_user_unit_dir(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
    )
    .unwrap()
    .join("openhuman.service");
    assert_eq!(path, expected);
    assert!(path.ends_with("systemd/user/openhuman.service"));
}

#[test]
fn linux_service_file_is_not_under_openhuman_config_dir() {
    let mut config = Config::default();
    config.config_path = PathBuf::from("/tmp/oh-systemd-test/.openhuman/config.toml");
    let path = linux_service_file(&config).unwrap();
    assert!(!path.starts_with("/tmp/oh-systemd-test/.openhuman"));
}

#[test]
fn legacy_linux_service_file_uses_old_config_dir_formula() {
    let mut config = Config::default();
    config.config_path = PathBuf::from("/tmp/oh-systemd-test/.openhuman/config.toml");
    assert_eq!(
        legacy_linux_service_file(&config),
        PathBuf::from("/tmp/oh-systemd-test/.openhuman/.config/systemd/user/openhuman.service")
    );
}

#[test]
fn remove_legacy_service_file_deletes_only_the_stale_unit() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join(".openhuman").join("config.toml");
    let legacy = legacy_linux_service_file(&config);
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, "[Unit]\n").unwrap();
    let sibling = legacy.with_file_name("other.service");
    fs::write(&sibling, "[Unit]\n").unwrap();

    remove_legacy_service_file(&config, &tmp.path().join("real").join("openhuman.service"));

    assert!(!legacy.exists());
    assert!(sibling.exists());
}

#[test]
fn remove_legacy_service_file_keeps_unit_when_paths_coincide() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    let legacy = legacy_linux_service_file(&config);
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, "[Unit]\n").unwrap();

    remove_legacy_service_file(&config, &legacy);

    assert!(legacy.exists());
}

#[test]
fn remove_legacy_service_file_keeps_unit_reached_through_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let unit_dir = home.join(".config").join("systemd").join("user");
    fs::create_dir_all(&unit_dir).unwrap();
    let current = unit_dir.join("openhuman.service");
    fs::write(&current, "[Unit]\n").unwrap();
    // Config dir is a symlink to the home dir, so the legacy path aliases `current`.
    let config_dir = tmp.path().join("oh-config");
    std::os::unix::fs::symlink(&home, &config_dir).unwrap();
    let mut config = Config::default();
    config.config_path = config_dir.join("config.toml");

    remove_legacy_service_file(&config, &current);

    assert!(current.exists());
}
