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
    let _g = ENV_LOCK.lock().unwrap();
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
    let _g = ENV_LOCK.lock().unwrap();
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
    let ws = PathBuf::from("/home/test/.openhuman/workspace");
    let (config_dir, workspace_dir) = resolve_config_dir_for_workspace(&ws);
    // Config dir is the parent of workspace.
    assert!(
        config_dir.ends_with(".openhuman") || config_dir == PathBuf::from("/home/test/.openhuman")
    );
    assert!(workspace_dir.ends_with("workspace"));
}

// ── apply_env_overlay_with: EnvLookup seam ─────────────────────
//
// These tests exercise every env override branch via a `HashMapEnv`
// fixture so they neither mutate the process environment nor need
// to grab `TEST_ENV_LOCK`. They can all run in parallel.

use std::collections::HashMap;

/// In-memory [`EnvLookup`] used by the overlay tests. Case-sensitive
/// to mirror Unix `std::env::var` semantics.
#[derive(Default)]
struct HashMapEnv {
    entries: HashMap<String, String>,
}

impl HashMapEnv {
    fn new() -> Self {
        Self::default()
    }

    fn with(mut self, key: &str, value: &str) -> Self {
        self.entries.insert(key.to_string(), value.to_string());
        self
    }
}

impl EnvLookup for HashMapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.entries.get(key).cloned()
    }

    fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }
}

#[test]
fn env_overlay_model_prefers_openhuman_over_alias() {
    // Both set → OPENHUMAN_MODEL wins.
    let env = HashMapEnv::new()
        .with("OPENHUMAN_MODEL", "specific-v2")
        .with("MODEL", "alias-fallback");
    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(&env);
    assert_eq!(cfg.default_model.as_deref(), Some("specific-v2"));

    // Only alias set → alias wins.
    let env = HashMapEnv::new().with("MODEL", "alias-only");
    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(&env);
    assert_eq!(cfg.default_model.as_deref(), Some("alias-only"));
}

#[test]
fn env_overlay_model_ignores_empty() {
    let env = HashMapEnv::new().with("OPENHUMAN_MODEL", "");
    let mut cfg = Config::default();
    let original = cfg.default_model.clone();
    cfg.apply_env_overlay_with(&env);
    assert_eq!(cfg.default_model, original, "empty value must not clobber");
}

#[test]
fn env_overlay_temperature_accepts_valid_and_ignores_out_of_range_or_garbage() {
    let mut cfg = Config::default();
    cfg.default_temperature = 0.5;

    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_TEMPERATURE", "1.5"));
    assert!((cfg.default_temperature - 1.5).abs() < f64::EPSILON);

    // Negative (< 0.0) — ignored.
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_TEMPERATURE", "-0.1"));
    assert!((cfg.default_temperature - 1.5).abs() < f64::EPSILON);

    // Above cap (> 2.0) — ignored.
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_TEMPERATURE", "2.5"));
    assert!((cfg.default_temperature - 1.5).abs() < f64::EPSILON);

    // Garbage — ignored.
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_TEMPERATURE", "nope"));
    assert!((cfg.default_temperature - 1.5).abs() < f64::EPSILON);

    // Boundaries — inclusive on both ends.
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_TEMPERATURE", "0"));
    assert_eq!(cfg.default_temperature, 0.0);
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_TEMPERATURE", "2"));
    assert_eq!(cfg.default_temperature, 2.0);
}

#[test]
fn env_overlay_reasoning_enabled_recognises_truthy_falsy_and_ignores_garbage() {
    let mut cfg = Config::default();
    cfg.runtime.reasoning_enabled = None;

    for truthy in ["1", "true", "yes", "on", "TRUE", " On "] {
        cfg.runtime.reasoning_enabled = None;
        cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_REASONING_ENABLED", truthy));
        assert_eq!(
            cfg.runtime.reasoning_enabled,
            Some(true),
            "truthy value {truthy:?} should enable reasoning"
        );
    }

    for falsy in ["0", "false", "no", "off", "OFF"] {
        cfg.runtime.reasoning_enabled = Some(true);
        cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_REASONING_ENABLED", falsy));
        assert_eq!(
            cfg.runtime.reasoning_enabled,
            Some(false),
            "falsy value {falsy:?} should disable reasoning"
        );
    }

    // Garbage leaves the previous value unchanged.
    cfg.runtime.reasoning_enabled = Some(true);
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_REASONING_ENABLED", "maybe"));
    assert_eq!(cfg.runtime.reasoning_enabled, Some(true));

    // Alias works when the OPENHUMAN variant is absent.
    cfg.runtime.reasoning_enabled = None;
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("REASONING_ENABLED", "yes"));
    assert_eq!(cfg.runtime.reasoning_enabled, Some(true));
}

