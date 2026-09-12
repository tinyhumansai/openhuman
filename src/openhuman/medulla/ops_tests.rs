use super::*;

/// RAII guard to snapshot and restore a process-global environment variable.
struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
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

/// Serialize env mutation via the crate-wide backend env lock: these
/// tests remove `BACKEND_URL`/`VITE_BACKEND_URL`, which `api::config`
/// and `core::cli_tests` tests concurrently set — a module-local lock
/// cannot prevent that cross-module race (it deterministically broke
/// these assertions under the full-suite coverage lane).
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::api::config::backend_env_test_lock()
}

/// A config whose workspace is an empty temp dir, so `get_session_token`
/// deterministically finds nothing regardless of the developer's real
/// sign-in state.
fn signed_out_config(tmp: &tempfile::TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        ..Config::default()
    }
}

/// Clearing every base-URL env var no longer produces `MedullaNoBaseUrl`.
///
/// Since #5245 `resolve::base_url` falls back to
/// `effective_backend_api_url`, which ends at the compiled-in
/// `DEFAULT_API_BASE_URL` — so it never returns `None` and the
/// `NoBaseUrl` arm is unreachable. "No backend configured" stopped being a
/// state the product can be in; the unconfigured state that remains is
/// "signed out", which is what this now pins. The env isolation is kept
/// because it is what proves the fallback, rather than an override, is
/// answering.
#[test]
fn not_configured_encodes_an_expected_user_state_envelope() {
    let tmp = tempfile::tempdir().unwrap();
    let config = signed_out_config(&tmp);
    // Isolate environment to ensure BACKEND_URL and VITE_BACKEND_URL do not
    // provide a fallback through effective_backend_api_url.
    let _guard = env_lock();
    let _env_medulla = EnvGuard::remove(resolve::MEDULLA_BASE_URL_ENV);
    let _env_backend = EnvGuard::remove("BACKEND_URL");
    let _env_vite = EnvGuard::remove("VITE_BACKEND_URL");
    let err = resolved(&config).expect_err("must not resolve");
    let decoded = StructuredRpcError::decode(&err).expect("structured envelope");
    assert!(
        decoded.expected_user_state,
        "an unconfigured host is a user state, not an internal fault"
    );
    // With every override cleared, `base_url` still resolves to the hosted
    // backend default by design (`resolve.rs`'s
    // `an_unconfigured_install_falls_back_to_the_hosted_backend` pins
    // that), so `MedullaNoBaseUrl` is unreachable from a cleared env — the
    // unconfigured state actually produced is the missing session token
    // (signed out).
    assert_eq!(
        decoded
            .data
            .and_then(|d| d["kind"].as_str().map(String::from)),
        Some("MedullaNoSessionToken".to_string())
    );
}

#[test]
fn api_error_preserves_the_backend_error_code_as_kind() {
    let err = call::<()>(
        Err(ClientError::Api {
            status: Some(409),
            message: "already archived".into(),
            error_code: Some("SessionArchived".into()),
            details: None,
        }),
        "medulla_test",
    )
    .expect_err("must be an error");
    let decoded = StructuredRpcError::decode(&err).expect("structured envelope");
    assert_eq!(decoded.data.unwrap()["kind"], "SessionArchived");
    assert!(!decoded.expected_user_state, "409 is not a sign-in prompt");
}

#[test]
fn unauthorized_is_marked_expected_user_state() {
    for status in [401, 403] {
        let err = call::<()>(
            Err(ClientError::Api {
                status: Some(status),
                message: "nope".into(),
                error_code: None,
                details: None,
            }),
            "medulla_test",
        )
        .expect_err("must be an error");
        let decoded = StructuredRpcError::decode(&err).expect("structured envelope");
        assert!(
            decoded.expected_user_state,
            "{status} should read as a sign-in prompt, not a bug"
        );
    }
}

#[test]
fn missing_error_code_falls_back_to_a_stable_kind() {
    let err = call::<()>(Err(ClientError::Decode("bad json".into())), "medulla_test")
        .expect_err("must be an error");
    let decoded = StructuredRpcError::decode(&err).expect("structured envelope");
    assert_eq!(decoded.data.unwrap()["kind"], "MedullaRequestFailed");
}

#[tokio::test]
async fn status_reports_unconfigured_rather_than_failing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = signed_out_config(&tmp);
    // Isolate environment to ensure BACKEND_URL and VITE_BACKEND_URL do not
    // provide a fallback through effective_backend_api_url.
    let _guard = env_lock();
    let _env_medulla = EnvGuard::remove(resolve::MEDULLA_BASE_URL_ENV);
    let _env_backend = EnvGuard::remove("BACKEND_URL");
    let _env_vite = EnvGuard::remove("VITE_BACKEND_URL");
    let out = status(&config).await.expect("status never errors");
    assert!(!out.value.configured);
    // See `not_configured_encodes_an_expected_user_state_envelope`: the
    // base URL always falls back to the hosted default, so the reason a
    // cleared env reports is the missing session token, never NoBaseUrl.
    assert_eq!(out.value.reason.as_deref(), Some("MedullaNoSessionToken"));
}
