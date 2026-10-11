//! Library secret storage needs no OS provider, environment setup or key files.

mod common;

use common::{offline_config, runtime_without_master_key};
use openhuman_embed::{RuntimeBuilder, ServiceSet, Workspace};

#[test]
fn embedded_secrets_default_to_memory_without_key_files() {
    std::thread::Builder::new()
        .stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            runtime_without_master_key().block_on(async {
                let root = tempfile::tempdir().unwrap();
                let mut config = offline_config();
                config.workspace_dir = root.path().join("workspace");
                config.config_path = root.path().join("config.toml");
                let workspace = config.workspace_dir.clone();
                let runtime = RuntimeBuilder::lean()
                    .workspace(Workspace::Dir(workspace.clone()))
                    .config(config)
                    .services(ServiceSet::none())
                    .build()
                    .await
                    .unwrap();
                use openhuman_core::security::secret_storage;
                use openhuman_embed::secret_storage as providers;
                assert!(providers::installed_secret_backend().is_none());
                assert_eq!(secret_storage::backend_name(), "memory");
                assert!(secret_storage::is_available());
                assert_eq!(
                    openhuman_core::security::keyring_consent::policy::current_status()
                        .active_mode
                        .to_string(),
                    "in_memory"
                );
                secret_storage::set("embed-user", "token", "memory-value").unwrap();
                assert_eq!(
                    secret_storage::get("embed-user", "token")
                        .unwrap()
                        .as_deref(),
                    Some("memory-value")
                );
                secret_storage::delete("embed-user", "token").unwrap();
                assert_eq!(secret_storage::get("embed-user", "token").unwrap(), None);
                for name in ["keyring/master.key", "secrets.enc", "dev-keychain.json"] {
                    assert!(
                        !workspace.join(name).exists(),
                        "in-memory storage must not write {name}"
                    );
                }
                let legacy_dir = workspace.join("legacy-secrets");
                std::fs::create_dir_all(&legacy_dir).unwrap();
                let legacy_key = legacy_dir.join(".secret_key");
                let key_bytes = "01".repeat(32);
                std::fs::write(&legacy_key, &key_bytes).unwrap();
                let store = secret_storage::SecretStore::new(&legacy_dir, true);
                let ciphertext = store.encrypt("legacy-compatible").unwrap();
                assert_eq!(store.decrypt(&ciphertext).unwrap(), "legacy-compatible");
                assert_eq!(
                    std::fs::read_to_string(&legacy_key).unwrap(),
                    key_bytes,
                    "volatile storage must leave a legacy durable key intact"
                );
                drop(runtime);
            })
        })
        .expect("test thread")
        .join()
        .expect("test thread should not panic");
}
