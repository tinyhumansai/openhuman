use super::*;

#[test]
fn xml_escape_replaces_entities() {
    let raw = "<tag>\"&'";
    let escaped = xml_escape(raw);
    assert!(escaped.contains("&lt;tag&gt;"));
    assert!(escaped.contains("&quot;"));
    assert!(escaped.contains("&amp;"));
    assert!(escaped.contains("&apos;"));
}

#[test]
fn systemd_user_unit_dir_prefers_absolute_xdg_config_home() {
    // `/xdg/config` is not absolute on Windows (no drive), so build the fixture
    // from a path that is absolute on every OS.
    let xdg = std::env::temp_dir().join("xdg-config");
    assert!(xdg.is_absolute());
    let dir = systemd_user_unit_dir(
        Some(xdg.as_os_str()),
        Some(std::path::Path::new("/home/alice")),
    );
    assert_eq!(dir, Some(xdg.join("systemd").join("user")));
}

#[test]
fn systemd_user_unit_dir_ignores_empty_xdg_config_home() {
    let dir = systemd_user_unit_dir(
        Some(std::ffi::OsStr::new("")),
        Some(std::path::Path::new("/home/alice")),
    );
    assert_eq!(dir, Some(PathBuf::from("/home/alice/.config/systemd/user")));
}

#[test]
fn systemd_user_unit_dir_ignores_relative_xdg_config_home() {
    let dir = systemd_user_unit_dir(
        Some(std::ffi::OsStr::new("relative/config")),
        Some(std::path::Path::new("/home/alice")),
    );
    assert_eq!(dir, Some(PathBuf::from("/home/alice/.config/systemd/user")));
}

#[test]
fn systemd_user_unit_dir_falls_back_to_home_without_xdg() {
    let dir = systemd_user_unit_dir(None, Some(std::path::Path::new("/home/alice")));
    assert_eq!(dir, Some(PathBuf::from("/home/alice/.config/systemd/user")));
}

#[test]
fn systemd_user_unit_dir_is_none_without_xdg_or_home() {
    assert_eq!(systemd_user_unit_dir(None, None), None);
    assert_eq!(
        systemd_user_unit_dir(Some(std::ffi::OsStr::new("rel")), None),
        None
    );
    assert_eq!(
        systemd_user_unit_dir(None, Some(std::path::Path::new(""))),
        None
    );
}

#[test]
fn systemd_user_unit_dir_is_not_under_openhuman_config_dir() {
    // Regression: the unit used to land in `~/.openhuman/.config/systemd/user`,
    // which systemd never reads.
    let dir = systemd_user_unit_dir(None, Some(std::path::Path::new("/home/alice"))).unwrap();
    assert!(!dir.starts_with("/home/alice/.openhuman"));
}
