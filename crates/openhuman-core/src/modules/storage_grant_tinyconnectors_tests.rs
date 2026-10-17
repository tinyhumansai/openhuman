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
fn the_grant_is_declared_for_tinyconnectors_and_validates() {
    let declared = declared(MODULE_ID).expect("tinyconnectors declares storage");
    assert_eq!(declared, grant());
    declared.validate().expect("a valid grant");
    assert!(declared.writable);
    assert!(declared.secret_prefixes.is_empty());
}

#[test]
fn the_declared_module_id_is_a_registry_module() {
    assert!(crate::modules::registry::ALL
        .iter()
        .any(|record| record.id == MODULE_ID));
}

#[test]
fn only_its_own_collection_and_trigger_streams_are_reachable() {
    assert_eq!(grant().collections.len(), 1);
    assert!(allowed(
        Object::Documents,
        "Put",
        json!({"collection": "composio-user-scopes", "id": "gmail"})
    ));
    for day in ["triggers/2026-10-13", "triggers/x"] {
        assert!(
            allowed(Object::Streams, "Append", json!({"stream": day})),
            "{day}"
        );
        assert!(
            allowed(Object::Streams, "Len", json!({"stream": day})),
            "{day}"
        );
    }
    for other in ["trigger", "mcp_audit", "x402.payments", "_secrets"] {
        assert_eq!(
            authorize(
                &grant(),
                Object::Streams,
                "Append",
                &json!({"stream": other})
            ),
            Err(Denied::Target),
            "{other}"
        );
    }
    for other in [
        "composio-user-scopes-x",
        "tinyconnectors_host_import",
        "approvals",
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
fn blobs_and_secrets_are_not_granted() {
    assert_eq!(
        authorize(&grant(), Object::Blobs, "Get", &json!({"key": "k"})),
        Err(Denied::Object)
    );
    assert_eq!(
        authorize(&grant(), Object::Secrets, "Get", &json!({"name": "n"})),
        Err(Denied::Object)
    );
}
