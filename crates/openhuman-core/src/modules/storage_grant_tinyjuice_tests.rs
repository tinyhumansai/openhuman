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
fn the_grant_is_declared_for_tinyjuice_and_validates() {
    let declared = declared(MODULE_ID).expect("tinyjuice declares storage");
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
fn only_ccr_blobs_are_reachable() {
    for selector in ["Get", "Head", "Delete"] {
        assert!(
            allowed(
                Object::Blobs,
                selector,
                json!({"key": "tinyjuice_ccr/0123456789abcdef0123456789abcdef"})
            ),
            "{selector}"
        );
    }
    for other in [
        "tinyjuice_ccr",
        "tinyjuice_ccr_x/a",
        "ccr/a",
        "../tinyjuice_ccr/a",
    ] {
        assert_eq!(
            authorize(&grant(), Object::Blobs, "Get", &json!({"key": other})),
            Err(Denied::Target),
            "{other}"
        );
    }
}

#[test]
fn documents_streams_and_secrets_are_not_granted() {
    assert!(grant().collections.is_empty());
    assert_eq!(
        authorize(
            &grant(),
            Object::Documents,
            "Get",
            &json!({"collection": "tinyjuice_host_import", "id": "x"})
        ),
        Err(Denied::Object)
    );
    assert_eq!(
        authorize(&grant(), Object::Streams, "Append", &json!({"stream": "s"})),
        Err(Denied::Object)
    );
    assert_eq!(
        authorize(&grant(), Object::Secrets, "Get", &json!({"name": "n"})),
        Err(Denied::Object)
    );
}
