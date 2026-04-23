use super::*;

#[test]
fn read_active_user_returns_none_when_no_file() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(read_active_user_id(tmp.path()).is_none());
}

#[test]
fn read_active_user_returns_none_when_empty() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(ACTIVE_USER_STATE_FILE), "").unwrap();
    assert!(read_active_user_id(tmp.path()).is_none());
}

#[test]
fn read_active_user_returns_id_when_present() {
    let tmp = tempfile::tempdir().unwrap();
    write_active_user_id(tmp.path(), "user-789").unwrap();
    assert_eq!(
        read_active_user_id(tmp.path()),
        Some("user-789".to_string())
    );
}

#[test]
fn write_and_clear_active_user_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();

    write_active_user_id(tmp.path(), "u-abc").unwrap();
    assert_eq!(read_active_user_id(tmp.path()), Some("u-abc".to_string()));

    clear_active_user(tmp.path()).unwrap();
    assert!(read_active_user_id(tmp.path()).is_none());
}

#[test]
fn user_openhuman_dir_builds_correct_path() {
    let root = PathBuf::from("/home/test/.openhuman");
    let dir = user_openhuman_dir(&root, "user-123");
    assert_eq!(dir, PathBuf::from("/home/test/.openhuman/users/user-123"));
}

#[tokio::test]
async fn resolve_dirs_uses_active_user_when_present() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let default_workspace = root.join("workspace");

    // No active user → falls back to the pre-login user directory so
    // memory/state/config are still encapsulated under users/.
    let (oh_dir, ws_dir, source) = resolve_runtime_config_dirs(root, &default_workspace)
        .await
        .unwrap();
    let expected_pre_login_dir = root.join("users").join(PRE_LOGIN_USER_ID);
    assert_eq!(oh_dir, expected_pre_login_dir);
    assert_eq!(ws_dir, expected_pre_login_dir.join("workspace"));
    assert_eq!(source, ConfigResolutionSource::DefaultConfigDir);

    // With active user → scopes to user dir.
    write_active_user_id(root, "u-test").unwrap();
    let (oh_dir, ws_dir, source) = resolve_runtime_config_dirs(root, &default_workspace)
        .await
        .unwrap();
    let expected_user_dir = root.join("users").join("u-test");
    assert_eq!(oh_dir, expected_user_dir);
    assert_eq!(ws_dir, expected_user_dir.join("workspace"));
    assert_eq!(source, ConfigResolutionSource::ActiveUser);
}

#[test]
fn pre_login_user_dir_is_under_users_tree() {
    let root = PathBuf::from("/home/test/.openhuman");
    let dir = pre_login_user_dir(&root);
    assert_eq!(
        dir,
        PathBuf::from("/home/test/.openhuman/users").join(PRE_LOGIN_USER_ID)
    );
}

#[test]
fn default_root_dir_name_uses_staging_suffix_for_staging_env() {
    let prior = std::env::var(crate::api::config::APP_ENV_VAR).ok();

    std::env::set_var(crate::api::config::APP_ENV_VAR, "staging");
    assert!(crate::api::config::is_staging_app_env(Some("staging")));
    assert_eq!(default_root_dir_name(), ".openhuman-staging");

    std::env::set_var(crate::api::config::APP_ENV_VAR, "production");
    assert_eq!(default_root_dir_name(), ".openhuman");

    match prior {
        Some(value) => std::env::set_var(crate::api::config::APP_ENV_VAR, value),
        None => std::env::remove_var(crate::api::config::APP_ENV_VAR),
    }
}

// ── apply_env_overrides ────────────────────────────────────────

use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;

fn clear_env(keys: &[&str]) {
    for key in keys {
        unsafe {
            std::env::remove_var(key);
        }
    }
}

#[test]
fn apply_env_overrides_picks_up_model() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_env(&["OPENHUMAN_MODEL", "MODEL"]);
    unsafe {
        std::env::set_var("OPENHUMAN_MODEL", "gpt-5");
    }
    let mut cfg = Config::default();
    cfg.apply_env_overrides();
    assert_eq!(cfg.default_model.as_deref(), Some("gpt-5"));
    unsafe {
        std::env::remove_var("OPENHUMAN_MODEL");
    }
}

