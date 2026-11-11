use super::*;

#[tokio::test]
async fn persisted_profile_document_is_encrypted_at_rest() {
    let tmp = tempfile::TempDir::new().unwrap();
    // The profile document always encrypts at rest; native per-field cipher
    // approval is exercised separately by the API-key/native boundary test.
    let store = AuthProfilesStore::new(tmp.path(), false);
    store
        .upsert_profile(
            AuthProfile::new_token("openai", "default", "profile-secret-token".into()),
            true,
        )
        .await
        .unwrap();

    let bytes = std::fs::read(store.path()).expect("encrypted profile document");
    assert!(!String::from_utf8_lossy(&bytes).contains("profile-secret-token"));
    assert!(serde_json::from_slice::<PersistedAuthProfiles>(&bytes).is_err());
    assert_eq!(
        store.load().await.unwrap().profiles["openai:default"]
            .token
            .as_deref(),
        Some("profile-secret-token")
    );
}

#[tokio::test]
async fn configured_generic_storage_does_not_replace_the_profile_local_document() {
    let _backend = crate::storage::test_support::install_backend_for_test(
        crate::storage::test_support::memory_backend(),
    );
    let tmp = tempfile::TempDir::new().unwrap();
    // The profile document always encrypts at rest; native per-field cipher
    // approval is exercised separately by the API-key/native boundary test.
    let store = AuthProfilesStore::new(tmp.path(), false);
    store
        .upsert_profile(
            AuthProfile::new_token("openai", "default", "local-secret".into()),
            true,
        )
        .await
        .unwrap();

    let bytes = std::fs::read(store.path()).expect("profile-local encrypted document");
    assert!(!String::from_utf8_lossy(&bytes).contains("local-secret"));
    assert_eq!(
        store.load().await.unwrap().profiles["openai:default"]
            .token
            .as_deref(),
        Some("local-secret")
    );
}

/// A host key source holding one fixed key, so the storage master key is
/// never minted or read from the developer's keychain.
struct FixedStorageKey;
impl crate::security::secret_storage::MasterKeySource for FixedStorageKey {
    fn load(
        &self,
    ) -> Result<Option<zeroize::Zeroizing<[u8; 32]>>, crate::security::secret_storage::SecretStorageError>
    {
        Ok(Some(zeroize::Zeroizing::new([0x42; 32])))
    }
    fn store(&self, _: &[u8; 32]) -> Result<(), crate::security::secret_storage::SecretStorageError> {
        Ok(())
    }
}

/// A context serving SaaS tenant `profile`, as the profile host binds one.
fn tenant_context(
    workspace: &Path,
    profile: &str,
) -> std::sync::Arc<crate::core::runtime::CoreContext> {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.to_path_buf();
    CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(config, DomainSet::full(), ToolGroups::none()).profile(profile),
    )
}

// Current-thread runtime: the key source slot is thread-local in unit tests.
#[tokio::test(flavor = "current_thread")]
async fn a_tenant_store_keeps_its_document_and_lock_in_the_configured_backend() {
    crate::security::secret_storage::install_master_key_source(std::sync::Arc::new(
        FixedStorageKey,
    ));
    let backend = crate::storage::test_support::memory_backend();
    let _installed =
        crate::storage::test_support::install_backend_for_test(std::sync::Arc::clone(&backend));
    let root = tempfile::TempDir::new().unwrap();
    let tenant_dir = root.path().join("users").join("alice");
    std::fs::create_dir_all(tenant_dir.join("workspace")).unwrap();

    let ctx = tenant_context(&tenant_dir.join("workspace"), "alice");
    let loaded = crate::core::runtime::CoreContext::scope(ctx, async {
        let store = AuthProfilesStore::new(&tenant_dir, false);
        store
            .upsert_profile(
                AuthProfile::new_token("openai", "default", "tenant-secret".into()),
                true,
            )
            .await
            .unwrap();
        store.load().await.unwrap()
    })
    .await;
    crate::security::secret_storage::clear_master_key_source();

    assert_eq!(
        loaded.profiles["openai:default"].token.as_deref(),
        Some("tenant-secret")
    );
    let beside: Vec<String> = std::fs::read_dir(&tenant_dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !beside.iter().any(|name| name.contains("auth")),
        "the backend owns a tenant's auth profiles and their lock: {beside:?}"
    );
    let scope = crate::storage::scope_for_profile("alice");
    let secrets = crate::storage::secrets::over(
        &backend.for_scope(&scope.to_storage_scope()).unwrap(),
        scope,
        crate::storage::secrets::keys().unwrap(),
    );
    let stored = crate::storage::secrets::get_blocking(&secrets, PROFILE_DOCUMENT_SECRET_NAME)
        .unwrap()
        .expect("the document lives in the tenant's scope on the backend");
    assert!(!String::from_utf8_lossy(&stored).is_empty());
}
