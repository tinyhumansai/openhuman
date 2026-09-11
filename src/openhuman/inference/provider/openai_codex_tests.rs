use super::*;
use tempfile::tempdir;

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::remove_var(key) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

#[test]
fn client_version_prefers_explicit_env_override() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _version = EnvVarGuard::set("OPENAI_CODEX_CLIENT_VERSION", "  0.200.0  ");
    let _codex_home = EnvVarGuard::remove("CODEX_HOME");

    assert_eq!(openai_codex_client_version(), "0.200.0");
}

#[test]
fn client_version_reads_codex_models_cache() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models_cache.json"),
        serde_json::json!({ "client_version": "0.137.0", "models": [] }).to_string(),
    )
    .unwrap();
    let _version = EnvVarGuard::remove("OPENAI_CODEX_CLIENT_VERSION");
    let _codex_home = EnvVarGuard::set("CODEX_HOME", tmp.path());

    assert_eq!(openai_codex_client_version(), "0.137.0");
}

#[test]
fn client_version_models_cache_precedes_version_file() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models_cache.json"),
        serde_json::json!({ "client_version": "0.137.0", "models": [] }).to_string(),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("version.json"),
        serde_json::json!({ "latest_version": "0.140.0" }).to_string(),
    )
    .unwrap();
    let _version = EnvVarGuard::remove("OPENAI_CODEX_CLIENT_VERSION");
    let _codex_home = EnvVarGuard::set("CODEX_HOME", tmp.path());

    assert_eq!(openai_codex_client_version(), "0.137.0");
}

#[test]
fn client_version_falls_back_to_codex_version_file() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    std::fs::write(
        tmp.path().join("version.json"),
        serde_json::json!({ "latest_version": "0.140.0" }).to_string(),
    )
    .unwrap();
    let _version = EnvVarGuard::remove("OPENAI_CODEX_CLIENT_VERSION");
    let _codex_home = EnvVarGuard::set("CODEX_HOME", tmp.path());

    assert_eq!(openai_codex_client_version(), "0.140.0");
}

#[test]
fn client_version_uses_default_when_codex_files_are_missing() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    let _version = EnvVarGuard::remove("OPENAI_CODEX_CLIENT_VERSION");
    let _codex_home = EnvVarGuard::set("CODEX_HOME", tmp.path());

    assert_eq!(
        openai_codex_client_version(),
        OPENAI_CODEX_DEFAULT_CLIENT_VERSION
    );
}
