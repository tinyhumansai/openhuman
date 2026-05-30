use super::*;
use crate::openhuman::memory_store::MemoryClient;
use std::sync::Arc;
use tempfile::TempDir;

fn make_client() -> (TempDir, Arc<MemoryClient>) {
    let tmp = TempDir::new().unwrap();
    let client = Arc::new(
        MemoryClient::from_workspace_dir(tmp.path().join("workspace"))
            .expect("memory client should initialize for user-scope tests"),
    );
    (tmp, client)
}

#[test]
fn default_is_read_write_no_admin() {
    let p = UserScopePref::default();
    assert!(p.read);
    assert!(p.write);
    assert!(!p.admin);
}

#[test]
fn allows_matches_scope() {
    let p = UserScopePref {
        read: true,
        write: false,
        admin: false,
    };
    assert!(p.allows(ToolScope::Read));
    assert!(!p.allows(ToolScope::Write));
    assert!(!p.allows(ToolScope::Admin));
}

#[test]
fn round_trip_serde() {
    let p = UserScopePref {
        read: true,
        write: true,
        admin: true,
    };
    let v = serde_json::to_value(p).unwrap();
    let back: UserScopePref = serde_json::from_value(v).unwrap();
    assert_eq!(p, back);
}

#[test]
fn missing_fields_default_to_true_for_read_write() {
    // Forward-compat: if we ever drop a field, existing stored
    // documents still deserialize sensibly.
    let v = serde_json::json!({});
    let p: UserScopePref = serde_json::from_value(v).unwrap();
    assert_eq!(p, UserScopePref::default());
}

#[tokio::test]
async fn save_and_load_round_trip_uses_normalized_toolkit_key() {
    let (_tmp, client) = make_client();
    let pref = UserScopePref {
        read: true,
        write: false,
        admin: true,
    };

    save(&client, "  GMail  ", pref).await.unwrap();

    let loaded = load(&client, "gmail").await;
    assert_eq!(loaded, pref);

    let raw = client
        .kv_get(Some(KV_NAMESPACE), "gmail")
        .await
        .unwrap()
        .expect("normalized toolkit key should be used");
    assert_eq!(raw.get("write").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(raw.get("admin").and_then(|v| v.as_bool()), Some(true));
}

#[tokio::test]
async fn load_falls_back_to_default_when_stored_payload_is_invalid() {
    let (_tmp, client) = make_client();
    client
        .kv_set(
            Some(KV_NAMESPACE),
            "gmail",
            &serde_json::json!("not-an-object"),
        )
        .await
        .unwrap();

    let loaded = load(&client, "gmail").await;
    assert_eq!(loaded, UserScopePref::default());
}

#[tokio::test]
async fn save_rejects_blank_toolkit() {
    let (_tmp, client) = make_client();
    let err = save(&client, "   ", UserScopePref::default())
        .await
        .unwrap_err();
    assert!(err.contains("toolkit must not be empty"));
}