#[test]
fn env_overlay_web_search_limits_validated() {
    let mut cfg = Config::default();
    cfg.web_search.max_results = 3;
    cfg.web_search.timeout_secs = 10;

    // Valid values apply.
    cfg.apply_env_overlay_with(
        &HashMapEnv::new()
            .with("OPENHUMAN_WEB_SEARCH_MAX_RESULTS", "7")
            .with("OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS", "25"),
    );
    assert_eq!(cfg.web_search.max_results, 7);
    assert_eq!(cfg.web_search.timeout_secs, 25);

    // Out-of-range — ignored.
    cfg.apply_env_overlay_with(
        &HashMapEnv::new()
            .with("OPENHUMAN_WEB_SEARCH_MAX_RESULTS", "0")
            .with("OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS", "0"),
    );
    assert_eq!(cfg.web_search.max_results, 7);
    assert_eq!(cfg.web_search.timeout_secs, 25);

    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_WEB_SEARCH_MAX_RESULTS", "11"));
    assert_eq!(cfg.web_search.max_results, 7);

    // Bare aliases also accepted when the OPENHUMAN-prefixed variant is absent.
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("WEB_SEARCH_MAX_RESULTS", "4"));
    assert_eq!(cfg.web_search.max_results, 4);
}

#[test]
fn env_overlay_proxy_url_enables_proxy_when_not_explicit() {
    let mut cfg = Config::default();
    assert!(!cfg.proxy.enabled);

    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_HTTP_PROXY", "http://proxy.local:3128"),
    );

    assert!(
        cfg.proxy.enabled,
        "setting a proxy URL without explicit enable should auto-enable"
    );
    assert_eq!(
        cfg.proxy.http_proxy.as_deref(),
        Some("http://proxy.local:3128")
    );
}

#[test]
fn env_overlay_explicit_proxy_enabled_overrides_auto_enable() {
    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(
        &HashMapEnv::new()
            .with("OPENHUMAN_PROXY_ENABLED", "false")
            .with("OPENHUMAN_HTTP_PROXY", "http://proxy.local:3128"),
    );
    assert!(
        !cfg.proxy.enabled,
        "explicit OPENHUMAN_PROXY_ENABLED=false must win over URL-driven auto-enable"
    );
}

#[test]
fn env_overlay_proxy_scope_invalid_value_leaves_scope_unchanged() {
    let mut cfg = Config::default();
    let original_scope = cfg.proxy.scope;
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_PROXY_SCOPE", "bogus-scope"));
    assert_eq!(cfg.proxy.scope, original_scope);
}

#[test]
fn env_overlay_node_flags_respect_bool_parser() {
    let mut cfg = Config::default();
    let original_version = cfg.node.version.clone();

    cfg.apply_env_overlay_with(
        &HashMapEnv::new()
            .with("OPENHUMAN_NODE_ENABLED", "yes")
            .with("OPENHUMAN_NODE_PREFER_SYSTEM", "off")
            .with("OPENHUMAN_NODE_CACHE_DIR", "/tmp/oh-node"),
    );
    assert!(cfg.node.enabled);
    assert!(!cfg.node.prefer_system);
    assert_eq!(cfg.node.cache_dir, "/tmp/oh-node");
    assert_eq!(
        cfg.node.version, original_version,
        "untouched keys stay at defaults"
    );

    // Unrecognised bool — ignored, keeps previous true.
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_NODE_ENABLED", "perhaps"));
    assert!(cfg.node.enabled);

    // Blank version does NOT clobber.
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_NODE_VERSION", "   "));
    assert_eq!(cfg.node.version, original_version);
}

