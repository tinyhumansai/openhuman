use super::*;

#[test]
fn normalize_key_replaces_non_alnum() {
    let key = normalize_key("hello/world", 0);
    assert_eq!(key, "hello_world");
}

#[test]
fn parse_category_defaults_to_core() {
    assert_eq!(
        parse_category("unknown"),
        MemoryCategory::Custom("unknown".to_string())
    );
}

#[test]
fn resolve_hermes_workspace_returns_override_when_provided() {
    let custom = PathBuf::from("/custom/hermes");
    let result = resolve_hermes_workspace(Some(custom.clone())).unwrap();
    assert_eq!(result, custom);
}

#[test]
fn resolve_hermes_workspace_defaults_to_home_dot_hermes() {
    let result = resolve_hermes_workspace(None).unwrap();
    #[cfg(windows)]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            assert_eq!(result, PathBuf::from(local_app_data).join("hermes"));
            return;
        }
    }
    let home = directories::UserDirs::new().unwrap();
    assert_eq!(result, home.home_dir().join(".hermes"));
}
