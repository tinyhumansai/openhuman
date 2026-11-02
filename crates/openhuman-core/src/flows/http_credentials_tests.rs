use super::*;

fn temp_store() -> (tempfile::TempDir, HttpCredentialsStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    // Focused tests exercise the HTTP credential adapter; TinySecurity crypto
    // is covered at the module boundary.
    let store = HttpCredentialsStore::new(dir.path(), false);
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

#[tokio::test]
async fn encrypted_upsert_fails_closed_without_module_config() {
    let (dir, store) = temp_store();
    let secret = "sk_live_super_secret_value";
    let result = store
        .upsert(&HttpCredential::bearer("stripe", secret))
        .await;
    assert!(
        result.is_err(),
        "encrypted storage must not use host crypto"
    );
    assert!(!dir.path().join(STORE_FILENAME).exists());
}

#[tokio::test]
async fn name_resolution_is_case_insensitive_and_trimmed() {
    let (_dir, store) = temp_store();
    store
        .upsert(&HttpCredential::bearer("Stripe", "tok"))
        .await
        .unwrap();
    assert!(store.get("  STRIPE ").await.unwrap().is_some());
    assert!(store.get("stripe").await.unwrap().is_some());
}

#[tokio::test]
async fn list_never_exposes_secrets() {
    let (_dir, store) = temp_store();
    store
        .upsert(&HttpCredential::header("apikey", "X-API-Key", "topsecret"))
        .await
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

#[tokio::test]
async fn get_unknown_name_returns_none() {
    let (_dir, store) = temp_store();
    assert!(store.get("does-not-exist").await.unwrap().is_none());
}

#[tokio::test]
async fn remove_deletes_record() {
    let (_dir, store) = temp_store();
    store
        .upsert(&HttpCredential::bearer("stripe", "tok"))
        .await
        .unwrap();
    assert!(store.remove("stripe").unwrap());
    assert!(store.get("stripe").await.unwrap().is_none());
    assert!(!store.remove("stripe").unwrap());
}
