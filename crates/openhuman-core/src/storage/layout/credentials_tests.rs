use super::*;
use crate::credentials::profiles::PROFILES_FILENAME;
use crate::security::secret_storage::{
    clear_master_key_source, install_master_key_source, MasterKeySource, SecretStorageError,
};
use crate::storage::layout::test_fixtures::{v1_profile, write, Fixture};
use crate::storage::layout::{migrate, restore, Options, Outcome};
use std::sync::Arc;
use tinystoragedrivers::secrets::DerivedKeys;
use zeroize::Zeroizing;

const PROFILES: &str = r#"{"schema_version":1,"updated_at":"2026-01-01T00:00:00Z","active_profiles":{},"profiles":{"openai:default":{"provider":"openai","profile_name":"default","kind":"token","token":"enc2:abcdef"}}}"#;
const HTTP: &str = r#"{"schema_version":1,"updated_at":"2026-01-01T00:00:00Z","credentials":{"gh":{"scheme":"bearer","secret":"enc2:012345","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}}}"#;

/// A host key source holding one fixed key, so no key is ever minted.
struct FixedKey;
impl MasterKeySource for FixedKey {
    fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>, SecretStorageError> {
        Ok(Some(Zeroizing::new([0x42; 32])))
    }
    fn store(&self, _: &[u8; 32]) -> Result<(), SecretStorageError> {
        Ok(())
    }
}

fn with_credentials() -> Fixture {
    let fx = v1_profile();
    write(&fx.target.profile_dir.join(PROFILES_FILENAME), PROFILES);
    write(&fx.target.profile_dir.join(STORE_FILENAME), HTTP);
    fx
}

async fn run(fx: &Fixture) -> Result<Outcome, LayoutError> {
    migrate(&fx.target, &fx.config, &Options::default()).await
}

/// The decrypted `local`-scope record `secret` in the profile's `storage.db`.
async fn stored(fx: &Fixture, secret: &str) -> Option<Vec<u8>> {
    let url = format!("sqlite:{}", fx.target.storage_db().display());
    let backend = storage::open(&url).await.unwrap();
    let scope = storage::tenant_scope(None);
    let scoped = backend.for_scope(&scope).unwrap();
    let secrets = storage::secrets::over(&scoped, scope, storage::secrets::keys().unwrap());
    secrets.get(secret).await.unwrap().map(|b| b.to_vec())
}