#[test]
fn env_overlay_sentry_dsn_trims_and_ignores_blank() {
    let mut cfg = Config::default();
    cfg.observability.sentry_dsn = None;

    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_SENTRY_DSN", "  https://t@sentry.io/42  "),
    );
    assert_eq!(
        cfg.observability.sentry_dsn.as_deref(),
        Some("https://t@sentry.io/42")
    );

    // Blank value — ignored (previous DSN retained).
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_SENTRY_DSN", "   "));
    assert_eq!(
        cfg.observability.sentry_dsn.as_deref(),
        Some("https://t@sentry.io/42")
    );
}

#[test]
fn env_overlay_prefers_namespaced_core_sentry_dsn() {
    let mut cfg = Config::default();
    cfg.observability.sentry_dsn = None;

    cfg.apply_env_overlay_with(
        &HashMapEnv::new()
            .with("OPENHUMAN_SENTRY_DSN", "https://legacy@sentry.io/1")
            .with("OPENHUMAN_CORE_SENTRY_DSN", "https://new@sentry.io/2"),
    );
    assert_eq!(
        cfg.observability.sentry_dsn.as_deref(),
        Some("https://new@sentry.io/2"),
        "OPENHUMAN_CORE_SENTRY_DSN must win over OPENHUMAN_SENTRY_DSN"
    );
}

#[test]
fn env_overlay_namespaced_core_sentry_dsn_works_alone() {
    let mut cfg = Config::default();
    cfg.observability.sentry_dsn = None;

    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_CORE_SENTRY_DSN", "https://token@sentry.io/3"),
    );
    assert_eq!(
        cfg.observability.sentry_dsn.as_deref(),
        Some("https://token@sentry.io/3")
    );
}

#[test]
fn env_overlay_analytics_enabled_parses_truthy_falsy() {
    let mut cfg = Config::default();
    cfg.observability.analytics_enabled = false;
    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_ANALYTICS_ENABLED", "1"));
    assert!(cfg.observability.analytics_enabled);

    cfg.apply_env_overlay_with(&HashMapEnv::new().with("OPENHUMAN_ANALYTICS_ENABLED", "0"));
    assert!(!cfg.observability.analytics_enabled);
}

#[test]
fn env_overlay_learning_source_values_and_invalid_ignored() {
    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_LEARNING_REFLECTION_SOURCE", "local"),
    );
    assert_eq!(
        cfg.learning.reflection_source,
        crate::openhuman::config::ReflectionSource::Local
    );

    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_LEARNING_REFLECTION_SOURCE", "cloud"),
    );
    assert_eq!(
        cfg.learning.reflection_source,
        crate::openhuman::config::ReflectionSource::Cloud
    );

    // Unknown — ignored, retains cloud from previous step.
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_LEARNING_REFLECTION_SOURCE", "bogus"),
    );
    assert_eq!(
        cfg.learning.reflection_source,
        crate::openhuman::config::ReflectionSource::Cloud
    );
}

#[test]
fn env_overlay_learning_numeric_values_parse() {
    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(
        &HashMapEnv::new()
            .with("OPENHUMAN_LEARNING_MAX_REFLECTIONS_PER_SESSION", "8")
            .with("OPENHUMAN_LEARNING_MIN_TURN_COMPLEXITY", "2"),
    );
    assert_eq!(cfg.learning.max_reflections_per_session, 8);
    assert_eq!(cfg.learning.min_turn_complexity, 2);
}

#[test]
fn env_overlay_dictation_activation_mode_only_toggle_or_push() {
    let mut cfg = Config::default();

    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_DICTATION_ACTIVATION_MODE", "toggle"),
    );
    assert_eq!(
        cfg.dictation.activation_mode,
        crate::openhuman::config::DictationActivationMode::Toggle
    );

    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_DICTATION_ACTIVATION_MODE", "push"),
    );
    assert_eq!(
        cfg.dictation.activation_mode,
        crate::openhuman::config::DictationActivationMode::Push
    );

    // Unknown — retains previous value (Push).
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_DICTATION_ACTIVATION_MODE", "wave"),
    );
    assert_eq!(
        cfg.dictation.activation_mode,
        crate::openhuman::config::DictationActivationMode::Push
    );
}

