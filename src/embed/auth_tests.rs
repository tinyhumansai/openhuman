use super::*;

#[test]
fn a_local_session_is_shaped_the_way_the_core_detects() {
    // The core recognizes a local session purely by the third dot-segment being
    // `local`. Building the token here rather than taking one means a caller
    // cannot get that wrong and be told only "validation failed".
    let session = Session::local("local-test");
    assert!(session.is_local());
    assert_eq!(session.token.split('.').count(), 3);
    assert!(session.token.ends_with(".local"));
}

#[test]
fn a_local_session_carries_a_user_payload() {
    // `store_session` rejects a local session without one.
    let session = Session::local("local-test");
    let user = session.user.expect("local sessions carry a user");
    assert_eq!(user.get("id").and_then(|v| v.as_str()), Some("local-test"));
}

#[test]
fn a_backend_session_is_not_local() {
    assert!(!Session::backend("header.payload.signature").is_local());
}

#[test]
fn user_overrides_the_generated_payload() {
    let session = Session::local("a").user(serde_json::json!({ "id": "b" }));
    assert_eq!(
        session.user.and_then(|u| u.get("id").cloned()),
        Some(serde_json::json!("b"))
    );
}

#[test]
fn auth_state_decodes_the_core_wire_shape() {
    let wire = serde_json::json!({ "isAuthenticated": true, "userId": "u-1" });
    let state: AuthState = serde_json::from_value(wire).expect("decodes");
    assert!(state.is_authenticated);
    assert_eq!(state.user_id.as_deref(), Some("u-1"));
}

#[test]
fn auth_state_tolerates_a_signed_out_shape() {
    // A signed-out response omits the user id entirely rather than nulling it.
    let state: AuthState =
        serde_json::from_value(serde_json::json!({ "isAuthenticated": false })).expect("decodes");
    assert!(!state.is_authenticated);
    assert!(state.user_id.is_none());
}

#[test]
fn a_token_payload_decodes_both_present_and_absent() {
    #[derive(serde::Deserialize)]
    struct TokenPayload {
        #[serde(default)]
        token: Option<String>,
    }

    // `auth_get_session_token_json` emits `{"token": <value|null>}`.
    let present: TokenPayload =
        serde_json::from_value(serde_json::json!({ "token": "jwt" })).expect("decodes");
    assert_eq!(present.token.as_deref(), Some("jwt"));

    let absent: TokenPayload =
        serde_json::from_value(serde_json::json!({ "token": null })).expect("decodes");
    assert!(absent.token.is_none());

    // Defensive: the key missing entirely is still the signed-out state, not a
    // decode failure.
    let missing: TokenPayload = serde_json::from_value(serde_json::json!({})).expect("decodes");
    assert!(missing.token.is_none());
}
