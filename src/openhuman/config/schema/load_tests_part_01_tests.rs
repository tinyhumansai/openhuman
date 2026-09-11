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
fn read_active_user_checked_returns_none_when_no_file() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(read_active_user_id_checked(tmp.path()).unwrap().is_none());
}

#[test]
fn read_active_user_checked_returns_none_when_empty() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(ACTIVE_USER_STATE_FILE), "").unwrap();
    assert!(read_active_user_id_checked(tmp.path()).unwrap().is_none());
}

#[test]
fn read_active_user_checked_returns_id_when_present() {
    let tmp = tempfile::tempdir().unwrap();
    write_active_user_id(tmp.path(), "user-checked").unwrap();
    assert_eq!(
        read_active_user_id_checked(tmp.path()).unwrap(),
        Some("user-checked".to_string())
    );
}

#[test]
fn read_active_user_checked_errors_when_marker_unreadable() {
    // A marker that EXISTS but cannot be read as a file (here: a directory at
    // the marker path) must surface an error rather than being laundered into
    // "signed out". Otherwise a returning user is silently downgraded to the
    // pre-login profile and their data under users/<id> is orphaned (#5334).
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(ACTIVE_USER_STATE_FILE)).unwrap();
    assert!(read_active_user_id_checked(tmp.path()).is_err());

    // The best-effort wrapper still collapses to `None` for hint-only callers.
    assert!(read_active_user_id(tmp.path()).is_none());
}

#[tokio::test]
async fn resolve_dirs_errors_instead_of_pre_login_when_marker_unreadable() {
    // End-to-end: an unreadable active_user.toml must make directory
    // resolution FAIL rather than silently resolve to users/local — the path
    // that presents a signed-in user with a fresh, empty profile (#5334).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(ACTIVE_USER_STATE_FILE)).unwrap();

    let default_workspace = root.join("workspace");
    let result =
        resolve_runtime_config_dirs_with(root, &default_workspace, &MapEnv::default()).await;

    assert!(
        result.is_err(),
        "unreadable marker must not silently fall back to the pre-login profile"
    );
}

#[test]
fn user_openhuman_dir_builds_correct_path() {
    let root = PathBuf::from("/home/test/.openhuman");
    let dir = user_openhuman_dir(&root, "user-123");
    assert_eq!(dir, PathBuf::from("/home/test/.openhuman/users/user-123"));
}

#[tokio::test]
// Races on `OPENHUMAN_WORKSPACE` env var with other tests holding
// `TEST_ENV_LOCK` — passes in isolation, intermittently fails in parallel.
// Runs reliably with `--ignored --test-threads=1`. See PR #1524.
#[ignore = "flaky in parallel cargo test; OPENHUMAN_WORKSPACE env-var race — see PR #1524"]
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
    // APP_ENV is process-global and `default_root_dir_name()` reads it on every
    // call, so flipping it here races any concurrent test that resolves the root
    // openhuman dir (e.g. the credentials active-session guard, which silently
    // stops finding `active_user.toml` once the root becomes `.openhuman-staging`).
    // Take the same lock those tests hold.
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
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

