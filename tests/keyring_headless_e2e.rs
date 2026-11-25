//! A headless core persists secret slots across process restarts with explicit
//! encrypted storage, without an OS provider or key injection.

use openhuman_core::security::secret_storage;
use std::process::Command;

#[test]
fn headless_core_reuses_its_workspace_key_after_restart() {
    const STAGE: &str = "OPENHUMAN_TEST_HEADLESS_KEYRING_STAGE";
    if let Ok(stage) = std::env::var(STAGE) {
        let workspace = std::path::PathBuf::from(std::env::var("OPENHUMAN_WORKSPACE").unwrap());
        secret_storage::init_workspace(&workspace);
        secret_storage::init_master_key().unwrap();
        assert_eq!(secret_storage::backend_name(), "encrypted_file");
        assert!(secret_storage::is_available());
        if stage == "write" {
            secret_storage::set("test-user", "token", "persisted-token").unwrap();
        } else {
            assert_eq!(
                secret_storage::get("test-user", "token")
                    .unwrap()
                    .as_deref(),
                Some("persisted-token")
            );
        }
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    // macOS exposes the temporary directory through `/var` aliases. The
    // secrets driver refuses symlinked namespace components, so pass its
    // canonical workspace path to both child processes.
    let workspace = std::fs::canonicalize(dir.path()).expect("canonicalize keyring workspace");
    let key_path = workspace.join("keyring/master.key");
    let mut saved_key = None;
    for stage in ["write", "read"] {
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "headless_core_reuses_its_workspace_key_after_restart",
            ])
            .env(STAGE, stage)
            .env("OPENHUMAN_WORKSPACE", &workspace)
            .env("OPENHUMAN_APP_ENV", "development")
            .env("OPENHUMAN_KEYRING_BACKEND", "encrypted_file")
            .env_remove("OPENHUMAN_KEYRING_MASTER_KEY")
            .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
            .env_remove("OPENHUMAN_STORAGE_URL")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "headless {stage} failed: {}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let key = std::fs::read(&key_path).unwrap();
        if let Some(previous) = &saved_key {
            assert!(previous == &key, "restart must not change the master key");
        }
        saved_key = Some(key);
    }
    assert!(workspace.join("secrets.enc").exists());
    assert!(!workspace.join("dev-keychain.json").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(key_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
