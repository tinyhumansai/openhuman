//! Round-trip + tamper coverage for the Argon2id + AES-256-GCM primitives
//! (plan.md §4 P0 #2 — previously zero unit tests). `key_bytes` is private,
//! so key equality is asserted *behaviourally*: a key derived twice from the
//! same (password, salt) must decrypt the other's ciphertext, and any change
//! to password, salt, ciphertext, or nonce must make decryption fail.
use super::*;

fn key(password: &str, salt: &[u8]) -> EncryptionKey {
    EncryptionKey::derive(password, salt).expect("derive")
}

#[test]
fn encrypt_decrypt_bytes_round_trip() {
    let k = key(
        "correct horse battery staple",
        &EncryptionKey::generate_salt(),
    );
    let plaintext = b"the launch codes are 0000".to_vec();
    let payload = k.encrypt(&plaintext).expect("encrypt");
    assert_ne!(
        payload.ciphertext, plaintext,
        "ciphertext must not be plaintext"
    );
    assert_eq!(k.decrypt(&payload).expect("decrypt"), plaintext);
}

#[test]
fn encrypt_decrypt_string_round_trip() {
    let k = key("pw", &EncryptionKey::generate_salt());
    let secret = "sk-live-🔐-multibyte";
    let json = k.encrypt_string(secret).expect("encrypt_string");
    assert_eq!(k.decrypt_string(&json).expect("decrypt_string"), secret);
}

#[test]
fn kdf_is_deterministic_for_same_password_and_salt() {
    // Two independent derivations from the same (password, salt) must yield
    // the same key: key_a encrypts, key_b decrypts.
    let salt = EncryptionKey::generate_salt();
    let key_a = key("hunter2", &salt);
    let key_b = key("hunter2", &salt);
    let payload = key_a.encrypt(b"cross-key").expect("encrypt");
    assert_eq!(key_b.decrypt(&payload).expect("decrypt"), b"cross-key");
}

#[test]
fn wrong_password_cannot_decrypt() {
    let salt = EncryptionKey::generate_salt();
    let good = key("right-password", &salt);
    let bad = key("wrong-password", &salt);
    let payload = good.encrypt(b"top secret").expect("encrypt");
    assert!(
        bad.decrypt(&payload).is_err(),
        "a key from a different password must not decrypt"
    );
}

#[test]
fn different_salt_derives_a_different_key() {
    let a = key("same-password", &EncryptionKey::generate_salt());
    let b = key("same-password", &EncryptionKey::generate_salt());
    let payload = a.encrypt(b"salted").expect("encrypt");
    assert!(
        b.decrypt(&payload).is_err(),
        "same password + different salt must yield a non-interchangeable key"
    );
}

#[test]
fn tampered_ciphertext_is_rejected_by_gcm_auth() {
    let k = key("pw", &EncryptionKey::generate_salt());
    let mut payload = k.encrypt(b"authentic bytes").expect("encrypt");
    payload.ciphertext[0] ^= 0xFF; // flip a bit in the ciphertext/tag
    assert!(
        k.decrypt(&payload).is_err(),
        "AES-GCM must reject a tampered ciphertext (auth failure)"
    );
}

#[test]
fn tampered_nonce_is_rejected() {
    let k = key("pw", &EncryptionKey::generate_salt());
    let mut payload = k.encrypt(b"authentic bytes").expect("encrypt");
    payload.nonce[0] ^= 0xFF; // wrong nonce → auth tag no longer verifies
    assert!(
        k.decrypt(&payload).is_err(),
        "decrypting under a mutated nonce must fail"
    );
}

#[test]
fn each_encryption_uses_a_fresh_random_nonce() {
    // Nonce reuse under a fixed key is catastrophic for GCM. Encrypting the
    // same plaintext twice must produce distinct nonces (and, therefore,
    // distinct ciphertexts) — the nonce is drawn from the CSPRNG per call.
    let k = key("pw", &EncryptionKey::generate_salt());
    let p1 = k.encrypt(b"identical plaintext").expect("encrypt");
    let p2 = k.encrypt(b"identical plaintext").expect("encrypt");
    assert_ne!(
        p1.nonce, p2.nonce,
        "each encryption must draw a fresh nonce"
    );
    assert_ne!(
        p1.ciphertext, p2.ciphertext,
        "a fresh nonce must produce different ciphertext for the same plaintext"
    );
}

#[test]
fn generate_salt_is_correct_length_and_random() {
    let s1 = EncryptionKey::generate_salt();
    let s2 = EncryptionKey::generate_salt();
    assert_eq!(s1.len(), SALT_LENGTH, "salt must be {SALT_LENGTH} bytes");
    assert_ne!(s1, s2, "two generated salts must differ (CSPRNG)");
}

#[test]
fn decrypt_string_rejects_malformed_json() {
    let k = key("pw", &EncryptionKey::generate_salt());
    assert!(
        k.decrypt_string("not-json").is_err(),
        "non-JSON payload must be a clean Err, not a panic"
    );
}

#[test]
fn empty_plaintext_round_trips() {
    let k = key("pw", &EncryptionKey::generate_salt());
    let payload = k.encrypt(b"").expect("encrypt empty");
    assert_eq!(k.decrypt(&payload).expect("decrypt empty"), b"");
}

// NOTE: an `encrypt_decrypt_round_trips_for_arbitrary_input` proptest was
// trialled here but removed: under coverage instrumentation Argon2id runs
// ~2.5s/case, so a 24-case property held the lib-test binary for ~60s and
// deterministically widened a pre-existing env-var race in the unrelated
// `config::schema::load` env-overlay tests (they mutate process-global env
// without per-test serialization). The round-trip is already covered by the
// fixed-input tests above (round-trip, tamper, KDF determinism); the
// property-based *fuzzing* value lives in the fast, panic-focused
// `security::policy::proptest_tests` instead.