#[test]
fn apply_env_overrides_picks_up_model() {
    let _g = env_lock();
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
    let _g = env_lock();
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
    let _g = env_lock();
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
fn apply_env_overrides_shell_hide_window_parses_truthy_falsy() {
    let _g = env_lock();
    clear_env(&["OPENHUMAN_SHELL_HIDE_WINDOW", "SHELL_HIDE_WINDOW"]);
    let mut cfg = Config::default();
    assert!(!cfg.shell.hide_window, "default should be off");

    unsafe {
        std::env::set_var("OPENHUMAN_SHELL_HIDE_WINDOW", "on");
    }
    cfg.apply_env_overrides();
    assert!(cfg.shell.hide_window);

    unsafe {
        std::env::set_var("OPENHUMAN_SHELL_HIDE_WINDOW", "false");
    }
    cfg.apply_env_overrides();
    assert!(!cfg.shell.hide_window);

    // The unprefixed alias `SHELL_HIDE_WINDOW` is honored too.
    unsafe {
        std::env::remove_var("OPENHUMAN_SHELL_HIDE_WINDOW");
        std::env::set_var("SHELL_HIDE_WINDOW", "on");
    }
    cfg.apply_env_overrides();
    assert!(cfg.shell.hide_window, "alias should set hide_window");

    // The namespaced var takes precedence over the alias when both are set.
    unsafe {
        std::env::set_var("OPENHUMAN_SHELL_HIDE_WINDOW", "off");
        std::env::set_var("SHELL_HIDE_WINDOW", "on");
    }
    cfg.apply_env_overrides();
    assert!(
        !cfg.shell.hide_window,
        "OPENHUMAN_-prefixed var should win over the alias"
    );

    // Unknown value leaves the field unchanged.
    cfg.shell.hide_window = true;
    unsafe {
        std::env::set_var("OPENHUMAN_SHELL_HIDE_WINDOW", "maybe");
        std::env::remove_var("SHELL_HIDE_WINDOW");
    }
    cfg.apply_env_overrides();
    assert!(cfg.shell.hide_window);

    // An empty / whitespace-only value is treated as unset: the field is left
    // unchanged and it must NOT hit the "unrecognized value" warn path (a bare
    // `OPENHUMAN_SHELL_HIDE_WINDOW=` in the environment previously warned on
    // every boot).
    cfg.shell.hide_window = true;
    unsafe {
        std::env::set_var("OPENHUMAN_SHELL_HIDE_WINDOW", "");
    }
    cfg.apply_env_overrides();
    assert!(
        cfg.shell.hide_window,
        "empty value should leave hide_window=true"
    );

    cfg.shell.hide_window = false;
    unsafe {
        std::env::set_var("OPENHUMAN_SHELL_HIDE_WINDOW", "   ");
    }
    cfg.apply_env_overrides();
    assert!(
        !cfg.shell.hide_window,
        "whitespace-only value should leave hide_window=false"
    );

    unsafe {
        std::env::remove_var("OPENHUMAN_SHELL_HIDE_WINDOW");
    }
}

#[test]
fn classify_shell_hide_window_distinguishes_unset_from_unrecognized() {
    use super::super::env_overlay::{classify_shell_hide_window, ShellHideWindowParse as P};
    // The key distinction the change relies on: an empty / whitespace-only value
    // is `Unset` (silent no-op), NOT `Unrecognized` (which warns on every boot).
    // Testing the classifier directly proves this — the field-unchanged assertion
    // above holds for BOTH branches and so can't catch a regression here.
    assert_eq!(classify_shell_hide_window(""), P::Unset);
    assert_eq!(classify_shell_hide_window("   "), P::Unset);
    assert_eq!(classify_shell_hide_window("\t"), P::Unset);
    assert_eq!(classify_shell_hide_window("on"), P::Set(true));
    assert_eq!(classify_shell_hide_window("FALSE"), P::Set(false));
    assert_eq!(classify_shell_hide_window("  yes  "), P::Set(true));
    assert_eq!(classify_shell_hide_window("maybe"), P::Unrecognized);
    assert_eq!(classify_shell_hide_window("2"), P::Unrecognized);
}

#[test]
fn apply_env_overrides_web_search_limits_only() {
    let _g = env_lock();
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
    let _g = env_lock();
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
fn apply_env_overrides_searxng_config() {
    let _g = env_lock();
    clear_env(&[
        "OPENHUMAN_SEARXNG_ENABLED",
        "SEARXNG_ENABLED",
        "OPENHUMAN_SEARXNG_BASE_URL",
        "SEARXNG_BASE_URL",
        "OPENHUMAN_SEARXNG_MAX_RESULTS",
        "SEARXNG_MAX_RESULTS",
        "OPENHUMAN_SEARXNG_DEFAULT_LANGUAGE",
        "SEARXNG_DEFAULT_LANGUAGE",
        "OPENHUMAN_SEARXNG_TIMEOUT_SECS",
        "OPENHUMAN_SEARXNG_TIMEOUT_SECONDS",
        "SEARXNG_TIMEOUT_SECS",
        "SEARXNG_TIMEOUT_SECONDS",
    ]);

    let mut cfg = Config::default();
    unsafe {
        std::env::set_var("OPENHUMAN_SEARXNG_ENABLED", "yes");
        std::env::set_var("OPENHUMAN_SEARXNG_BASE_URL", "http://127.0.0.1:8081");
        std::env::set_var("OPENHUMAN_SEARXNG_MAX_RESULTS", "25");
        std::env::set_var("OPENHUMAN_SEARXNG_DEFAULT_LANGUAGE", "zh-CN");
        std::env::set_var("OPENHUMAN_SEARXNG_TIMEOUT_SECONDS", "12");
    }

    cfg.apply_env_overrides();

    assert!(cfg.searxng.enabled);
    assert_eq!(cfg.searxng.base_url, "http://127.0.0.1:8081");
    assert_eq!(cfg.searxng.max_results, 25);
    assert_eq!(cfg.searxng.default_language, "zh-CN");
    assert_eq!(cfg.searxng.timeout_secs, 12);
    clear_env(&[
        "OPENHUMAN_SEARXNG_ENABLED",
        "OPENHUMAN_SEARXNG_BASE_URL",
        "OPENHUMAN_SEARXNG_MAX_RESULTS",
        "OPENHUMAN_SEARXNG_DEFAULT_LANGUAGE",
        "OPENHUMAN_SEARXNG_TIMEOUT_SECONDS",
    ]);
}

#[test]
fn searxng_timeout_seconds_alias_deserializes() {
    let cfg: crate::openhuman::config::SearxngConfig =
        toml::from_str(r#"timeout_seconds = 7"#).expect("deserialize searxng config");
    assert_eq!(cfg.timeout_secs, 7);
}

#[test]
fn apply_env_overrides_picks_up_sentry_dsn() {
    let _g = env_lock();
    clear_env(&["OPENHUMAN_CORE_SENTRY_DSN", "OPENHUMAN_SENTRY_DSN"]);
    let mut cfg = Config::default();
    unsafe {
        std::env::set_var("OPENHUMAN_SENTRY_DSN", "https://token@sentry.io/1");
    }
    cfg.apply_env_overrides();
    assert_eq!(
        cfg.observability.sentry_dsn.as_deref(),
        Some("https://token@sentry.io/1")
    );
    clear_env(&["OPENHUMAN_CORE_SENTRY_DSN", "OPENHUMAN_SENTRY_DSN"]);
}

#[test]
fn apply_env_overrides_prefers_core_sentry_dsn_when_both_set() {
    let _g = env_lock();
    clear_env(&["OPENHUMAN_CORE_SENTRY_DSN", "OPENHUMAN_SENTRY_DSN"]);
    let mut cfg = Config::default();
    unsafe {
        std::env::set_var("OPENHUMAN_SENTRY_DSN", "https://legacy@sentry.io/1");
        std::env::set_var("OPENHUMAN_CORE_SENTRY_DSN", "https://new@sentry.io/2");
    }
    cfg.apply_env_overrides();
    assert_eq!(
        cfg.observability.sentry_dsn.as_deref(),
        Some("https://new@sentry.io/2"),
        "namespaced var must win over the legacy unprefixed one"
    );
    clear_env(&["OPENHUMAN_CORE_SENTRY_DSN", "OPENHUMAN_SENTRY_DSN"]);
}

#[test]
fn apply_env_overrides_picks_up_core_sentry_dsn_alone() {
    let _g = env_lock();
    clear_env(&["OPENHUMAN_CORE_SENTRY_DSN", "OPENHUMAN_SENTRY_DSN"]);
    let mut cfg = Config::default();
    unsafe {
        std::env::set_var("OPENHUMAN_CORE_SENTRY_DSN", "https://token@sentry.io/3");
    }
    cfg.apply_env_overrides();
    assert_eq!(
        cfg.observability.sentry_dsn.as_deref(),
        Some("https://token@sentry.io/3")
    );
    clear_env(&["OPENHUMAN_CORE_SENTRY_DSN", "OPENHUMAN_SENTRY_DSN"]);
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
    let (oh_dir, ws_dir, source) = resolve_runtime_config_dirs_with(root, &default_workspace, &env)
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
    let (oh_dir, ws_dir, source) = resolve_runtime_config_dirs_with(root, &default_workspace, &env)
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
    let (oh_dir, ws_dir, source) = resolve_runtime_config_dirs_with(root, &default_workspace, &env)
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
    // `default_root_dir_name()` reads `OPENHUMAN_APP_ENV`; hold the shared env
    // lock so a concurrent staging-flip test can't turn the root into
    // `.openhuman-staging` mid-assertion.
    let _g = env_lock();
    let ws = PathBuf::from("/home/test/.openhuman/workspace");
    let (config_dir, workspace_dir) = resolve_config_dir_for_workspace(&ws);
    // Modern default layout: the config dir IS the `.openhuman` parent, resolved
    // by pure path structure (no filesystem probe), so the fake path resolves
    // even though it doesn't exist. The old, buggy code returned the doubled
    // `/home/test/.openhuman/.openhuman` here — which `Path::ends_with` still
    // matched against `".openhuman"` (whole-component match), making the old
    // assertion vacuous. `assert_eq!` against the exact parent catches that.
    assert_eq!(config_dir, PathBuf::from("/home/test/.openhuman"));
    assert_eq!(workspace_dir, ws);
}

#[test]
fn resolve_config_dir_for_workspace_modern_layout_does_not_double_openhuman() {
    // The #6079 repro: `OPENHUMAN_WORKSPACE=~/.openhuman/workspace` must resolve
    // to `~/.openhuman` (where `config.toml` lives), NOT the doubled
    // `~/.openhuman/.openhuman`, which never exists and silently reverts every
    // setting to schema defaults. This is a pure path-structure decision, so it
    // holds for a non-existent path.
    let _g = env_lock();
    let ws = PathBuf::from("/home/test/.openhuman/workspace");
    let (config_dir, workspace_dir) = resolve_config_dir_for_workspace(&ws);
    assert_eq!(config_dir, PathBuf::from("/home/test/.openhuman"));
    assert_ne!(
        config_dir,
        PathBuf::from("/home/test/.openhuman/.openhuman"),
        "must never return the doubled .openhuman/.openhuman path"
    );
    assert_eq!(workspace_dir, ws);
}

#[test]
fn resolve_config_dir_for_workspace_legacy_sibling_layout_is_preserved() {
    // Legacy layout: `<tmp>/project/workspace` with a real sibling
    // `<tmp>/project/.openhuman/config.toml` still resolves to the sibling
    // `.openhuman`. This arm probes the filesystem, so use a real temp dir.
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("project");
    let legacy = project.join(".openhuman");
    let ws = project.join("workspace");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(legacy.join("config.toml"), "").unwrap();

    let (config_dir, workspace_dir) = resolve_config_dir_for_workspace(&ws);
    assert_eq!(config_dir, legacy);
    assert_eq!(workspace_dir, ws);
}

#[test]
fn resolve_config_dir_for_workspace_workspace_basename_resolves_to_fresh_legacy_sibling() {
    // Legacy layout on a fresh volume: basename is `workspace`, the parent is
    // NOT the `.openhuman` config dir (so the modern arm does not fire), and the
    // sibling `.openhuman` does not exist yet. This must resolve to the sibling
    // `<proj>/.openhuman` — where `config::load` writes config for this layout —
    // NOT nest the workspace inside itself as `ws/workspace`. Regression guard
    // for the over-corrected `.exists()` gate (Codex P2).
    let _g = env_lock();
    let ws = PathBuf::from("/home/test/some-project/workspace");
    let (config_dir, workspace_dir) = resolve_config_dir_for_workspace(&ws);
    assert_eq!(
        config_dir,
        PathBuf::from("/home/test/some-project/.openhuman"),
        "a fresh legacy workspace must resolve to its sibling .openhuman, not nest itself"
    );
    assert_eq!(workspace_dir, ws);
    assert_ne!(
        config_dir,
        PathBuf::from("/home/test/some-project/.openhuman/.openhuman"),
        "must never return the doubled .openhuman/.openhuman path"
    );
}

#[test]
fn env_overlay_toggles_agent_tracing_capture_content() {
    // Serialize with the sibling env-overlay tests (TEST_ENV_LOCK note at the
    // top of the file) so a concurrent test's env mutation can't race in.
    let _g = env_lock();

    // ON by default since #4498 (`default_capture_content() == true` — traces
    // without content aren't actionable in Langfuse). This assertion was left
    // asserting the pre-#4498 `false` default and is corrected here.
    let mut cfg = Config::default();
    assert!(cfg.observability.agent_tracing.capture_content);

    // An explicit falsy env value turns it off.
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_AGENT_TRACING_CAPTURE_CONTENT", "off"),
    );
    assert!(!cfg.observability.agent_tracing.capture_content);

    // A truthy value turns it back on.
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_AGENT_TRACING_CAPTURE_CONTENT", "true"),
    );
    assert!(cfg.observability.agent_tracing.capture_content);
}

#[test]
fn env_overlay_runtime_pool_workers_and_enabled() {
    // Baseline: master switch on, both pools at the default worker count.
    let mut cfg = Config::default();
    assert!(cfg.runtime_pool.enabled, "master switch defaults on");
    assert_eq!(cfg.runtime_pool.node.max_workers, 2);
    assert_eq!(cfg.runtime_pool.python.max_workers, 2);

    // Valid overrides land; `enabled` parses via the shared bool parser.
    cfg.apply_env_overlay_with(
        &HashMapEnv::new()
            .with("OPENHUMAN_RUNTIME_POOL_ENABLED", "off")
            .with("OPENHUMAN_RUNTIME_POOL_NODE_MAX_WORKERS", "7")
            .with("OPENHUMAN_RUNTIME_POOL_PYTHON_MAX_WORKERS", "3"),
    );
    assert!(!cfg.runtime_pool.enabled, "explicit off disables the pool");
    assert_eq!(cfg.runtime_pool.node.max_workers, 7);
    assert_eq!(cfg.runtime_pool.python.max_workers, 3);

    // Unparseable worker counts are ignored (the warn arm) — the previously
    // applied values survive rather than resetting to a default or zero.
    cfg.apply_env_overlay_with(
        &HashMapEnv::new()
            .with("OPENHUMAN_RUNTIME_POOL_NODE_MAX_WORKERS", "not-a-number")
            .with("OPENHUMAN_RUNTIME_POOL_PYTHON_MAX_WORKERS", ""),
    );
    assert_eq!(
        cfg.runtime_pool.node.max_workers, 7,
        "invalid node worker count keeps the prior value"
    );
    assert_eq!(
        cfg.runtime_pool.python.max_workers, 3,
        "empty python worker count keeps the prior value"
    );
}
