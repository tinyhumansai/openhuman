use super::{effective_backend_kind_for, BackendKind};

#[test]
fn explicit_file_backend_wins_over_staging_environment() {
    assert_eq!(
        effective_backend_kind_for(Some("staging"), Some("file"), false),
        BackendKind::File
    );
}

#[test]
fn explicit_encrypted_file_backend_wins_in_dev_environment() {
    assert_eq!(
        effective_backend_kind_for(Some("development"), Some("encrypted_file"), false),
        BackendKind::EncryptedFile
    );
}

#[test]
fn standalone_and_library_hosts_default_to_memory_in_every_environment() {
    for app_env in [
        None,
        Some("development"),
        Some("staging"),
        Some("production"),
    ] {
        for test_build in [false, true] {
            assert_eq!(
                effective_backend_kind_for(app_env, None, test_build),
                BackendKind::Memory
            );
            assert_eq!(
                effective_backend_kind_for(app_env, Some("bogus"), test_build),
                BackendKind::Memory
            );
        }
    }
}

#[test]
fn explicit_memory_backend_wins_in_every_environment() {
    assert_eq!(
        effective_backend_kind_for(Some("production"), Some(" MEMORY "), false),
        BackendKind::Memory
    );
}

#[test]
fn default_memory_backend_needs_no_workspace_files_or_master_key() {
    let _env = crate::config::test_env::EnvVarGuard::locked().without("OPENHUMAN_KEYRING_BACKEND");
    let dir = tempfile::tempdir().unwrap();
    let _workspace = super::test_scope::ScopedWorkspace::new(dir.path());
    assert_eq!(super::build_backend_at(dir.path()).name(), "memory");
    assert!(crate::security::secret_storage::is_available());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn installed_backend_takes_precedence_and_is_thread_local() {
    use crate::security::secret_storage::SecretBackend;
    use crate::security::secret_storage::{self, backend::MockBackend};
    use std::sync::Arc;
    let _env = crate::config::test_env::EnvVarGuard::locked().without("OPENHUMAN_KEYRING_BACKEND");
    let dir = tempfile::tempdir().unwrap();
    let _workspace = super::test_scope::ScopedWorkspace::new(dir.path());
    let installed = Arc::new(MockBackend::new());
    secret_storage::install_secret_backend(installed.clone());
    let backend = super::build_backend_at(dir.path());
    backend.set("user:key", "host-value").unwrap();
    assert_eq!(
        installed.get("user:key").unwrap().as_deref(),
        Some("host-value")
    );
    assert_eq!(secret_storage::backend_name(), "mock");
    assert!(secret_storage::is_available());
    assert!(
        !dir.path().join("keyring/master.key").exists(),
        "probing a host backend must not initialize a file key"
    );
    assert!(
        std::thread::spawn(|| secret_storage::installed_secret_backend().is_none())
            .join()
            .unwrap()
    );
    secret_storage::clear_secret_backend();
    assert_eq!(super::build_backend_at(dir.path()).name(), "memory");
}

#[test]
fn explicit_os_backend_requires_a_host_provider() {
    let _env =
        crate::config::test_env::EnvVarGuard::locked().with("OPENHUMAN_KEYRING_BACKEND", "os");
    let dir = tempfile::tempdir().unwrap();
    assert!(super::validate_backend_selection()
        .unwrap_err()
        .contains("host-installed OS"));
    let backend = super::build_backend_at(dir.path());
    assert!(backend
        .get("user:key")
        .unwrap_err()
        .to_string()
        .contains("host-installed OS"));
    assert!(backend.set("user:key", "value").is_err());
    assert!(backend.delete("user:key").is_err());
}
