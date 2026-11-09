use super::*;
use crate::security::secret_storage::backend::SecretBackend;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

struct RacingBackend {
    values: Mutex<HashMap<String, String>>,
    lock_path: std::path::PathBuf,
}

impl SecretBackend for RacingBackend {
    fn get(&self, name: &str) -> Result<Option<String>, SecretStorageError> {
        let sidecar = crate::security::secret_storage::file_store::lock_path_for(&self.lock_path);
        let lock_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(sidecar)
            .expect("the cross-process lock must exist before reading the key");
        assert!(
            fs2::FileExt::try_lock_exclusive(&lock_file).is_err(),
            "the cross-process lock must remain held while reading the key"
        );
        // Widen the old get/set race window without introducing a timing
        // assertion into the test itself.
        std::thread::yield_now();
        let result = self.values.lock().unwrap().get(name).cloned();
        Ok(result)
    }

    fn set(&self, name: &str, value: &str) -> Result<(), SecretStorageError> {
        self.values
            .lock()
            .unwrap()
            .insert(name.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), SecretStorageError> {
        self.values.lock().unwrap().remove(name);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "racing-test"
    }
}

#[test]
fn concurrent_random_key_creation_returns_the_persisted_value_to_every_caller() {
    let dir = TempDir::new().unwrap();
    let lock_path = dir.path().join("key-lock");
    let backend = Arc::new(RacingBackend {
        values: Mutex::new(HashMap::new()),
        lock_path: lock_path.clone(),
    });
    let workers: Vec<_> = (0..24)
        .map(|_| {
            let backend = backend.clone();
            let lock_path = lock_path.clone();
            std::thread::spawn(move || {
                get_or_create_random_with_backend(backend.as_ref(), "u", "k", 32, &lock_path)
                    .unwrap()
            })
        })
        .collect();
    let first = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(first[0], first[1]);
    assert_eq!(backend.values.lock().unwrap().get("u:k"), Some(&first[0]));
}

#[test]
fn blank_migration_source_is_rejected_and_retained() {
    let dir = TempDir::new().unwrap();
    let backend = crate::security::secret_storage::backend::FileBackend::new(dir.path());
    let source = dir.path().join(".secret_key");
    std::fs::write(&source, " \n\t").unwrap();

    let result = migrate_from_file_with_backend("u", "master", &source, &backend);
    assert!(result.is_err(), "blank key material must not be migrated");
    assert_eq!(std::fs::read_to_string(&source).unwrap(), " \n\t");
    assert_eq!(backend.get("u:master").unwrap(), None);
}
