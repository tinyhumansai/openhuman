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
fn the_grant_is_declared_for_tinybox_and_validates() {
    let declared = declared(MODULE_ID).expect("tinybox declares storage");
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
fn box_records_and_the_jail_index_are_reachable() {
    for name in ["tinybox_boxes", "tinybox_jail_index"] {
        assert!(
            allowed(
                Object::Documents,
                "Put",
                json!({"collection": name, "id": "x"})
            ),
            "{name}"
        );
    }
    assert_eq!(grant().collections.len(), 2);
    for other in [
        "tinybox_host_import",
        "tinybox_boxes_x",
        "approvals",
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
