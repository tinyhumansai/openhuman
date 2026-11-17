use super::*;

#[test]
fn product_hosts_persist_secrets_in_the_workspace_registered_after_preflight() {
    const STAGE: &str = "OPENHUMAN_TEST_PRODUCT_SECRET_STAGE";
    if let Ok(stage) = std::env::var(STAGE) {
        let dir =
            std::path::PathBuf::from(std::env::var("OPENHUMAN_TEST_PRODUCT_SECRET_DIR").unwrap());
        install();
        assert!(!dir.join("secrets.enc").exists() || stage == "read");
        crate::core_host::security::secret_storage::init_workspace(&dir);
        let backend = installed_secret_backend().expect("persistent host default");
        assert_eq!(backend.name(), "encrypted_file");
        if stage == "write" {
            backend.set("user:token", "first-value").unwrap();
            backend.set("user:token", "saved-value").unwrap();
        } else {
            assert_eq!(
                backend.get("user:token").unwrap().as_deref(),
                Some("saved-value")
            );
            backend.delete("user:token").unwrap();
            backend.delete("user:token").unwrap();
            assert_eq!(backend.get("user:token").unwrap(), None);
        }
        assert!(dir.join("secrets.enc").is_file());
        assert!(!dir.join("dev-keychain.json").exists());
        return;
    }
    #[cfg(target_os = "macos")]
    let dir = tempfile::tempdir_in("/private/tmp").unwrap();
    #[cfg(not(target_os = "macos"))]
    let dir = tempfile::tempdir().unwrap();
    for stage in ["write", "read"] {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "host::secret_storage::tests::product_hosts_persist_secrets_in_the_workspace_registered_after_preflight"])
            .env(STAGE, stage)
            .env("OPENHUMAN_TEST_PRODUCT_SECRET_DIR", dir.path())
            .env("OPENHUMAN_KEYRING_MASTER_KEY", "12".repeat(32))
            .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
            .env_remove("OPENHUMAN_KEYRING_BACKEND")
            .env_remove("OPENHUMAN_STORAGE_URL")
            .output().unwrap();
        assert!(
            result.status.success(),
            "host {stage} failed: {}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
