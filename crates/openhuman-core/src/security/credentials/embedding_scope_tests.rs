use super::*;

/// Configured embedding credentials follow the caller's config directory and
/// encryption setting, even when the process has a different default scope.
#[test]
fn configured_scope_uses_config_directory_and_encryption() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = temp
        .path()
        .join("users")
        .join("account")
        .join("config.toml");
    config.secrets.encrypt = false;

    let (state_dir, encrypt) = credential_scope(Some(&config));

    assert_eq!(state_dir, config.config_path.parent().unwrap());
    assert!(!encrypt);
}

/// Call sites without a config retain the encrypted process-default policy.
#[test]
fn unconfigured_scope_defaults_to_encrypted_credentials() {
    let (_, encrypt) = credential_scope(None);

    assert!(encrypt);
}

/// The keyless credential scope must land in the active user's directory,
/// never the root where there is no user auth profile.
#[test]
fn default_scope_is_the_active_user_dir_not_the_root() {
    let root = Path::new("/tmp/openhuman-root");

    let resolved = user_scoped_state_dir(root, Some("user-abc123"));

    assert_eq!(resolved, root.join("users").join("user-abc123"));
    assert_ne!(resolved, root);
}

/// Before sign-in, credential resolution uses the pre-login profile directory.
#[test]
fn default_scope_falls_back_to_the_pre_login_user_dir() {
    let root = Path::new("/tmp/openhuman-root");

    let resolved = user_scoped_state_dir(root, None);

    assert_eq!(
        resolved,
        root.join("users").join(crate::config::PRE_LOGIN_USER_ID)
    );
}

/// Legacy workspace overrides resolve to their sibling config directory,
/// matching the location that stores auth profiles.
#[test]
fn env_workspace_scope_is_the_config_dir_not_the_raw_workspace() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let sibling_config = root.path().join(".openhuman");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&sibling_config).unwrap();

    let resolved = env_workspace_state_dir(&workspace);

    assert_eq!(resolved, sibling_config);
    assert_ne!(resolved, workspace);
}

/// The modern `~/.openhuman/workspace` layout keeps config and credentials in
/// the parent `.openhuman` directory too.
#[test]
fn modern_workspace_scope_is_the_parent_openhuman_dir() {
    let root = tempfile::tempdir().unwrap();
    let config_dir = root.path().join(".openhuman");
    let workspace = config_dir.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let resolved = env_workspace_state_dir(&workspace);

    assert_eq!(resolved, config_dir);
}
