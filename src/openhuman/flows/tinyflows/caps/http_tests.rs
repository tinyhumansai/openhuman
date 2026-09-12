use super::*;

fn store(tmp: &tempfile::TempDir) -> HttpCredentialsStore {
    HttpCredentialsStore::new(tmp.path(), false)
}

#[test]
fn credential_resolution_ignores_absent_and_foreign_refs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = store(&tmp);
    assert!(resolve_http_credential(&store, None).unwrap().is_none());
    assert!(resolve_http_credential(&store, Some("composio:x:y"))
        .unwrap()
        .is_none());
}

#[test]
fn credential_resolution_fails_closed_for_malformed_or_unknown_refs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = store(&tmp);
    assert!(resolve_http_credential(&store, Some("http_cred:")).is_err());
    assert!(resolve_http_credential(&store, Some("http_cred:missing")).is_err());
}
