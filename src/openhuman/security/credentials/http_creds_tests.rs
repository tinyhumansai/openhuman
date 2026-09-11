use super::*;

fn temp_store() -> (tempfile::TempDir, HttpCredentialsStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    // encrypt=true exercises the ChaCha20-Poly1305 at-rest path.
    let store = HttpCredentialsStore::new(dir.path(), true);
    (dir, store)
}

#[test]
fn bearer_to_header_is_authorization_bearer() {
    let cred = HttpCredential::bearer("stripe", "sk_live_abc123");
    let (name, value) = cred.to_header().unwrap();
    assert_eq!(name, "Authorization");
    assert_eq!(value, "Bearer sk_live_abc123");
}

#[test]
fn basic_to_header_is_base64_user_pass() {
    let cred = HttpCredential::basic("acme", "alice", "hunter2");
    let (name, value) = cred.to_header().unwrap();
    assert_eq!(name, "Authorization");
    // base64("alice:hunter2")
    let expected = base64::engine::general_purpose::STANDARD.encode("alice:hunter2");
    assert_eq!(value, format!("Basic {expected}"));
}

#[test]
fn header_scheme_uses_custom_header_name() {
    let cred = HttpCredential::header("apikey", "X-API-Key", "topsecret");
    let (name, value) = cred.to_header().unwrap();
    assert_eq!(name, "X-API-Key");
    assert_eq!(value, "topsecret");
}

#[test]
fn header_scheme_without_header_name_errors() {
    let mut cred = HttpCredential::header("apikey", "X-API-Key", "topsecret");
    cred.header_name = None;
    assert!(cred.to_header().is_err());
}

#[test]
fn roundtrip_encrypts_secret_at_rest() {
    let (dir, store) = temp_store();
    let secret = "sk_live_super_secret_value";
    store
        .upsert(&HttpCredential::bearer("stripe", secret))
        .unwrap();

    // The on-disk file must NOT contain the plaintext secret.
    let raw = std::fs::read_to_string(dir.path().join(STORE_FILENAME)).unwrap();
    assert!(
        !raw.contains(secret),
        "plaintext secret leaked into on-disk store: {raw}"
    );
    assert!(raw.contains("enc2:"), "secret was not encrypted: {raw}");

    // But get() decrypts it back.
    let got = store.get("stripe").unwrap().expect("credential present");
    assert_eq!(got.secret, secret);
    assert_eq!(got.scheme, HttpCredentialScheme::Bearer);
}

#[test]
fn name_resolution_is_case_insensitive_and_trimmed() {
    let (_dir, store) = temp_store();
    store
        .upsert(&HttpCredential::bearer("Stripe", "tok"))
        .unwrap();
    assert!(store.get("  STRIPE ").unwrap().is_some());
    assert!(store.get("stripe").unwrap().is_some());
}

#[test]
fn list_never_exposes_secrets() {
    let (_dir, store) = temp_store();
    store
        .upsert(&HttpCredential::header("apikey", "X-API-Key", "topsecret"))
        .unwrap();
    let summaries = store.list().unwrap();
    assert_eq!(summaries.len(), 1);
    let s = &summaries[0];
    assert_eq!(s.name, "apikey");
    assert_eq!(s.scheme, "header");
    assert_eq!(s.header_name.as_deref(), Some("X-API-Key"));
    // The summary type has no secret field at all — assert via serialization
    // that "topsecret" never appears.
    let json = serde_json::to_string(&summaries).unwrap();
    assert!(
        !json.contains("topsecret"),
        "secret leaked into summary: {json}"
    );
}

#[test]
fn get_unknown_name_returns_none() {
    let (_dir, store) = temp_store();
    assert!(store.get("does-not-exist").unwrap().is_none());
}

#[test]
fn remove_deletes_record() {
    let (_dir, store) = temp_store();
    store
        .upsert(&HttpCredential::bearer("stripe", "tok"))
        .unwrap();
    assert!(store.remove("stripe").unwrap());
    assert!(store.get("stripe").unwrap().is_none());
    assert!(!store.remove("stripe").unwrap());
}
