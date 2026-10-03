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

// ── Operator-supplied master key (#6926) ─────────────────────────────────────

fn hex_key(byte: u8) -> String {
    format!("{byte:02x}").repeat(KEY_LEN)
}

fn file_must_not_be_read(path: &Path) -> Result<String, String> {
    panic!("master key file {} must not be read", path.display())
}

#[test]
fn env_inline_key_is_used_and_names_its_source() {
    let hex = hex_key(0xab);
    let (key, source) = master_key_from_env(Some(&hex), None, file_must_not_be_read)
        .expect("valid inline key")
        .expect("inline key counts as supplied");
    assert_eq!(key, [0xabu8; KEY_LEN]);
    assert_eq!(source, MASTER_KEY_ENV);
}

#[test]
fn env_inline_key_tolerates_surrounding_whitespace_and_uppercase_hex() {
    let hex = format!("  {}\n", hex_key(0xcd).to_uppercase());
    let (key, _) = master_key_from_env(Some(&hex), None, file_must_not_be_read)
        .expect("trimmed key is valid")
        .expect("supplied");
    assert_eq!(key, [0xcdu8; KEY_LEN]);
}

#[test]
fn env_file_key_is_read_trimmed_and_names_the_path() {
    let hex = hex_key(0x11);
    let (key, source) =
        master_key_from_env(None, Some("/run/secrets/openhuman_master_key"), |path| {
            assert_eq!(path, Path::new("/run/secrets/openhuman_master_key"));
            Ok(format!("{hex}\n"))
        })
        .expect("file key is valid")
        .expect("supplied");
    assert_eq!(key, [0x11u8; KEY_LEN]);
    assert!(source.starts_with(MASTER_KEY_FILE_ENV), "{source}");
    assert!(
        source.contains("/run/secrets/openhuman_master_key"),
        "{source}"
    );
}

#[test]
fn unreadable_env_file_is_an_error_that_names_the_path() {
    let err = master_key_from_env(None, Some("/nonexistent/master.key"), |_| {
        Err("cannot read master key file: boom".to_string())
    })
    .expect_err("unreadable file must not fall through to the keychain");
    assert!(err.contains(MASTER_KEY_FILE_ENV), "{err}");
    assert!(err.contains("/nonexistent/master.key"), "{err}");
    assert!(err.contains("cannot read"), "{err}");
}

#[test]
fn both_env_sources_set_is_rejected_before_reading_anything() {
    let hex = hex_key(0x22);
    let err = master_key_from_env(Some(&hex), Some("/run/secrets/key"), file_must_not_be_read)
        .expect_err("ambiguous configuration must be rejected");
    assert!(
        err.contains(MASTER_KEY_ENV) && err.contains(MASTER_KEY_FILE_ENV),
        "{err}"
    );
}

#[test]
fn malformed_env_key_is_rejected_without_leaking_the_value() {
    let too_short = "abcd";
    let err = master_key_from_env(Some(too_short), None, file_must_not_be_read)
        .expect_err("wrong length is rejected");
    assert!(err.contains("expected 64 hex characters, got 4"), "{err}");

    let not_hex = "zz".repeat(KEY_LEN);
    let err = master_key_from_env(Some(&not_hex), None, file_must_not_be_read)
        .expect_err("non-hex is rejected");
    assert!(err.contains("not valid hex"), "{err}");
    assert!(
        !err.contains(&not_hex),
        "error must not echo the value: {err}"
    );

    // Right character count, wrong bytes: must be rejected, not panic in the
    // byte-sliced hex decoder.
    let non_ascii = "é".repeat(KEY_LEN * 2);
    let err = master_key_from_env(Some(&non_ascii), None, file_must_not_be_read)
        .expect_err("non-ASCII is rejected");
    assert!(err.contains("not valid hex"), "{err}");

    // The same validation applies to a file's contents.
    let err = master_key_from_env(None, Some("/run/secrets/key"), |_| Ok("0123".to_string()))
        .expect_err("short file contents are rejected");
    assert!(
        err.contains(MASTER_KEY_FILE_ENV) && err.contains("got 4"),
        "{err}"
    );
}

#[test]
fn empty_env_values_fall_through_to_the_keychain() {
    assert!(master_key_from_env(None, None, file_must_not_be_read)
        .expect("nothing set is fine")
        .is_none());
    assert!(
        master_key_from_env(Some(""), Some("   "), file_must_not_be_read)
            .expect("empty values count as unset")
            .is_none()
    );
}

