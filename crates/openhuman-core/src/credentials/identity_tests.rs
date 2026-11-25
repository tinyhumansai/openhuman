use super::*;
use serde_json::json;

#[test]
fn identity_reduces_to_id_name_email() {
    let user = json!({ "_id": "u1", "displayName": "Ada", "email": "a@x.io", "token": "nope" });
    let identity = identity_from_user(&user).unwrap();
    assert_eq!(identity.id.as_deref(), Some("u1"));
    assert_eq!(identity.name.as_deref(), Some("Ada"));
    assert_eq!(identity.email.as_deref(), Some("a@x.io"));
}

#[test]
fn placeholder_payloads_yield_no_identity() {
    assert!(identity_from_user(&json!({ "pendingBackendValidation": true })).is_none());
    assert!(identity_from_user(&json!({ "id": "  " })).is_none());
    assert!(identity_from_user(&json!("str")).is_none());
}

#[test]
fn slot_round_trip_ignores_empty_objects() {
    set_current_user(Some(json!({ "id": "slot-user" })));
    assert_eq!(
        peek_credential_user_identity()
            .and_then(|i| i.id)
            .as_deref(),
        Some("slot-user")
    );
    set_current_user(Some(json!({})));
    assert!(current_user().is_none());
    set_current_user(Some(json!({ "id": "again" })));
    clear_current_user();
    assert!(peek_credential_user_identity().is_none());
}
