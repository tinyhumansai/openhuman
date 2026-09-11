use super::*;

/// Serialize env mutation via the crate-wide backend env lock:
/// `base_url` reads process-globals that `api::config`, `core::cli_tests`,
/// and `medulla::ops` tests also mutate — a module-local lock cannot
/// prevent that cross-module race.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::api::config::backend_env_test_lock()
}

/// RAII guard to snapshot and restore a process-global environment variable.
struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, val: &str) -> Self {
        let prev = std::env::var(key).ok();
        // SAFETY: caller holds ENV_LOCK guard.
        unsafe { std::env::set_var(key, val) };
        Self { key, prev }
    }

    fn remove(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        // SAFETY: caller holds ENV_LOCK guard.
        unsafe { std::env::remove_var(key) };
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            // SAFETY: caller's ENV_LOCK guard is still alive during drop.
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

fn config_with_api_url(url: Option<&str>) -> Config {
    let mut config = Config::default();
    config.api_url = url.map(str::to_string);
    config
}

#[test]
fn env_override_wins_over_api_url() {
    let _guard = env_lock();
    let _env = EnvGuard::set(MEDULLA_BASE_URL_ENV, "https://medulla.example");
    let resolved = base_url(&config_with_api_url(Some("https://api.example")));
    assert_eq!(resolved.as_deref(), Some("https://medulla.example"));
}

#[test]
fn falls_back_to_api_url_when_env_unset() {
    let _guard = env_lock();
    let _env = EnvGuard::remove(MEDULLA_BASE_URL_ENV);
    let resolved = base_url(&config_with_api_url(Some("https://api.example/")));
    assert_eq!(resolved.as_deref(), Some("https://api.example"));
}

#[test]
fn blank_env_does_not_shadow_api_url() {
    // An exported-but-empty var is a common shell accident; treating it as
    // "configured" would break an otherwise-working setup.
    let _guard = env_lock();
    let _env = EnvGuard::set(MEDULLA_BASE_URL_ENV, "   ");
    let resolved = base_url(&config_with_api_url(Some("https://api.example")));
    assert_eq!(resolved.as_deref(), Some("https://api.example"));
}

#[test]
fn an_unconfigured_install_falls_back_to_the_hosted_backend() {
    // Medulla and the OpenHuman backend are one deployment, so an install
    // that never wrote an explicit `api_url` — the normal case — must reach
    // the same host every other backend call defaults to, not report itself
    // unconfigured while auth and billing work.
    let _guard = env_lock();
    let _env_medulla = EnvGuard::remove(MEDULLA_BASE_URL_ENV);
    let _env_backend = EnvGuard::remove("BACKEND_URL");
    let _env_vite = EnvGuard::remove("VITE_BACKEND_URL");
    assert_eq!(
        base_url(&config_with_api_url(None)),
        Some(crate::api::config::effective_backend_api_url(&None))
    );
}

#[test]
fn a_local_model_runner_url_does_not_become_the_medulla_host() {
    // `api_url` doubles as the inference endpoint. Pointing it at Ollama
    // must not aim the Medulla client at a model runner that 404s every
    // path it speaks — the same guard every other backend call gets.
    let _guard = env_lock();
    let _env_medulla = EnvGuard::remove(MEDULLA_BASE_URL_ENV);
    let _env_backend = EnvGuard::remove("BACKEND_URL");
    let _env_vite = EnvGuard::remove("VITE_BACKEND_URL");
    let resolved = base_url(&config_with_api_url(Some("http://localhost:11434")));
    // Assert that the local model runner URL does not become the Medulla host;
    // instead, it falls back to the effective backend URL (same as all other backend calls).
    assert_eq!(
        resolved,
        Some(crate::api::config::effective_backend_api_url(&None))
    );
}

#[test]
fn trailing_slashes_are_trimmed_so_paths_do_not_double_up() {
    let _guard = env_lock();
    let _env = EnvGuard::set(MEDULLA_BASE_URL_ENV, "https://medulla.example///");
    let resolved = base_url(&config_with_api_url(None));
    assert_eq!(resolved.as_deref(), Some("https://medulla.example"));
}

#[test]
fn not_configured_messages_carry_no_secrets() {
    for reason in [NotConfigured::NoBaseUrl, NotConfigured::NoSessionToken] {
        let msg = reason.message();
        assert!(!msg.contains("http"), "message must not leak a URL: {msg}");
        assert!(!reason.kind().is_empty());
    }
}