#[test]
fn read_master_key_file_returns_contents_and_reports_a_missing_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("master.key");
    std::fs::write(&path, format!("{}\n", hex_key(0x33))).unwrap();

    let contents = read_master_key_file(&path).expect("readable file");
    assert_eq!(contents.trim(), hex_key(0x33));

    let err = read_master_key_file(&tmp.path().join("missing.key")).expect_err("missing file");
    assert!(err.contains("cannot read master key file"), "{err}");
}

#[cfg(unix)]
#[test]
fn read_master_key_file_accepts_a_group_readable_secret_mount() {
    // Docker secrets are mounted 0444 and Kubernetes secret volumes 0644:
    // permissive modes are warned about, never refused.
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("master.key");
    std::fs::write(&path, hex_key(0x44)).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let contents = read_master_key_file(&path).expect("permissive mode is accepted");
    assert_eq!(contents, hex_key(0x44));
}

#[test]
fn parse_master_key_hex_round_trips_an_encoded_key() {
    let key_bytes = crypto::generate_random_bytes(KEY_LEN);
    let parsed = parse_master_key_hex(&crypto::hex_encode(&key_bytes)).expect("valid");
    assert_eq!(parsed.as_slice(), key_bytes.as_slice());
}

#[test]
fn env_value_distinguishes_unset_from_invalid_unicode() {
    use std::env::VarError;

    assert_eq!(
        env_value(MASTER_KEY_ENV, Ok("abc".to_string())).expect("set"),
        Some("abc".to_string())
    );
    assert_eq!(
        env_value(MASTER_KEY_ENV, Err(VarError::NotPresent)).expect("unset is not an error"),
        None
    );

    // Invalid Unicode must be rejected, not treated as unset: falling through
    // to the keychain could mint a different key and orphan `secrets.enc`.
    let rejected = std::ffi::OsString::from("not-the-real-bytes");
    let err = env_value(MASTER_KEY_FILE_ENV, Err(VarError::NotUnicode(rejected)))
        .expect_err("invalid Unicode is a configuration error");
    assert!(err.contains(MASTER_KEY_FILE_ENV), "{err}");
    assert!(err.contains("not valid Unicode"), "{err}");
    assert!(
        !err.contains("not-the-real-bytes"),
        "error must not echo the value: {err}"
    );
}

// ── Wiring: the real env → `try_load_master_key` → key, no keychain ─────────

#[test]
fn try_load_master_key_prefers_the_inline_env_key_and_never_touches_the_keychain() {
    let hex = hex_key(0x55);
    let _env = crate::config::test_env::EnvVarGuard::locked()
        .with(MASTER_KEY_ENV, &hex)
        .without(MASTER_KEY_FILE_ENV);

    // A real OS keychain cannot be exercised under `cargo test` (the first
    // access blocks on a GUI prompt), so reaching the keychain here would hang
    // or fail; returning means the env source short-circuited it.
    let (key, source) = try_load_master_key().expect("inline env key loads");
    assert_eq!(key, [0x55u8; KEY_LEN]);
    assert_eq!(source, MASTER_KEY_ENV);
}

#[test]
fn try_load_master_key_reads_the_file_source_and_is_stable_across_restarts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("master.key");
    std::fs::write(&path, format!("{}\n", hex_key(0x66))).unwrap();
    let _env = crate::config::test_env::EnvVarGuard::locked()
        .without(MASTER_KEY_ENV)
        .with(MASTER_KEY_FILE_ENV, &path);

    // Two loads stand in for two process starts: the same file must yield
    // the same key, so a secret encrypted before a restart decrypts after it.
    let (first, source) = try_load_master_key().expect("file key loads");
    let (second, _) = try_load_master_key().expect("file key loads again");
    assert_eq!(first, [0x66u8; KEY_LEN]);
    assert_eq!(first, second);
    assert!(source.contains(&path.display().to_string()), "{source}");

    let blob = crypto::chacha20_encrypt(&first, b"sk-live-secret").expect("encrypt");
    assert_eq!(
        crypto::chacha20_decrypt(&second, &blob).expect("decrypt with the restarted key"),
        b"sk-live-secret"
    );
}

#[test]
fn try_load_master_key_reports_a_configured_source_error_as_configured() {
    let _env = crate::config::test_env::EnvVarGuard::locked()
        .with(MASTER_KEY_ENV, "not-a-key")
        .without(MASTER_KEY_FILE_ENV);

    match try_load_master_key() {
        Err(MasterKeyError::Configured(e)) => {
            assert!(e.contains(MASTER_KEY_ENV), "{e}");
            assert!(!e.contains("not-a-key"), "must not echo the value: {e}");
        }
        other => panic!("expected a configuration error, got {other:?}"),
    }
}