// Current-thread runtime: the key source slot is thread-local in unit tests.
#[tokio::test(flavor = "current_thread")]
async fn a_migration_moves_the_credential_files_into_encrypted_secrets() {
    install_master_key_source(Arc::new(FixedKey));
    let fx = with_credentials();
    let Outcome::Migrated(report) = run(&fx).await.unwrap() else {
        panic!("expected a migration");
    };
    clear_master_key_source();
    for &(file, secret) in FILES {
        assert!(
            !fx.target.profile_dir.join(file).exists(),
            "{file} is deleted once archived"
        );
        assert!(report.deleted.contains(&format!("{PROFILE_PREFIX}{file}")));
        assert_eq!(stored(&fx, secret).await.unwrap(), HTTP.as_bytes());
    }
    assert_eq!(
        std::fs::read_to_string(fx.target.profile_dir.join(PROFILES_FILENAME)).unwrap(),
        PROFILES,
        "auth profiles stay in their profile-local encrypted file"
    );
    // storage.db never holds the plaintext
    let db = std::fs::read(fx.target.storage_db()).unwrap();
    assert!(!db.windows(11).any(|w| w == b"openai:defa"));
    assert!(!db.windows(6).any(|w| w == b"\"gh\":{"));
    assert!(!db.windows(11).any(|w| w == b"enc2:abcdef"));

    // the archive keeps them and a restore puts them back beside config.toml
    restore(&fx.target, &report.archive).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(fx.target.profile_dir.join(PROFILES_FILENAME)).unwrap(),
        PROFILES
    );
    assert_eq!(
        std::fs::read_to_string(fx.target.profile_dir.join(STORE_FILENAME)).unwrap(),
        HTTP
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_stored_record_under_another_key_stops_the_migration_and_keeps_the_files() {
    install_master_key_source(Arc::new(FixedKey));
    let fx = with_credentials();
    let url = super::super::legacy::v1_url(&fx.target);
    let backend = storage::open(&url).await.unwrap();
    let scope = storage::tenant_scope(None);
    let scoped = backend.for_scope(&scope).unwrap();
    let other = Arc::new(DerivedKeys::new(Zeroizing::new([0x99; 32])));
    storage::secrets::over(&scoped, scope, other)
        .set(STORAGE_SECRET_NAME, b"{}")
        .await
        .unwrap();
    drop(backend);
    let error = run(&fx).await.unwrap_err();
    clear_master_key_source();
    assert!(matches!(error, LayoutError::Verify(_)), "{error}");
    assert!(fx.target.profile_dir.join(PROFILES_FILENAME).exists());
    assert!(fx.target.profile_dir.join(STORE_FILENAME).exists());
    assert!(!fx.target.profile_dir.join(".layout").exists());
}

#[tokio::test(flavor = "current_thread")]
async fn a_record_already_on_the_backend_wins_over_a_stale_file() {
    install_master_key_source(Arc::new(FixedKey));
    let fx = with_credentials();
    let url = super::super::legacy::v1_url(&fx.target);
    let backend = storage::open(&url).await.unwrap();
    let scope = storage::tenant_scope(None);
    let scoped = backend.for_scope(&scope).unwrap();
    storage::secrets::over(&scoped, scope, storage::secrets::keys().unwrap())
        .set(STORAGE_SECRET_NAME, br#"{"newer":true}"#)
        .await
        .unwrap();
    drop(backend);
    run(&fx).await.unwrap();
    clear_master_key_source();
    assert_eq!(
        stored(&fx, STORAGE_SECRET_NAME).await.unwrap(),
        br#"{"newer":true}"#
    );
    assert!(fx.target.profile_dir.join(PROFILES_FILENAME).exists());
    assert!(!fx.target.profile_dir.join(STORE_FILENAME).exists());
}

#[tokio::test(flavor = "current_thread")]
async fn a_file_that_is_not_a_credential_document_is_left_in_place() {
    install_master_key_source(Arc::new(FixedKey));
    let fx = v1_profile();
    write(
        &fx.target.profile_dir.join(PROFILES_FILENAME),
        r#"{"profiles":[]}"#,
    );
    write(&fx.target.profile_dir.join(STORE_FILENAME), HTTP);
    run(&fx).await.unwrap();
    clear_master_key_source();
    assert_eq!(
        std::fs::read_to_string(fx.target.profile_dir.join(PROFILES_FILENAME)).unwrap(),
        r#"{"profiles":[]}"#
    );
    assert!(!fx.target.profile_dir.join(STORE_FILENAME).exists());
}

#[test]
fn a_key_is_resolvable_only_from_a_source_that_does_not_mint() {
    use crate::security::secret_storage::encrypted_file_backend::resolvable;
    assert!(!resolvable(false, false, false, false));
    assert!(resolvable(true, false, false, false));
    assert!(resolvable(false, true, false, false));
    assert!(resolvable(false, false, true, false));
    assert!(resolvable(false, false, false, true));
}

#[test]
fn the_secret_names_match_the_stores_that_read_them() {
    assert_eq!(STORAGE_SECRET_NAME, "file:http-credentials.json");
    assert_eq!(PROFILES_FILENAME, "auth-profiles.json");
    assert_eq!(STORE_FILENAME, "http-credentials.json");
    assert_eq!(FILES, &[(STORE_FILENAME, STORAGE_SECRET_NAME)]);
}

#[tokio::test(flavor = "current_thread")]
async fn the_dry_run_lists_only_the_files_the_run_would_import() {
    install_master_key_source(Arc::new(FixedKey));
    let fx = v1_profile();
    write(&fx.target.profile_dir.join(PROFILES_FILENAME), "not json");
    write(&fx.target.profile_dir.join(STORE_FILENAME), HTTP);
    let options = Options {
        dry_run: true,
        ..Options::default()
    };
    let Outcome::DryRun(plan) = migrate(&fx.target, &fx.config, &options).await.unwrap() else {
        panic!("expected a dry run");
    };
    clear_master_key_source();
    let listed: Vec<_> = plan
        .delete_entries
        .iter()
        .map(|e| e.path.as_str())
        .collect();
    assert!(listed.contains(&"@profile/http-credentials.json"));
    assert!(!listed.contains(&"@profile/auth-profiles.json"));
}

#[tokio::test(flavor = "current_thread")]
async fn a_resumed_dry_run_lists_what_the_finished_import_recorded() {
    install_master_key_source(Arc::new(FixedKey));
    let fx = with_credentials();
    let mut state = crate::storage::layout::state::LayoutState::new("u1", 0);
    state.credentials = vec![STORE_FILENAME.to_owned()];
    state.done.push(crate::storage::layout::Step::Import);
    state.save(&fx.target.profile_dir).unwrap();
    let options = Options {
        dry_run: true,
        ..Options::default()
    };
    let Outcome::DryRun(plan) = migrate(&fx.target, &fx.config, &options).await.unwrap() else {
        panic!("expected a dry run");
    };
    clear_master_key_source();
    let listed: Vec<_> = plan
        .delete_entries
        .iter()
        .map(|e| e.path.as_str())
        .collect();
    assert!(listed.contains(&"@profile/http-credentials.json"));
    assert!(!listed.contains(&"@profile/auth-profiles.json"));
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn an_unreadable_credential_file_aborts_the_migration_before_v2_is_published() {
    use std::os::unix::fs::PermissionsExt;
    install_master_key_source(Arc::new(FixedKey));
    let fx = with_credentials();
    let path = fx.target.profile_dir.join(STORE_FILENAME);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&path).is_ok() {
        clear_master_key_source();
        return; // running as root: permissions do not apply
    }
    let result = run(&fx).await;
    clear_master_key_source();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(result.is_err());
    assert!(!fx.target.profile_dir.join(".layout").exists());
}
