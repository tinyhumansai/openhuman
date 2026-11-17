use super::*;
use openhuman_rpc::tinyhumans::link::CoreAuthState;
#[test]
fn cached_profile_is_visible_without_leaking_credentials_or_resetting_same_account() {
    let mut ui = UiState::new("t".into(), "c".into());
    let state = SessionState {
        core: CoreAuthState {
            is_authenticated: true,
            user_id: Some("user".into()),
            credential: Some("session".into()),
            ..Default::default()
        },
        current_user: Some(
            serde_json::json!({"firstName":"Ada","email":"ada@example.test","token":"must-not-display"}),
        ),
        current_user_stale: true,
        ..Default::default()
    };
    apply_session(&state, &mut ui);
    assert!(ui.identity_changed);
    assert!(ui.auth_summary.contains("cached"));
    assert!(!ui.account_detail.contains("must-not-display"));
    ui.identity_changed = false;
    apply_session(&state, &mut ui);
    assert!(!ui.identity_changed);
}
#[test]
fn stale_login_completion_cannot_replace_a_newer_account_operation() {
    let mut ui = UiState::new("t".into(), "c".into());
    ui.auth_generation = 2;
    apply(
        Message::Finished {
            generation: 1,
            operation: Operation::Login,
            result: Ok(SessionState::default()),
        },
        &mut ui,
    );
    assert!(!ui.identity_changed);
}
#[test]
fn signout_clears_visible_account_and_requests_context_reset() {
    let mut ui = UiState::new("t".into(), "c".into());
    ui.authenticated = true;
    ui.auth_user_id = Some("old".into());
    ui.account_detail = "old profile".into();
    apply_session(&SessionState::default(), &mut ui);
    assert!(ui.identity_changed);
    assert!(ui.account_detail.is_empty());
    assert_eq!(ui.auth_summary, "Signed out");
}

#[test]
fn signed_out_state_does_not_display_a_leftover_cached_profile() {
    let mut ui = UiState::new("t".into(), "c".into());
    let state = SessionState {
        current_user: Some(serde_json::json!({"email":"previous@example.test"})),
        ..Default::default()
    };
    apply_session(&state, &mut ui);
    assert!(ui.account_detail.is_empty());
    assert_eq!(ui.auth_summary, "Signed out");
}
#[test]
fn cancellation_does_not_detach_an_inflight_token_session_update() {
    let mut ui = UiState::new("t".into(), "c".into());
    ui.auth_pending = true;
    ui.auth_operation = Some(Operation::Login);
    ui.auth_generation = 4;
    cancel(&mut ui);
    assert!(ui.auth_pending);
    assert_eq!(ui.auth_generation, 4);
    ui.auth_browser = true;
    cancel(&mut ui);
    assert!(!ui.auth_pending);
    assert_eq!(ui.auth_generation, 5);
}
#[test]
fn pending_browser_forms_cannot_block_mouse_login_controls() {
    let mut ui = UiState::new("t".into(), "c".into());
    ui.auth_pending = true;
    ui.auth_browser = true;
    for logout in [false, true] {
        form(&mut ui, logout);
        assert_eq!(ui.active_tab, AppTab::Settings);
        assert!(ui.login_token.is_none());
        assert!(!ui.logout_confirm);
    }
    ui.auth_pending = false;
    form(&mut ui, false);
    assert_eq!(ui.login_token.as_deref(), Some(""));
    form(&mut ui, true);
    assert!(ui.logout_confirm);
    assert!(ui.login_token.is_none());
}
#[test]
fn provisional_offline_session_is_not_presented_as_a_verified_account() {
    let mut ui = UiState::new("t".into(), "c".into());
    let state = SessionState {
        core: CoreAuthState {
            is_authenticated: true,
            credential: Some("session".into()),
            user: Some(serde_json::json!({"pendingBackendValidation":true})),
            ..Default::default()
        },
        ..Default::default()
    };
    apply_session(&state, &mut ui);
    assert_eq!(ui.auth_summary, "Session saved · validation pending");
    assert!(ui.account_detail.is_empty());
}
