use super::*;
use openhuman_rpc::embed::chat_surface::WebChannelEvent;

fn web_event(value: serde_json::Value) -> WebChannelEvent {
    serde_json::from_value(value).expect("test event matches TinyChannels' web contract")
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn plain_digits_remain_chat_input_and_alt_digits_switch_tabs() {
    for digit in ['1', '2', '3', '4'] {
        assert_eq!(
            tab_shortcut(key(KeyCode::Char(digit), KeyModifiers::NONE), AppTab::Chat),
            None
        );
    }
    assert_eq!(
        tab_shortcut(key(KeyCode::Char('3'), KeyModifiers::ALT), AppTab::Chat),
        Some(AppTab::Config)
    );
}

#[test]
fn paste_routes_only_to_the_active_editable_surface() {
    let mut ui = UiState::new("thread".into(), "client".into());
    ui.active_tab = AppTab::Chat;
    handle_paste("model-4", &mut ui);
    assert_eq!(ui.composer.text(), "model-4");

    ui.active_tab = AppTab::Settings;
    ui.login_token = Some(String::new());
    handle_paste("one-time-token", &mut ui);
    assert_eq!(ui.login_token.as_deref(), Some("one-time-token"));
}

#[test]
fn workspace_file_picker_is_bounded_filtered_and_skips_heavy_trees() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    std::fs::create_dir_all(root.path().join("target")).unwrap();
    std::fs::write(root.path().join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(root.path().join("target/hidden.rs"), "").unwrap();
    let files = collect_files(root.path().to_str().unwrap(), "main", 10).unwrap();
    assert_eq!(files, vec!["src/main.rs"]);
}

#[test]
fn workspace_file_picker_stops_at_the_depth_limit() {
    let root = tempfile::tempdir().unwrap();
    let mut deep = root.path().to_path_buf();
    for level in 0..10 {
        deep.push(format!("level-{level}"));
    }
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("too-deep.txt"), "hidden").unwrap();
    let files = collect_files(root.path().to_str().unwrap(), "too-deep", 10).unwrap();
    assert!(files.is_empty());
}

#[test]
fn decision_shortcuts_do_not_treat_filter_text_as_deny() {
    assert_eq!(
        decision_shortcut(OverlayKind::Approvals, false, KeyCode::Char('d')),
        None
    );
    assert_eq!(
        decision_shortcut(OverlayKind::Approvals, false, KeyCode::Delete),
        Some("deny")
    );
    assert_eq!(
        decision_shortcut(OverlayKind::PlanReview, true, KeyCode::Char('a')),
        None
    );
}

#[test]
fn inbound_approval_preserves_typed_overlay_input() {
    let mut state = TranscriptState::new("client");
    state.set_thread("thread");
    let mut ui = UiState::new("thread".into(), "client".into());
    let mut overlay = Overlay::new(OverlayKind::Rename, "Rename");
    overlay.input = Some("draft title".into());
    ui.overlay = Some(overlay);
    handle_web_event(
        &web_event(json!({
            "event":"approval_request","client_id":"client","thread_id":"thread",
            "request_id":"approval-1","tool_name":"shell"
        })),
        &mut state,
        &mut ui,
    );
    assert_eq!(
        ui.overlay
            .as_ref()
            .and_then(|overlay| overlay.input.as_deref()),
        Some("draft title")
    );
    assert_eq!(ui.pending_approvals.len(), 1);
}

#[test]
fn inbound_plan_review_waits_for_typed_overlay_to_close() {
    let mut state = TranscriptState::new("client");
    state.set_thread("thread");
    let mut ui = UiState::new("thread".into(), "client".into());
    let mut overlay = Overlay::new(OverlayKind::Rename, "Rename");
    overlay.input = Some("draft title".into());
    ui.overlay = Some(overlay);
    handle_web_event(
        &web_event(json!({
            "event":"plan_review_request","client_id":"client","thread_id":"thread",
            "request_id":"review-1","message":"Review this plan",
            "args":{"steps":["Inspect","Implement"]}
        })),
        &mut state,
        &mut ui,
    );
    assert_eq!(
        ui.overlay
            .as_ref()
            .and_then(|overlay| overlay.input.as_deref()),
        Some("draft title")
    );

    ui.overlay = None;
    present_pending_plan_review(&mut ui);
    assert_eq!(
        ui.overlay.as_ref().map(|overlay| overlay.kind),
        Some(OverlayKind::PlanReview)
    );
    assert_eq!(
        ui.pending_plan_review
            .as_ref()
            .map(|review| review.request_id.as_str()),
        Some("review-1")
    );
}
#[test]
fn failed_account_thread_replacement_rejects_old_and_unscoped_activity() {
    let mut state = TranscriptState::new("client");
    let mut ui = UiState::new(String::new(), "client".into());
    // Identity reset clears the binding before replacement. A failed local
    // threads_create_new leaves it empty, so neither scoped nor legacy blank
    // events may repopulate another account's newly cleared conversation.
    state.push_system("Could not create a new thread (see logs).");
    for thread in ["previous-account-thread", ""] {
        handle_web_event(
            &web_event(json!({
                "event":"text_delta","client_id":"client","thread_id":thread,
                "request_id":"old-turn","delta":"old secret"
            })),
            &mut state,
            &mut ui,
        );
        handle_web_event(
            &web_event(json!({
                "event":"approval_request","client_id":"client","thread_id":thread,
                "request_id":"old-approval"
            })),
            &mut state,
            &mut ui,
        );
    }
    assert_eq!(state.entries().len(), 1);
    assert!(!state.is_streaming());
    assert!(ui.pending_approvals.is_empty());
}
