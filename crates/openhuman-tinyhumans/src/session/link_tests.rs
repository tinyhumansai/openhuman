use super::*;
use crate::session::test_support::FakeCore;

#[test]
fn unwrap_envelope_walks_result_and_data() {
    let wrapped = json!({ "result": { "data": { "token": "t" } }, "logs": ["x"] });
    assert_eq!(unwrap_envelope(&wrapped), &json!({ "token": "t" }));
    let bare = json!({ "isAuthenticated": true });
    assert_eq!(unwrap_envelope(&bare), &bare);
}

#[test]
fn core_auth_state_parses_camel_case_and_kind() {
    let state: CoreAuthState = serde_json::from_value(json!({
        "isAuthenticated": true,
        "credential": "api-key",
        "userId": null,
        "user": null,
        "profileId": null,
        "expiresAt": "2100-01-01T00:00:00Z"
    }))
    .unwrap();
    assert!(state.is_authenticated);
    assert_eq!(state.kind(), Some(CredentialKind::ApiKey));
    assert_eq!(state.expires_at.as_deref(), Some("2100-01-01T00:00:00Z"));
    assert!(
        state.issuing_backend.is_none(),
        "legacy DTOs remain compatible"
    );

    let signed_out: CoreAuthState =
        serde_json::from_value(json!({ "isAuthenticated": false })).unwrap();
    assert_eq!(signed_out.kind(), None);
}

#[tokio::test]
async fn push_credential_sends_kind_user_id_and_user() {
    let core = FakeCore::new("http://backend");
    let credential = Credential::session("jwt-1");
    let state = push_credential(
        core.as_ref(),
        &credential,
        Some(" u1 "),
        Some(&json!({ "id": "u1" })),
    )
    .await
    .unwrap();
    assert!(state.is_authenticated);
    assert_eq!(state.user_id.as_deref(), Some("u1"));
    let (method, params) = core.calls().pop().unwrap();
    assert_eq!(method, AUTH_SET_CREDENTIAL);
    assert_eq!(params["token"], "jwt-1");
    assert_eq!(params["kind"], "session");
    assert_eq!(params["userId"], "u1");
    assert_eq!(params["user"]["id"], "u1");
}

#[tokio::test]
async fn push_credential_omits_blank_user_id_and_missing_user() {
    let core = FakeCore::new("http://backend");
    push_credential(core.as_ref(), &Credential::api_key("k"), Some("  "), None)
        .await
        .unwrap();
    let (_, params) = core.calls().pop().unwrap();
    assert_eq!(params["kind"], "api-key");
    assert!(params.get("userId").is_none());
    assert!(params.get("user").is_none());
}

#[tokio::test]
async fn clear_credential_passes_kind_or_nothing() {
    let core = FakeCore::new("http://backend");
    clear_credential(core.as_ref(), Some(CredentialKind::ApiKey))
        .await
        .unwrap();
    clear_credential(core.as_ref(), None).await.unwrap();
    let calls = core.calls();
    assert_eq!(calls[0].0, AUTH_CLEAR_CREDENTIAL);
    assert_eq!(calls[0].1, json!({ "kind": "api-key" }));
    assert_eq!(calls[1].1, json!({}));
}

#[tokio::test]
async fn session_token_and_backend_url_unwrap_envelopes() {
    let core = FakeCore::new("http://backend/");
    assert_eq!(core_session_token(core.as_ref()).await.unwrap(), None);
    push_credential(core.as_ref(), &Credential::session("jwt-2"), None, None)
        .await
        .unwrap();
    assert_eq!(
        core_session_token(core.as_ref()).await.unwrap().as_deref(),
        Some("jwt-2")
    );
    assert_eq!(
        resolve_backend_url(core.as_ref()).await.unwrap(),
        "http://backend/"
    );
}

#[tokio::test]
async fn link_errors_propagate() {
    let core = FakeCore::new("http://backend");
    *core.fail_with.lock().unwrap() = Some("core down".to_string());
    assert_eq!(
        core_auth_state(core.as_ref()).await.unwrap_err(),
        "core down".to_string()
    );
}
