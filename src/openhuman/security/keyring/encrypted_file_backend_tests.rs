use super::*;
use std::cell::{Cell, RefCell};

/// In-memory fake of the keychain entry, so [`load_or_mint_master_key`] can
/// be exercised without a real OS keychain. `absent_error` is a fn pointer
/// because `keyring::Error` is not `Clone` — we mint a fresh error per call.
/// These tests touch no process-wide state (`load_or_mint_master_key` never
/// reads `MASTER_KEY`), so no OnceLock reset seam is needed.
struct FakeEntry {
    stored: RefCell<Option<String>>,
    absent_error: fn() -> keyring::Error,
    set_calls: Cell<usize>,
}

impl FakeEntry {
    fn with_stored(value: &str) -> Self {
        Self {
            stored: RefCell::new(Some(value.to_string())),
            absent_error: || keyring::Error::NoEntry,
            set_calls: Cell::new(0),
        }
    }
    fn absent(err: fn() -> keyring::Error) -> Self {
        Self {
            stored: RefCell::new(None),
            absent_error: err,
            set_calls: Cell::new(0),
        }
    }
}

impl MasterKeyEntry for FakeEntry {
    fn get_password(&self) -> Result<String, keyring::Error> {
        match &*self.stored.borrow() {
            Some(v) => Ok(v.clone()),
            None => Err((self.absent_error)()),
        }
    }
    fn set_password(&self, value: &str) -> Result<(), keyring::Error> {
        self.set_calls.set(self.set_calls.get() + 1);
        *self.stored.borrow_mut() = Some(value.to_string());
        Ok(())
    }
}

fn access_denied() -> keyring::Error {
    keyring::Error::NoStorageAccess(Box::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "keychain access denied",
    )))
}

fn platform_failure() -> keyring::Error {
    keyring::Error::PlatformFailure(Box::new(std::io::Error::other("platform boom")))
}

#[test]
fn loads_existing_key_without_minting() {
    let hex = "ab".repeat(KEY_LEN); // 32 bytes of 0xab
    let entry = FakeEntry::with_stored(&hex);
    let key = load_or_mint_master_key(&entry).expect("should load existing key");
    assert_eq!(key, [0xabu8; KEY_LEN]);
    assert_eq!(entry.set_calls.get(), 0, "must not overwrite existing key");
}

#[test]
fn mints_only_on_no_entry() {
    let entry = FakeEntry::absent(|| keyring::Error::NoEntry);
    let key = load_or_mint_master_key(&entry).expect("should mint when genuinely absent");
    assert_ne!(key, [0u8; KEY_LEN], "minted key should be random, not zero");
    assert_eq!(
        entry.set_calls.get(),
        1,
        "should store the freshly minted key"
    );
    // The key is now persisted, so a second load returns the same one.
    assert!(entry.stored.borrow().is_some());
}

#[test]
fn does_not_mint_on_access_denied() {
    // The #3311 case: existing key unreadable due to post-update ACL change.
    let entry = FakeEntry::absent(access_denied);
    let result = load_or_mint_master_key(&entry);
    assert!(result.is_err(), "access denial must NOT mint a new key");
    assert_eq!(
        entry.set_calls.get(),
        0,
        "must never call set_password on access denial — that orphans existing secrets"
    );
    assert!(
        entry.stored.borrow().is_none(),
        "keychain entry left untouched"
    );
}

#[test]
fn does_not_mint_on_platform_failure() {
    // Variant-independence: any non-NoEntry error fails safe, not just
    // NoStorageAccess (the exact macOS denial variant is unconfirmed).
    let entry = FakeEntry::absent(platform_failure);
    let result = load_or_mint_master_key(&entry);
    assert!(result.is_err(), "platform failure must NOT mint a new key");
    assert_eq!(entry.set_calls.get(), 0);
}

#[test]
fn rejects_wrong_length_key_without_minting() {
    let entry = FakeEntry::with_stored("abcd"); // 2 bytes, not KEY_LEN
    let result = load_or_mint_master_key(&entry);
    assert!(result.is_err(), "wrong-length stored key is an error");
    assert_eq!(
        entry.set_calls.get(),
        0,
        "must not overwrite on length mismatch"
    );
}