#[test]
fn env_overlay_context_tool_result_budget_env_suppresses_legacy_migration() {
    // If the env var is *present*, the `agent.tool_result_budget_bytes`
    // migration must NOT run — even when the explicit env value equals
    // the default. This protects users who explicitly set the env to
    // the default.
    let default_budget = crate::openhuman::context::DEFAULT_TOOL_RESULT_BUDGET_BYTES;
    let mut cfg = Config::default();
    cfg.context.tool_result_budget_bytes = default_budget;
    cfg.agent.tool_result_budget_bytes = 999_999;

    cfg.apply_env_overlay_with(&HashMapEnv::new().with(
        "OPENHUMAN_CONTEXT_TOOL_RESULT_BUDGET_BYTES",
        &default_budget.to_string(),
    ));
    assert_eq!(
        cfg.context.tool_result_budget_bytes, default_budget,
        "env presence must suppress the legacy agent→context copy"
    );
}

#[test]
fn env_overlay_context_tool_result_budget_legacy_migration_when_env_absent() {
    // Env absent, context at default, agent customised → agent value copies forward.
    let default_budget = crate::openhuman::context::DEFAULT_TOOL_RESULT_BUDGET_BYTES;
    let mut cfg = Config::default();
    cfg.context.tool_result_budget_bytes = default_budget;
    cfg.agent.tool_result_budget_bytes = 777_777;

    cfg.apply_env_overlay_with(&HashMapEnv::new());
    assert_eq!(cfg.context.tool_result_budget_bytes, 777_777);
}

#[test]
fn env_overlay_context_tool_result_budget_env_wins_over_legacy_migration() {
    // Env present with a non-default value, and agent also customised.
    // The env value must apply; the legacy agent→context copy must NOT
    // overwrite it.
    let mut cfg = Config::default();
    cfg.agent.tool_result_budget_bytes = 111_111;

    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_CONTEXT_TOOL_RESULT_BUDGET_BYTES", "222222"),
    );
    assert_eq!(
        cfg.context.tool_result_budget_bytes, 222_222,
        "env value wins; legacy migration suppressed"
    );
}

#[test]
fn env_overlay_auto_update_interval_parses_u32() {
    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(
        &HashMapEnv::new()
            .with("OPENHUMAN_AUTO_UPDATE_ENABLED", "true")
            .with("OPENHUMAN_AUTO_UPDATE_INTERVAL_MINUTES", "60"),
    );
    assert!(cfg.update.enabled);
    assert_eq!(cfg.update.interval_minutes, 60);

    // Garbage numeric — ignored, previous value retained.
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_AUTO_UPDATE_INTERVAL_MINUTES", "hello"),
    );
    assert_eq!(cfg.update.interval_minutes, 60);
}

#[test]
fn env_overlay_auto_update_restart_strategy_accepts_supported_values() {
    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_AUTO_UPDATE_RESTART_STRATEGY", "supervisor"),
    );
    assert_eq!(
        cfg.update.restart_strategy,
        crate::openhuman::config::UpdateRestartStrategy::Supervisor
    );

    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_AUTO_UPDATE_RESTART_STRATEGY", "self_replace"),
    );
    assert_eq!(
        cfg.update.restart_strategy,
        crate::openhuman::config::UpdateRestartStrategy::SelfReplace
    );
}

#[test]
fn env_overlay_auto_update_rpc_mutations_enabled_parses_bool() {
    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(
        &HashMapEnv::new().with("OPENHUMAN_AUTO_UPDATE_RPC_MUTATIONS_ENABLED", "false"),
    );
    assert!(!cfg.update.rpc_mutations_enabled);
}

#[test]
fn env_overlay_empty_lookup_leaves_defaults_intact() {
    // The seam with no env entries should be a no-op on a fresh Config.
    let mut cfg = Config::default();
    let before = (
        cfg.default_model.clone(),
        cfg.default_temperature,
        cfg.runtime.reasoning_enabled,
        cfg.update.enabled,
        cfg.dictation.enabled,
    );
    cfg.apply_env_overlay_with(&HashMapEnv::new());
    let after = (
        cfg.default_model.clone(),
        cfg.default_temperature,
        cfg.runtime.reasoning_enabled,
        cfg.update.enabled,
        cfg.dictation.enabled,
    );
    assert_eq!(before, after);
}

#[test]
fn env_lookup_get_any_preserves_precedence() {
    let env = HashMapEnv::new()
        .with("KEY_A", "first-wins")
        .with("KEY_B", "second")
        .with("KEY_C", "third");
    // Ordered lookup: first hit wins.
    assert_eq!(env.get_any(&["KEY_A", "KEY_B"]), Some("first-wins".into()));
    // Missing first → falls through.
    assert_eq!(
        env.get_any(&["KEY_MISSING", "KEY_B"]),
        Some("second".into())
    );
    // All missing → None.
    assert_eq!(env.get_any(&["KEY_X", "KEY_Y"]), None);
}