#[test]
fn apply_env_overrides_validates_temperature_range() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_env(&["OPENHUMAN_TEMPERATURE"]);
    let mut cfg = Config::default();
    cfg.default_temperature = 0.5;
    unsafe {
        std::env::set_var("OPENHUMAN_TEMPERATURE", "1.2");
    }
    cfg.apply_env_overrides();
    assert!((cfg.default_temperature - 1.2).abs() < f64::EPSILON);

    // Out of range — should be ignored.
    unsafe {
        std::env::set_var("OPENHUMAN_TEMPERATURE", "5");
    }
    cfg.apply_env_overrides();
    assert!((cfg.default_temperature - 1.2).abs() < f64::EPSILON);

    // Garbage value — ignored.
    unsafe {
        std::env::set_var("OPENHUMAN_TEMPERATURE", "not-a-number");
    }
    cfg.apply_env_overrides();
    assert!((cfg.default_temperature - 1.2).abs() < f64::EPSILON);
    unsafe {
        std::env::remove_var("OPENHUMAN_TEMPERATURE");
    }
}

#[test]
fn apply_env_overrides_reasoning_enabled_parses_truthy_falsy() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_env(&["OPENHUMAN_REASONING_ENABLED", "REASONING_ENABLED"]);
    let mut cfg = Config::default();
    cfg.runtime.reasoning_enabled = None;

    unsafe {
        std::env::set_var("OPENHUMAN_REASONING_ENABLED", "yes");
    }
    cfg.apply_env_overrides();
    assert_eq!(cfg.runtime.reasoning_enabled, Some(true));

    unsafe {
        std::env::set_var("OPENHUMAN_REASONING_ENABLED", "off");
    }
    cfg.apply_env_overrides();
    assert_eq!(cfg.runtime.reasoning_enabled, Some(false));

    // Unknown value — leaves field unchanged.
    unsafe {
        std::env::set_var("OPENHUMAN_REASONING_ENABLED", "maybe");
    }
    cfg.apply_env_overrides();
    assert_eq!(cfg.runtime.reasoning_enabled, Some(false));
    unsafe {
        std::env::remove_var("OPENHUMAN_REASONING_ENABLED");
    }
}

#[test]
fn apply_env_overrides_web_search_limits_only() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_env(&[
        "OPENHUMAN_WEB_SEARCH_MAX_RESULTS",
        "WEB_SEARCH_MAX_RESULTS",
        "OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS",
        "WEB_SEARCH_TIMEOUT_SECS",
    ]);
    let mut cfg = Config::default();
    unsafe {
        std::env::set_var("OPENHUMAN_WEB_SEARCH_MAX_RESULTS", "5");
        std::env::set_var("OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS", "20");
    }
    cfg.apply_env_overrides();
    assert_eq!(cfg.web_search.max_results, 5);
    assert_eq!(cfg.web_search.timeout_secs, 20);
    clear_env(&[
        "OPENHUMAN_WEB_SEARCH_MAX_RESULTS",
        "OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS",
    ]);
}

#[test]
fn apply_env_overrides_web_search_max_results_and_timeout_clamped() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_env(&[
        "OPENHUMAN_WEB_SEARCH_MAX_RESULTS",
        "WEB_SEARCH_MAX_RESULTS",
        "OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS",
        "WEB_SEARCH_TIMEOUT_SECS",
    ]);
    let mut cfg = Config::default();
    cfg.web_search.max_results = 3;
    cfg.web_search.timeout_secs = 10;

    // Valid values apply.
    unsafe {
        std::env::set_var("OPENHUMAN_WEB_SEARCH_MAX_RESULTS", "5");
        std::env::set_var("OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS", "20");
    }
    cfg.apply_env_overrides();
    assert_eq!(cfg.web_search.max_results, 5);
    assert_eq!(cfg.web_search.timeout_secs, 20);

    // Out-of-range (>10 for max_results, 0 for timeout) — ignored.
    unsafe {
        std::env::set_var("OPENHUMAN_WEB_SEARCH_MAX_RESULTS", "999");
        std::env::set_var("OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS", "0");
    }
    cfg.apply_env_overrides();
    assert_eq!(
        cfg.web_search.max_results, 5,
        "out-of-range must be ignored"
    );
    assert_eq!(cfg.web_search.timeout_secs, 20);
    clear_env(&[
        "OPENHUMAN_WEB_SEARCH_MAX_RESULTS",
        "OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS",
    ]);
}

