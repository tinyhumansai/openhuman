use super::*;
use crate::security::secret_storage::file_store;
use std::sync::Mutex;

#[derive(Default)]
struct FakeSource {
    key: Mutex<Option<[u8; 32]>>,
    writes: Mutex<usize>,
    denied: bool,
    wrong_readback: bool,
}
impl MasterKeySource for FakeSource {
    fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>, SecretStorageError> {
        if self.denied {
            return Err(SecretStorageError::Backend("access denied".into()));
        }
        let key = *self.key.lock().unwrap();
        Ok(key.map(|key| Zeroizing::new(if self.wrong_readback { [9; 32] } else { key })))
    }
    fn store(&self, key: &[u8; 32]) -> Result<(), SecretStorageError> {
        *self.writes.lock().unwrap() += 1;
        *self.key.lock().unwrap() = Some(*key);
        Ok(())
    }
}

#[test]
fn loads_existing_host_key_without_minting() {
    let source = FakeSource {
        key: Mutex::new(Some([0xab; 32])),
        ..Default::default()
    };
    assert_eq!(*load_or_mint_master_key(&source).unwrap(), [0xab; 32]);
    assert_eq!(*source.writes.lock().unwrap(), 0);
}

#[test]
fn mints_and_verifies_only_on_genuine_absence() {
    let source = FakeSource::default();
    let first = load_or_mint_master_key(&source).unwrap();
    assert_ne!(*first, [0; 32]);
    assert_eq!(*source.writes.lock().unwrap(), 1);
    assert_eq!(*first, *load_or_mint_master_key(&source).unwrap());
}

#[test]
fn unavailable_host_source_never_mints_a_replacement() {
    let source = FakeSource {
        denied: true,
        ..Default::default()
    };
    assert!(load_or_mint_master_key(&source).is_err());
    assert_eq!(*source.writes.lock().unwrap(), 0);
}

#[test]
fn host_key_write_requires_matching_readback() {
    let source = FakeSource {
        wrong_readback: true,
        ..Default::default()
    };
    assert!(load_or_mint_master_key(&source)
        .unwrap_err()
        .to_string()
        .contains("verification failed"));
}

#[test]
fn master_key_file_read_is_limited_to_4096_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("master.key");
    std::fs::write(&path, vec![b'a'; 4096]).unwrap();
    assert_eq!(read_master_key_file(&path).unwrap().len(), 4096);

    std::fs::write(&path, vec![b'a'; 4097]).unwrap();
    assert!(read_master_key_file(&path)
        .unwrap_err()
        .contains("4096-byte limit"));
}

#[cfg(unix)]
#[test]
fn master_key_file_rejects_fifo_without_waiting_for_a_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("master-key.fifo");
    let status = std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .expect("mkfifo is available on Unix test hosts");
    assert!(status.success());

    let error = read_master_key_file(&path).unwrap_err();
    assert!(error.contains("regular file"));
}

#[test]
fn workspace_key_is_created_private_and_reused() {
    let dir = tempfile::tempdir().unwrap();
    let first = workspace_master_key(dir.path()).unwrap();
    assert_ne!(*first, [0; 32]);
    let path = dir.path().join("keyring/master.key");
    assert_eq!(
        parse_master_key_hex(&std::fs::read_to_string(&path).unwrap()).unwrap(),
        *first
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert_eq!(*first, *workspace_master_key(dir.path()).unwrap());
}

#[test]
fn concurrent_workspace_initialization_reuses_one_key() {
    let dir = tempfile::tempdir().unwrap();
    let keys = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| workspace_master_key(dir.path()).unwrap()))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(keys.iter().all(|key| **key == *keys[0]));
}

#[test]
fn corrupt_workspace_key_is_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keyring/master.key");
    file_store::write_atomic(&path, b"invalid-key").unwrap();
    assert!(workspace_master_key(dir.path()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"invalid-key");
}

#[cfg(unix)]
#[test]
fn workspace_key_rejects_shared_permissions_and_symlinks() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keyring/master.key");
    workspace_master_key(dir.path()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(workspace_master_key(dir.path()).is_err());
    std::fs::remove_file(&path).unwrap();
    let other = dir.path().join("other");
    std::fs::write(&other, "ab".repeat(32)).unwrap();
    symlink(&other, &path).unwrap();
    assert!(workspace_master_key(dir.path()).is_err());
}

// These exercise the enabled backend's resolver (`try_load_master_key`),
// which a build without `secret-file` does not have: this file is also
// mounted under the disabled stub.
#[cfg(feature = "secret-file")]
#[test]
fn installed_source_wins_over_workspace_file_and_does_not_leak_threads() {
    let _env = crate::config::test_env::EnvVarGuard::locked()
        .without(MASTER_KEY_ENV)
        .without(MASTER_KEY_FILE_ENV);
    let dir = tempfile::tempdir().unwrap();
    let _workspace =
        crate::security::secret_storage::store::test_scope::ScopedWorkspace::new(dir.path());
    install_master_key_source(std::sync::Arc::new(FakeSource {
        key: Mutex::new(Some([7; 32])),
        ..Default::default()
    }));
    assert_eq!(super::super::try_load_master_key().unwrap().0, [7; 32]);
    assert!(!dir.path().join("keyring/master.key").exists());
    assert!(
        std::thread::spawn(|| installed_master_key_source().is_none())
            .join()
            .unwrap()
    );
    clear_master_key_source();
    assert!(super::super::try_load_master_key().is_ok());
    assert!(dir.path().join("keyring/master.key").exists());
}

#[cfg(feature = "secret-file")]
#[test]
fn configured_env_key_precedes_host_source_and_errors_never_fall_through() {
    let _env = crate::config::test_env::EnvVarGuard::locked()
        .with(MASTER_KEY_ENV, "ab".repeat(32))
        .without(MASTER_KEY_FILE_ENV);
    let source = std::sync::Arc::new(FakeSource {
        denied: true,
        ..Default::default()
    });
    install_master_key_source(source.clone());
    assert_eq!(super::super::try_load_master_key().unwrap().0, [0xab; 32]);
    let _bad = crate::config::test_env::EnvVarGuard::set(MASTER_KEY_ENV, "invalid");
    assert!(matches!(
        super::super::try_load_master_key(),
        Err(super::super::MasterKeyError::Configured(_))
    ));
    assert_eq!(*source.writes.lock().unwrap(), 0);
    clear_master_key_source();
}

#[cfg(feature = "secret-file")]
#[test]
fn denied_host_source_does_not_fall_back_to_workspace_key() {
    let _env = crate::config::test_env::EnvVarGuard::locked()
        .without(MASTER_KEY_ENV)
        .without(MASTER_KEY_FILE_ENV);
    let dir = tempfile::tempdir().unwrap();
    let _workspace =
        crate::security::secret_storage::store::test_scope::ScopedWorkspace::new(dir.path());
    install_master_key_source(std::sync::Arc::new(FakeSource {
        denied: true,
        ..Default::default()
    }));
    assert!(super::super::try_load_master_key().is_err());
    assert!(!dir.path().join("keyring/master.key").exists());
    clear_master_key_source();
}
