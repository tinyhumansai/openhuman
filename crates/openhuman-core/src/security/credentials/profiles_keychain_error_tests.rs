//! A keychain read error is not a missing secret. Treating it as one dropped
//! keychain-only OAuth profiles and persisted the drop, and every write
//! rebuilds the whole store (keychain entries included) from that partial view.

use super::*;
use crate::security::secret_storage::{SecretBackend, SecretStorageError};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

/// In-memory keychain whose reads can be made to fail (a locked or denied keychain).
#[derive(Default)]
struct FlakyKeychain {
    map: Mutex<HashMap<String, String>>,
    fail_reads: AtomicBool,
}

impl SecretBackend for FlakyKeychain {
    fn get(&self, key: &str) -> Result<Option<String>, SecretStorageError> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(SecretStorageError::Backend("keychain locked".into()));
        }
        Ok(self.map.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<(), SecretStorageError> {
        self.map.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), SecretStorageError> {
        self.map.lock().unwrap().remove(key);
        Ok(())
    }
    fn name(&self) -> &'static str {
        "flaky-keychain"
    }
}

struct Fixture {
    keychain: Arc<FlakyKeychain>,
    store: AuthProfilesStore,
    oauth_id: String,
    _tmp: TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        crate::security::secret_storage::clear_secret_backend();
    }
}

/// A store holding one keychain-only OAuth profile and one Token profile,
/// with keychain reads failing from here on.
fn locked_keychain_store() -> Fixture {
    let keychain = Arc::new(FlakyKeychain::default());
    crate::security::secret_storage::install_secret_backend(keychain.clone());
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), true);
    let oauth = AuthProfile::new_oauth(
        "openai-codex",
        "default",
        TokenSet {
            access_token: "kc-access".into(),
            refresh_token: Some("kc-refresh".into()),
            id_token: None,
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            token_type: Some("Bearer".into()),
            scope: None,
        },
    );
    let oauth_id = oauth.id.clone();
    store.upsert_profile(oauth, true).unwrap();
    store
        .upsert_profile(
            AuthProfile::new_token("anthropic", "default", "kc-token".into()),
            true,
        )
        .unwrap();
    let json = std::fs::read_to_string(store.path()).unwrap();
    assert!(
        json.contains(&oauth_id) && !json.contains("kc-access") && !json.contains("kc-token"),
        "precondition: secrets live only in the keychain"
    );
    keychain.fail_reads.store(true, Ordering::SeqCst);
    Fixture {
        keychain,
        store,
        oauth_id,
        _tmp: tmp,
    }
}

#[test]
fn keychain_read_error_fails_the_load_and_keeps_the_profile() {
    let f = locked_keychain_store();
    let json_before = std::fs::read_to_string(f.store.path()).unwrap();

    assert!(
        f.store.load().is_err(),
        "a keychain read error must fail the load"
    );

    assert_eq!(
        std::fs::read_to_string(f.store.path()).unwrap(),
        json_before
    );
    f.keychain.fail_reads.store(false, Ordering::SeqCst);
    let data = f.store.load().expect("load after the keychain unlocks");
    assert!(
        data.profiles.contains_key(&f.oauth_id),
        "profile was purged"
    );
}

#[test]
fn keychain_read_error_refuses_writes_and_keeps_every_secret() {
    let f = locked_keychain_store();
    let json_before = std::fs::read_to_string(f.store.path()).unwrap();
    let keychain_before = f.keychain.map.lock().unwrap().clone();

    let upsert = f.store.upsert_profile(
        AuthProfile::new_token("openrouter", "default", "new-token".into()),
        false,
    );

    assert!(
        upsert.is_err(),
        "a write from a partial view must be refused"
    );
    assert_eq!(
        std::fs::read_to_string(f.store.path()).unwrap(),
        json_before
    );
    assert_eq!(
        *f.keychain.map.lock().unwrap(),
        keychain_before,
        "keychain entries were rewritten from a partial view"
    );
}

#[test]
fn keychain_read_error_with_undecryptable_json_keeps_the_profile() {
    // The JSON still names an access token for the OAuth profile, but it
    // cannot be decrypted: falling back to it would drop a profile whose
    // secret the keychain may still hold. Only that profile is kept, so the
    // keychain-only Token profile cannot fail the load on its own.
    let f = locked_keychain_store();
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(f.store.path()).unwrap()).unwrap();
    let profiles = json["profiles"].as_object_mut().unwrap();
    profiles.retain(|id, _| id == &f.oauth_id);
    profiles[&f.oauth_id]["access_token"] = "enc2:not-decryptable".into();
    json["active_profiles"] = serde_json::json!({});
    std::fs::write(f.store.path(), json.to_string()).unwrap();
    let json_before = std::fs::read_to_string(f.store.path()).unwrap();

    assert!(
        f.store.load().is_err(),
        "an undecryptable fallback must fail the load"
    );
    assert_eq!(
        std::fs::read_to_string(f.store.path()).unwrap(),
        json_before
    );
}

fn persisted_entry(store: &AuthProfilesStore, id: &str) -> serde_json::Value {
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.path()).unwrap()).unwrap();
    json["profiles"][id].clone()
}

#[test]
fn an_unavailable_keychain_carries_keychain_only_profiles_untouched() {
    // A keychain that is locked when the store is built turns the keychain off
    // for that store (`AuthProfilesStore::new`'s availability probe), so its
    // reads never reach the keychain and a keychain-only OAuth profile resolves
    // to no access token. It is carried, not purged, through loads and writes.
    let mut f = locked_keychain_store();
    f.store.use_keychain = false;
    let entry_before = persisted_entry(&f.store, &f.oauth_id);

    let data = f.store.load().expect("load without the keychain");
    assert!(
        !data.profiles.contains_key(&f.oauth_id),
        "an unresolved profile is not handed out"
    );
    f.store
        .upsert_profile(
            AuthProfile::new_token("openrouter", "default", "new-token".into()),
            false,
        )
        .expect("a write without the keychain");
    assert_eq!(
        persisted_entry(&f.store, &f.oauth_id),
        entry_before,
        "the keychain-only profile was dropped or rewritten"
    );

    f.store.use_keychain = true;
    f.keychain.fail_reads.store(false, Ordering::SeqCst);
    let data = f.store.load().expect("load once the keychain reads");
    assert_eq!(
        data.profiles[&f.oauth_id]
            .token_set
            .as_ref()
            .map(|t| t.access_token.as_str()),
        Some("kc-access")
    );
}
