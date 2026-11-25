use super::*;

fn store(tmp: &tempfile::TempDir) -> HttpCredentialsStore {
    HttpCredentialsStore::new(tmp.path(), false)
}

#[tokio::test]
async fn credential_resolution_ignores_an_absent_parsed_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = store(&tmp);
    assert!(resolve_http_credential(&store, None)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn credential_resolution_fails_closed_for_empty_or_unknown_names() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = store(&tmp);
    assert!(resolve_http_credential(&store, Some(" ")).await.is_err());
    assert!(resolve_http_credential(&store, Some("missing"))
        .await
        .is_err());
}

#[test]
fn a_stored_credential_is_refused_over_plain_http_to_a_remote_host() {
    let refused = |url: &str| refuse_credential_over_plain_http(&json!({ "url": url })).is_err();
    // Remote plain http, including userinfo smuggling a loopback-looking prefix.
    assert!(refused("http://api.example.com/v1"));
    assert!(refused("http://127.0.0.1:8080@evil.example/v1"));
    assert!(refused("not a url"));
    assert!(refuse_credential_over_plain_http(&json!({})).is_err());
    // https anywhere, and plain http to loopback, still go through.
    assert!(!refused("https://api.example.com/v1"));
    assert!(!refused("http://127.0.0.1:8080/v1"));
    assert!(!refused("http://localhost/v1"));
    assert!(!refused("http://[::1]:9000/v1"));
}
