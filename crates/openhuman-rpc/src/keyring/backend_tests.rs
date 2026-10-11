use super::*;
use std::any::Any;
use std::collections::HashMap;
use std::io::Write as _;
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;

use keyring::credential::{Credential, CredentialApi, CredentialBuilderApi};
use tinystoragedrivers::secrets::KeyringSecrets;

use crate::core_host::security::secret_storage::SecretBackend;

type Shared = Arc<Mutex<HashMap<(String, String), Vec<u8>>>>;

/// A credential store shared by every credential the builder makes, so a set
/// through one `Entry` is visible through the next, as on a real keychain.
#[derive(Debug, Default)]
struct FakeBuilder {
    store: Shared,
    locked: bool,
}

#[derive(Debug)]
struct FakeCredential {
    store: Shared,
    id: (String, String),
    locked: bool,
}

impl CredentialBuilderApi for FakeBuilder {
    fn build(
        &self,
        _t: Option<&str>,
        service: &str,
        user: &str,
    ) -> keyring::Result<Box<Credential>> {
        Ok(Box::new(FakeCredential {
            store: Arc::clone(&self.store),
            id: (service.to_string(), user.to_string()),
            locked: self.locked,
        }))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn denied() -> keyring::Error {
    keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("locked")))
}

impl CredentialApi for FakeCredential {
    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        if self.locked {
            return Err(denied());
        }
        self.store
            .lock()
            .unwrap()
            .insert(self.id.clone(), secret.to_vec());
        Ok(())
    }
    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        if self.locked {
            return Err(denied());
        }
        self.store
            .lock()
            .unwrap()
            .get(&self.id)
            .cloned()
            .ok_or(keyring::Error::NoEntry)
    }
    fn delete_credential(&self) -> keyring::Result<()> {
        if self.locked {
            return Err(denied());
        }
        self.store
            .lock()
            .unwrap()
            .remove(&self.id)
            .map(|_| ())
            .ok_or(keyring::Error::NoEntry)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A persistent fake OS keychain under the production service name.
pub(crate) fn fake_os_backend() -> OsKeyringBackend {
    OsKeyringBackend::with_store(KeyringSecrets::with_credential_builder(
        "openhuman",
        Box::new(FakeBuilder::default()),
    ))
}

fn locked_os_backend() -> OsKeyringBackend {
    OsKeyringBackend::with_store(KeyringSecrets::with_credential_builder(
        "openhuman",
        Box::new(FakeBuilder {
            locked: true,
            ..Default::default()
        }),
    ))
}

// ── OsKeyringBackend ────────────────────────────────────────────────────────────────

#[test]
fn os_backend_round_trips_through_the_port() {
    let os = fake_os_backend();
    assert_eq!(os.get("u:k").unwrap(), None);
    os.set("u:k", "v1").unwrap();
    os.set("u:k", "v2").unwrap();
    assert_eq!(os.get("u:k").unwrap().as_deref(), Some("v2"));
    os.delete("u:k").unwrap();
    os.delete("u:k").unwrap();
    assert_eq!(os.get("u:k").unwrap(), None);
    assert_eq!(os.name(), "os");
}

#[test]
fn os_backend_keeps_the_credential_layout() {
    // Service `openhuman`, user `{user_id}:{key}`, the password as the value:
    // what the previous direct `keyring::Entry` implementation wrote.
    let builder = FakeBuilder::default();
    let store = Arc::clone(&builder.store);
    let os = OsKeyringBackend::with_store(KeyringSecrets::with_credential_builder(
        "openhuman",
        Box::new(builder),
    ));
    os.set("alice:token", "s3cret").unwrap();
    let held = store.lock().unwrap();
    assert_eq!(
        held[&("openhuman".to_string(), "alice:token".to_string())],
        b"s3cret"
    );
}

#[test]
fn a_locked_keychain_reads_empty_and_deletes_quietly_but_fails_a_write() {
    let os = locked_os_backend();
    assert_eq!(os.get("u:k").unwrap(), None);
    os.delete("u:k").unwrap();
    let error = os.set("u:k", "v").unwrap_err();
    assert!(matches!(error, SecretStorageError::Backend(_)), "{error:?}");
}

#[test]
fn os_round_trip_get_set_delete() {
    if !os_keychain_available() {
        eprintln!("skip: OS keychain not available");
        return;
    }
    let b = OsKeyringBackend::new();
    let nk = "__openhuman_test__rtgsd:round_trip_key_001";
    let _ = b.delete(nk);

    assert!(b.get(nk).unwrap().is_none(), "absent before set");
    b.set(nk, "my_secret_value").unwrap();
    assert_eq!(b.get(nk).unwrap().as_deref(), Some("my_secret_value"));
    b.delete(nk).unwrap();
    assert!(b.get(nk).unwrap().is_none(), "absent after delete");
}

#[test]
fn os_delete_nonexistent_is_ok() {
    if !os_keychain_available() {
        eprintln!("skip: OS keychain not available");
        return;
    }
    OsKeyringBackend::new()
        .delete("__openhuman_test__del_ne:__nonexistent__")
        .expect("idempotent delete");
}

#[test]
fn os_user_id_isolation() {
    if !os_keychain_available() {
        eprintln!("skip: OS keychain not available");
        return;
    }
    let b = OsKeyringBackend::new();
    let nk_a = "__openhuman_test__user_a_iso:__shared_key_iso__";
    let nk_b = "__openhuman_test__user_b_iso:__shared_key_iso__";
    let _ = b.delete(nk_a);
    let _ = b.delete(nk_b);

    b.set(nk_a, "value_for_a").unwrap();
    b.set(nk_b, "value_for_b").unwrap();
    assert_eq!(b.get(nk_a).unwrap().as_deref(), Some("value_for_a"));
    assert_eq!(b.get(nk_b).unwrap().as_deref(), Some("value_for_b"));
    assert_ne!(b.get(nk_a).unwrap(), b.get(nk_b).unwrap());

    let _ = b.delete(nk_a);
    let _ = b.delete(nk_b);
}

#[test]
fn availability_probe_handles_a_leftover_os_entry_and_caches_success() {
    use crate::core_host::security::secret_storage::{
        clear_secret_backend, install_secret_backend, is_available,
    };
    let b = Arc::new(fake_os_backend());
    let probe_key = "__probe__:__openhuman_keyring_probe__";
    b.set(probe_key, "__probe_value__").unwrap();
    install_secret_backend(b.clone());
    assert!(is_available());
    assert_eq!(
        b.get(probe_key).unwrap(),
        None,
        "probe must remove its entry"
    );
    assert!(is_available());
    clear_secret_backend();
}

#[test]
fn migrate_from_file_happy_path_os() {
    if !os_keychain_available() {
        eprintln!("skip: OS keychain not available");
        return;
    }
    let b = OsKeyringBackend::new();
    let user_id = "__openhuman_test__mig_hp";
    let key = "__migrate_key_hp__";
    let nk = format!("{user_id}:{key}");
    let _ = b.delete(&nk);

    let mut tmp = NamedTempFile::new().expect("temp file");
    write!(tmp, "  migrated_secret_value  ").expect("write temp");
    let path = tmp.path().to_path_buf();
    let _ = tmp.keep();

    // Test the OsBackend directly.
    let value = std::fs::read_to_string(&path).unwrap();
    let value = value.trim();
    b.set(&nk, value).unwrap();
    assert_eq!(
        b.get(&nk).unwrap().as_deref(),
        Some("migrated_secret_value")
    );
    std::fs::remove_file(&path).unwrap();
    let _ = b.delete(&nk);
}

fn os_keychain_available() -> bool {
    if std::env::var("OPENHUMAN_TEST_OS_KEYCHAIN").as_deref() != Ok("1") {
        return false;
    }
    let b = OsKeyringBackend::new();
    let probe_key = "__openhuman_probe_test__";
    let probe_val = "__probe_ok__";
    if b.set(probe_key, probe_val).is_err() {
        return false;
    }
    let ok = b.get(probe_key).ok().flatten().as_deref() == Some(probe_val);
    let _ = b.delete(probe_key);
    ok
}
