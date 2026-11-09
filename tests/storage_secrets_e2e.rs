//! Keyring secrets and the credential stores on a configured storage
//! backend, end to end through their public functions.
//!
//! Its own test binary because it installs a backend into the process-wide
//! storage slot and sets the keyring master-key environment, which would
//! reroute every other suite's secrets in a shared process. The tests take
//! turns on the slot (`support/storage_case.rs`).

use openhuman_core::credentials::profiles::{AuthProfile, AuthProfilesStore};
use openhuman_core::flows::http_credentials::{HttpCredential, HttpCredentialsStore};
use openhuman_core::security::secret_storage;

#[path = "support/storage_case.rs"]
mod storage_case;

use storage_case::Case;

/// The workspace every case shares. The process keyring caches the backend it
/// first resolves (a `dev-keychain.json` under the workspace in effect then),
/// so a later case in the same process must find it in the same place. The
/// credential files live in a directory of each case's own.
fn keyring_workspace() -> &'static std::path::Path {
    static WORKSPACE: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    WORKSPACE
        .get_or_init(|| tempfile::tempdir().unwrap())
        .path()
}

#[tokio::test]
async fn a_configured_backend_holds_keyring_and_credential_secrets() {
    let case = Case::open();
    let workspace = keyring_workspace();
    // Keep any process-backend fallback inside the temp workspace, and give
    // the storage secrets a master key without touching an OS keychain.
    std::env::set_var("OPENHUMAN_WORKSPACE", workspace);
    std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "file");
    std::env::set_var("OPENHUMAN_KEYRING_MASTER_KEY", "11".repeat(32));

    // Credentials written before any backend existed (the classic files).
    let state = case.data_dir.path().join("state");
    AuthProfilesStore::new(&state, false)
        .upsert_profile(
            AuthProfile::new_token("legacy", "default", "sk-legacy".to_string()),
            true,
        )
        .await
        .unwrap();
    HttpCredentialsStore::new(&state, false)
        .upsert(&HttpCredential::bearer("legacy-http", "ghp-legacy"))
        .await
        .unwrap();
    secret_storage::set("legacy-user", "old_token", "old-value").unwrap();
    let http_file = std::fs::read(state.join("http-credentials.json")).unwrap();

    let before: std::collections::HashMap<&str, Option<Vec<u8>>> =
        ["secrets.enc", "dev-keychain.json"]
            .into_iter()
            .map(|f| (f, std::fs::read(workspace.join(f)).ok()))
            .collect();

    case.install();

    secret_storage::set("user-1", "api_token", "tok-123").unwrap();
    assert_eq!(
        secret_storage::get("user-1", "api_token")
            .unwrap()
            .as_deref(),
        Some("tok-123")
    );
    assert!(secret_storage::is_available());
    secret_storage::delete("user-1", "api_token").unwrap();
    assert!(secret_storage::get("user-1", "api_token")
        .unwrap()
        .is_none());

    // A secret the process keyring held is adopted on first read, and a
    // delete removes it from both places so it cannot resurface.
    assert_eq!(
        secret_storage::get("legacy-user", "old_token")
            .unwrap()
            .as_deref(),
        Some("old-value")
    );
    secret_storage::delete("legacy-user", "old_token").unwrap();
    assert!(secret_storage::get("legacy-user", "old_token")
        .unwrap()
        .is_none());

    let profiles = AuthProfilesStore::new(&state, false);
    profiles
        .upsert_profile(
            AuthProfile::new_token("openai", "default", "sk-test".to_string()),
            true,
        )
        .await
        .unwrap();
    // The pre-existing profile was adopted, not replaced.
    let loaded = profiles.load().await.unwrap();
    assert_eq!(loaded.profiles.len(), 2);

    let http = HttpCredentialsStore::new(&state, false);
    http.upsert(&HttpCredential::bearer("github", "ghp-test"))
        .await
        .unwrap();
    assert!(http.get("github").await.unwrap().is_some());
    assert!(http.get("legacy-http").await.unwrap().is_some());

    // The profile document is kept in its encrypted local store; the HTTP
    // credential compatibility file remains untouched by the configured
    // secret backend path.
    assert!(state.join("auth-profiles/secrets.enc").exists());
    assert_eq!(
        std::fs::read(state.join("http-credentials.json")).unwrap(),
        http_file
    );
    // `secrets.enc` was not touched, and nothing of the storage-side secret
    // reached the process keychain file.
    assert_eq!(
        std::fs::read(workspace.join("secrets.enc")).ok(),
        before.get("secrets.enc").cloned().flatten(),
        "secrets.enc unchanged"
    );
    let dev = std::fs::read_to_string(workspace.join("dev-keychain.json")).unwrap();
    assert!(!dev.contains("user-1") && !dev.contains("tok-123"), "{dev}");

    // Clearing the configured backend returns the HTTP store to its legacy
    // file. Auth profiles have their own encrypted local store and remain.
    assert!(openhuman_core::storage::clear());
    assert!(http.get("github").await.unwrap().is_none());
    assert!(http.get("legacy-http").await.unwrap().is_some());
    assert_eq!(profiles.load().await.unwrap().profiles.len(), 2);
}