#[test]
fn apply_env_overrides_picks_up_sentry_dsn() {
    let _g = ENV_LOCK.lock().unwrap();
    clear_env(&["OPENHUMAN_SENTRY_DSN"]);
    let mut cfg = Config::default();
    unsafe {
        std::env::set_var("OPENHUMAN_SENTRY_DSN", "https://token@sentry.io/1");
    }
    cfg.apply_env_overrides();
    assert_eq!(
        cfg.observability.sentry_dsn.as_deref(),
        Some("https://token@sentry.io/1")
    );
    clear_env(&["OPENHUMAN_SENTRY_DSN"]);
}

// ── EnvLookup seam for resolve_runtime_config_dirs ─────────────

#[derive(Default)]
struct MapEnv(std::collections::HashMap<String, String>);

impl MapEnv {
    fn with(mut self, k: &str, v: &str) -> Self {
        self.0.insert(k.to_string(), v.to_string());
        self
    }
}

impl EnvLookup for MapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

#[tokio::test]
async fn env_workspace_override_wins_via_seam() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // Active user would otherwise win — confirm env override takes precedence.
    write_active_user_id(root, "u-active").unwrap();

    let ws_root = tempfile::tempdir().unwrap();
    let ws_path = ws_root.path().join("my-workspace");
    let env = MapEnv::default().with("OPENHUMAN_WORKSPACE", ws_path.to_str().unwrap());

    let default_workspace = root.join("workspace");
    let (oh_dir, ws_dir, source) =
        resolve_runtime_config_dirs_with_env(root, &default_workspace, &env)
            .await
            .unwrap();

    let (expected_oh, expected_ws) = resolve_config_dir_for_workspace(&ws_path);
    assert_eq!(source, ConfigResolutionSource::EnvWorkspace);
    assert_eq!(oh_dir, expected_oh);
    assert_eq!(ws_dir, expected_ws);
}

#[tokio::test]
async fn empty_env_workspace_falls_through_to_active_user() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_active_user_id(root, "u-fallthrough").unwrap();
    let env = MapEnv::default().with("OPENHUMAN_WORKSPACE", "");

    let default_workspace = root.join("workspace");
    let (oh_dir, ws_dir, source) =
        resolve_runtime_config_dirs_with_env(root, &default_workspace, &env)
            .await
            .unwrap();

    let expected = root.join("users").join("u-fallthrough");
    assert_eq!(source, ConfigResolutionSource::ActiveUser);
    assert_eq!(oh_dir, expected);
    assert_eq!(ws_dir, expected.join("workspace"));
}

#[tokio::test]
async fn missing_env_workspace_uses_pre_login_default() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let env = MapEnv::default(); // no OPENHUMAN_WORKSPACE, no active user

    let default_workspace = root.join("workspace");
    let (oh_dir, ws_dir, source) =
        resolve_runtime_config_dirs_with_env(root, &default_workspace, &env)
            .await
            .unwrap();

    let expected = root.join("users").join(PRE_LOGIN_USER_ID);
    assert_eq!(source, ConfigResolutionSource::DefaultConfigDir);
    assert_eq!(oh_dir, expected);
    assert_eq!(ws_dir, expected.join("workspace"));
}

// ── resolve_config_dir_for_workspace ───────────────────────────

#[test]
fn resolve_config_dir_for_workspace_returns_parent_and_workspace() {
    let ws = PathBuf::from("/home/test/.openhuman/workspace");
    let (config_dir, workspace_dir) = resolve_config_dir_for_workspace(&ws);
    // Config dir is the parent of workspace.
    assert!(
        config_dir.ends_with(".openhuman") || config_dir == PathBuf::from("/home/test/.openhuman")
    );
    assert!(workspace_dir.ends_with("workspace"));
}
