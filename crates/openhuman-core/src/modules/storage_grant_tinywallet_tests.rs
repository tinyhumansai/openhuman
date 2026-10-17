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
fn the_grant_is_declared_for_tinywallet_and_validates() {
    let declared = declared(MODULE_ID).expect("tinywallet declares storage");
    assert_eq!(declared, grant());
    declared.validate().expect("a valid grant");
    assert!(declared.writable);
    assert!(declared.secret_prefixes.is_empty());
}

#[cfg(feature = "web3")]
#[test]
fn the_declared_module_id_is_a_registry_module() {
    assert!(crate::modules::registry::ALL
        .iter()
        .any(|record| record.id == MODULE_ID));
}

#[test]
fn the_payments_stream_and_the_budget_hold_document_are_reachable() {
    assert!(allowed(
        Object::Streams,
        "Append",
        json!({"stream": "x402.payments"})
    ));
    assert!(allowed(
        Object::Streams,
        "ReadWindow",
        json!({"stream": "x402.payments"})
    ));
    // The hold is compare-and-swap on one document: puts with a precondition
    // are forwarded like any other write.
    assert!(allowed(
        Object::Documents,
        "Put",
        json!({"collection": "x402_budget", "id": "holds"})
    ));
    assert!(allowed(
        Object::Documents,
        "Get",
        json!({"collection": "x402_budget", "id": "holds"})
    ));
}

#[test]
fn nothing_else_is_reachable() {
    for other in [
        "x402_budget_extra",
        "tinywallet_host_import",
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
    assert_eq!(
        authorize(
            &grant(),
            Object::Streams,
            "Append",
            &json!({"stream": "mcp_audit"})
        ),
        Err(Denied::Target)
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