// ── resolve_runtime_config_dirs_with ──────────────────────────────────────

#[tokio::test]
async fn resolve_runtime_config_dirs_with_env_workspace_override() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let default_workspace = root.join("workspace");

    // Point OPENHUMAN_WORKSPACE at a custom path via HashMapEnv — no
    // process-env mutation needed.
    let custom_ws = tmp.path().join("custom_ws");
    let env = HashMapEnv::new().with("OPENHUMAN_WORKSPACE", custom_ws.to_str().unwrap());

    let (oh_dir, ws_dir, source) = resolve_runtime_config_dirs_with(root, &default_workspace, &env)
        .await
        .unwrap();

    assert_eq!(source, ConfigResolutionSource::EnvWorkspace);
    // resolve_config_dir_for_workspace: no config.toml and basename ≠
    // "workspace" → oh_dir == custom_ws, ws_dir == custom_ws/workspace.
    assert_eq!(oh_dir, custom_ws);
    assert_eq!(ws_dir, custom_ws.join("workspace"));
}

#[tokio::test]
async fn resolve_runtime_config_dirs_with_empty_env_falls_back_to_default() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let default_workspace = root.join("workspace");

    // Empty env: no OPENHUMAN_WORKSPACE → falls through to the pre-login
    // user directory path (no active_user.toml, no workspace marker).
    let env = HashMapEnv::new();
    let (oh_dir, _ws_dir, source) =
        resolve_runtime_config_dirs_with(root, &default_workspace, &env)
            .await
            .unwrap();

    assert_eq!(source, ConfigResolutionSource::DefaultConfigDir);
    // Should be under the users/pre-login tree, not the bare root.
    assert!(
        oh_dir.starts_with(root.join("users")),
        "expected oh_dir under users/, got {oh_dir:?}"
    );
}

#[test]
fn apply_env_overrides_commits_side_effects_to_runtime_proxy() {
    use crate::openhuman::config::schema::proxy::{runtime_proxy_config, set_runtime_proxy_config};

    // Hold the env lock so no other test races on proxy-related env vars.
    let _g = ENV_LOCK.lock().unwrap();
    clear_env(&[
        "OPENHUMAN_PROXY_ENABLED",
        "OPENHUMAN_HTTP_PROXY",
        "HTTP_PROXY",
        "OPENHUMAN_HTTPS_PROXY",
        "HTTPS_PROXY",
        "OPENHUMAN_ALL_PROXY",
        "ALL_PROXY",
    ]);

    // Snapshot the global runtime proxy config so we can restore it afterwards
    // and avoid leaking state into other tests.
    let previous_runtime = runtime_proxy_config();

    // Build a config with proxy fields set directly on the struct.
    // We cannot pre-configure via apply_env_overlay_with + a HashMapEnv and
    // then call apply_env_overrides(), because apply_env_overrides() internally
    // re-runs apply_env_overlay_with(&ProcessEnv) which reads the real process
    // environment — overwriting anything set via a HashMapEnv beforehand.
    // Setting fields directly ensures they survive the ProcessEnv overlay
    // (which only writes fields when the corresponding env var is present).
    let mut cfg = Config::default();
    cfg.proxy.http_proxy = Some("http://proxy.test:8080".to_string());
    cfg.proxy.enabled = true;

    // apply_env_overrides commits side effects: it calls set_runtime_proxy_config
    // with the current proxy config after the ProcessEnv overlay.
    cfg.apply_env_overrides();

    // `set_runtime_proxy_config` must have been called: the global should
    // reflect the proxy URL we set on cfg.proxy.
    let runtime = runtime_proxy_config();
    assert!(
        runtime.enabled,
        "runtime proxy must be enabled after apply_env_overrides"
    );
    assert_eq!(
        runtime.http_proxy.as_deref(),
        Some("http://proxy.test:8080"),
        "runtime proxy URL must match the value set on cfg.proxy"
    );

    // Restore the global runtime proxy state so this test doesn't bleed into
    // other tests that inspect runtime_proxy_config().
    set_runtime_proxy_config(previous_runtime);
}
