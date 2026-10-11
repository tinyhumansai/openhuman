use super::*;

#[test]
fn extract_query_token_returns_none_on_missing_query() {
    assert_eq!(extract_query_token(None), None);
}

#[test]
fn extract_query_token_returns_none_when_key_absent() {
    assert_eq!(extract_query_token(Some("other=1&foo=bar")), None);
}

#[test]
fn extract_query_token_returns_none_on_empty_value() {
    assert_eq!(extract_query_token(Some("token=")), None);
    assert_eq!(extract_query_token(Some("token=%20%20")), None);
}

#[test]
fn extract_query_token_returns_first_value_on_duplicate_keys() {
    // Last-wins vs first-wins is a question the FE never hits; pin
    // first-wins so any future ambiguity is documented.
    assert_eq!(
        extract_query_token(Some("token=alpha&token=beta")),
        Some("alpha".to_string())
    );
}

#[test]
fn extract_query_token_url_decodes_value() {
    // `encodeURIComponent` on the FE may percent-encode a hex token
    // accidentally (it shouldn't, but defensive); confirm round-trip.
    assert_eq!(
        extract_query_token(Some("token=cafe%2Dbabe")),
        Some("cafe-babe".to_string())
    );
}

#[test]
fn login_callbacks_are_no_longer_public() {
    // The desktop `/auth` fallback and `/auth/telegram` exchanged login tokens
    // inside the core; the host (openhuman_tinyhumans::session) owns that now.
    assert!(!PUBLIC_PATHS.contains(&"/auth"));
    assert!(!PUBLIC_PATHS.contains(&"/auth/telegram"));
}

#[test]
fn agentbox_run_and_jobs_paths_are_no_longer_public() {
    // These bypassed bearer auth only to serve the AgentBox marketplace
    // surface, which moved to tinybox. Nothing mounts them now, so they
    // must authenticate like any other path — a re-added entry here would
    // silently open an unauthenticated route.
    assert!(!is_public_path("/run"));
    assert!(!is_public_path("/jobs/abc-123"));
    assert!(!is_public_path(
        "/jobs/00000000-0000-0000-0000-000000000000"
    ));
    // Sanity: still protect the executable surface.
    assert!(!is_public_path("/rpc"));
    assert!(!is_public_path("/v1/chat/completions"));
}

#[test]
fn is_external_inference_path_matches_only_v1_routes() {
    assert!(is_external_inference_path("/v1"));
    assert!(is_external_inference_path("/v1/models"));
    assert!(is_external_inference_path("/v1/chat/completions"));
    assert!(!is_external_inference_path("/rpc"));
    assert!(!is_external_inference_path("/v10/models"));
}

#[test]
fn verify_external_inference_bearer_for_config_accepts_stored_key() {
    // Keep a session_store test from installing a storage backend mid-test.
    let _slot = crate::STORAGE_SLOT_TEST_LOCK.blocking_lock();
    // This headless fixture exercises the encrypted profile without the
    // operator's keychain or a caller-supplied master-key file.
    struct FixtureKey {
        key: Option<std::ffi::OsString>,
        file: Option<std::ffi::OsString>,
        workspace: Option<std::ffi::OsString>,
        backend: Option<std::ffi::OsString>,
    }
    impl Drop for FixtureKey {
        fn drop(&mut self) {
            for (name, value) in [
                ("OPENHUMAN_KEYRING_MASTER_KEY", self.key.take()),
                ("OPENHUMAN_KEYRING_MASTER_KEY_FILE", self.file.take()),
                ("OPENHUMAN_WORKSPACE", self.workspace.take()),
                ("OPENHUMAN_KEYRING_BACKEND", self.backend.take()),
            ] {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
    let tmp = tempfile::tempdir().unwrap();
    let _key = FixtureKey {
        key: std::env::var_os("OPENHUMAN_KEYRING_MASTER_KEY"),
        file: std::env::var_os("OPENHUMAN_KEYRING_MASTER_KEY_FILE"),
        workspace: std::env::var_os("OPENHUMAN_WORKSPACE"),
        backend: std::env::var_os("OPENHUMAN_KEYRING_BACKEND"),
    };
    std::env::remove_var("OPENHUMAN_KEYRING_MASTER_KEY_FILE");
    std::env::set_var("OPENHUMAN_KEYRING_MASTER_KEY", "11".repeat(32));
    std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "encrypted_file");
    crate::embed::process::init_master_key().expect("initialize fixture master key");
    let config = Config {
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };

    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        EXTERNAL_OPENAI_COMPAT_PROVIDER,
        "default",
        "external-test-key",
        std::collections::HashMap::new(),
        true,
    )
    .unwrap();

    assert!(verify_external_inference_bearer_for_config(
        &config,
        "external-test-key"
    ));
    assert!(!verify_external_inference_bearer_for_config(
        &config,
        "wrong-key"
    ));
}
