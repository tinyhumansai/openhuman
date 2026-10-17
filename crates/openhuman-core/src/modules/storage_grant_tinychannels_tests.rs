use super::*;
use crate::modules::storage_grants::{authorize, declared, Denied, Object, Plan};
use serde_json::json;

fn allowed(object: Object, selector: &str, request: serde_json::Value) -> bool {
    matches!(
        authorize(&grant(), object, selector, &request),
        Ok(Plan::Forward { .. })
    )
}

#[test]
fn the_grant_is_declared_for_tinychannels_and_validates() {
    let declared = declared(MODULE_ID).expect("tinychannels declares storage");
    assert_eq!(declared, grant());
    declared.validate().expect("a valid grant");
    assert!(declared.writable);
    assert!(declared.stream_prefixes.is_empty());
    assert!(declared.blob_prefixes.is_empty());
    assert!(declared.secret_prefixes.is_empty());
}

#[test]
fn the_declared_module_id_is_a_registry_module() {
    assert!(crate::modules::registry::ALL
        .iter()
        .any(|record| record.id == MODULE_ID));
}

#[test]
fn delivery_remote_and_whatsapp_collections_are_all_granted() {
    let names: Vec<String> = grant().collections.iter().map(|s| s.name.clone()).collect();
    assert_eq!(names.len(), 14);
    for name in &names {
        assert!(
            allowed(
                Object::Documents,
                "Put",
                json!({"collection": name, "id": "x"})
            ),
            "{name}"
        );
    }
    for expected in [
        "channels_delivery_pending",
        "channels_delivery_failed",
        "channels_delivery_results",
        "channel_remote_bindings",
        "whatsapp_session",
        "whatsapp_prekey",
    ] {
        assert!(names.iter().any(|n| n == expected), "{expected}");
    }
}

#[test]
fn other_collections_and_the_host_import_marker_are_refused() {
    for other in [
        "approvals",
        "mcp_servers",
        "channels_host_import",
        "_secrets",
    ] {
        assert_eq!(
            authorize(
                &grant(),
                Object::Documents,
                "Get",
                &json!({"collection": other, "id": "x"})
            ),
            Err(Denied::Target),
            "{other}"
        );
    }
}

#[test]
fn streams_blobs_and_secrets_are_not_granted() {
    assert_eq!(
        authorize(&grant(), Object::Streams, "Append", &json!({"stream": "s"})),
        Err(Denied::Object)
    );
    assert_eq!(
        authorize(&grant(), Object::Blobs, "Get", &json!({"key": "k"})),
        Err(Denied::Object)
    );
    assert_eq!(
        authorize(&grant(), Object::Secrets, "Get", &json!({"name": "n"})),
        Err(Denied::Object)
    );
}
