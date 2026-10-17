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
fn the_grant_is_declared_for_tinymcp_and_validates() {
    let declared = declared(MODULE_ID).expect("tinymcp declares storage");
    assert_eq!(declared, grant());
    declared.validate().expect("a valid grant");
    assert!(declared.writable);
    assert!(declared.blob_prefixes.is_empty());
}

#[test]
fn the_declared_module_id_is_a_registry_module() {
    assert!(crate::modules::registry::ALL
        .iter()
        .any(|record| record.id == MODULE_ID));
}

#[test]
fn tinymcp_reaches_exactly_its_own_collections() {
    for name in grant().collections.iter().map(|spec| spec.name.clone()) {
        assert!(
            allowed(
                Object::Documents,
                "Get",
                json!({"collection": name, "id": "x"})
            ),
            "{name}"
        );
    }
    for other in ["approvals", "mcp_servers_extra", "_secrets", "mcp_audit"] {
        assert_eq!(
            authorize(
                &grant(),
                Object::Documents,
                "Put",
                &json!({"collection": other})
            ),
            Err(Denied::Target),
            "{other}"
        );
    }
}

#[test]
fn the_catalog_index_is_declared_for_the_modules_own_search_filters() {
    let grant = grant();
    let spec = |name: &str| {
        grant
            .collections
            .iter()
            .find(|spec| spec.name == name)
            .unwrap()
    };
    assert_eq!(spec("mcp_servers").indexes.len(), 2);
    assert_eq!(spec("mcp_registry_index").indexes.len(), 1);
    assert!(spec("mcp_server_names").indexes.is_empty());
}

#[test]
fn the_audit_stream_and_env_secrets_are_granted_and_nothing_else() {
    assert!(allowed(
        Object::Streams,
        "Append",
        json!({"stream": "mcp_audit", "payload": {}})
    ));
    assert_eq!(
        authorize(
            &grant(),
            Object::Streams,
            "Append",
            &json!({"stream": "other"})
        ),
        Err(Denied::Target)
    );
    for name in ["mcp_client_env/srv-1", "mcp_cache_fingerprint_key"] {
        assert!(
            allowed(Object::Secrets, "Get", json!({"name": name})),
            "{name}"
        );
    }
    assert_eq!(
        authorize(
            &grant(),
            Object::Secrets,
            "Get",
            &json!({"name": "wallet/seed"})
        ),
        Err(Denied::Target)
    );
    assert_eq!(
        authorize(&grant(), Object::Blobs, "Get", &json!({"key": "any"})),
        Err(Denied::Object)
    );
}
