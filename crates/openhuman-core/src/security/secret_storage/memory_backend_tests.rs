use super::*;

#[test]
fn memory_instances_and_user_namespaces_are_independent() {
    let backend = MemoryBackend::new();
    backend.set("alice:token", "alice-secret").unwrap();
    backend.set("bob:token", "bob-secret").unwrap();
    backend.delete("alice:token").unwrap();
    assert_eq!(
        backend.get("bob:token").unwrap().as_deref(),
        Some("bob-secret")
    );
    assert_eq!(MemoryBackend::new().get("bob:token").unwrap(), None);
}

#[test]
fn memory_secrets_are_shared_across_threads_without_lost_writes() {
    let backend = std::sync::Arc::new(MemoryBackend::new());
    let workers: Vec<_> = (0..8)
        .map(|worker| {
            let backend = backend.clone();
            std::thread::spawn(move || backend.set(&format!("user:{worker}"), "value").unwrap())
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    for worker in 0..8 {
        assert_eq!(
            backend.get(&format!("user:{worker}")).unwrap().as_deref(),
            Some("value")
        );
    }
}

#[test]
fn migration_to_memory_never_deletes_a_durable_source() {
    let _env = crate::config::test_env::EnvVarGuard::locked().without("OPENHUMAN_KEYRING_BACKEND");
    let dir = tempfile::tempdir().unwrap();
    let _workspace =
        crate::security::secret_storage::store::test_scope::ScopedWorkspace::new(dir.path());
    let source = dir.path().join("legacy-secret");
    std::fs::write(&source, "durable-value").unwrap();
    let error =
        crate::security::secret_storage::migrate_from_file("user", "token", &source).unwrap_err();
    assert!(error.to_string().contains("persistent"));
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "durable-value");
}
