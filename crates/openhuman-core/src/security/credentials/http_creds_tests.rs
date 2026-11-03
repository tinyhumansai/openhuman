use super::*;

fn temp_store() -> (tempfile::TempDir, HttpCredentialsStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = crate::config::Config {
        config_path: dir.path().join("config.toml"),
        workspace_dir: dir.path().join("workspace"),
        action_dir: dir.path().join("workspace"),
        ..crate::config::Config::default()
    };
    config.modules.enabled = false;
    let store = HttpCredentialsStore::from_config(&config);
    (dir, store)
}

fn plaintext_store() -> (tempfile::TempDir, HttpCredentialsStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = crate::config::Config {
        config_path: dir.path().join("config.toml"),
        workspace_dir: dir.path().join("workspace"),
        action_dir: dir.path().join("workspace"),
        ..crate::config::Config::default()
    };
    let mut config = config;
    config.secrets.encrypt = false;
    (dir, HttpCredentialsStore::from_config(&config))
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
async fn encrypted_http_credentials_fail_closed_without_tinysecurity() {
    let (dir, store) = temp_store();
    let secret = "sk_live_super_secret_value";
    let error = store
        .upsert(&HttpCredential::bearer("stripe", secret))
        .await
        .expect_err("encrypted writes must use the admitted TinySecurity cipher");
    assert!(error.to_string().contains("TinySecurity"));

    assert!(!dir.path().join(STORE_FILENAME).exists());
}

#[tokio::test]
async fn plaintext_http_credentials_round_trip_and_normalize_names() {
    let (_dir, store) = plaintext_store();
    store
        .upsert(&HttpCredential::bearer("Stripe", "tok"))
        .await
        .unwrap();
    assert!(store.get("  STRIPE ").await.unwrap().is_some());
    assert!(store.get("stripe").await.unwrap().is_some());
}

#[tokio::test]
async fn list_never_exposes_secrets() {
    let (_dir, store) = plaintext_store();
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
    let (_dir, store) = plaintext_store();
    assert!(store.get("does-not-exist").await.unwrap().is_none());
}

#[tokio::test]
async fn remove_deletes_record() {
    let (_dir, store) = plaintext_store();
    store
        .upsert(&HttpCredential::bearer("stripe", "tok"))
        .await
        .unwrap();
    assert!(store.remove("stripe").unwrap());
    assert!(store.get("stripe").await.unwrap().is_none());
    assert!(!store.remove("stripe").unwrap());
}
