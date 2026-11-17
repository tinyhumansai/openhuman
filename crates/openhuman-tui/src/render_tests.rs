use super::*;
use ratatui::backend::{Backend, TestBackend};
use ratatui::Terminal;
fn rendered(width: u16, height: u16, state: &TranscriptState, ui: &mut UiState) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| draw(f, state, ui)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
#[test]
fn conversation_and_click_targets_fit_standard_terminal() {
    let mut ui = UiState::new("thread".into(), "client".into());
    let state = TranscriptState::new("client");
    let output = rendered(80, 24, &state, &mut ui);
    for label in [
        "OpenHuman",
        "Sessions",
        "Agents",
        "Tools",
        "Settings",
        "Describe a task",
    ] {
        assert!(output.contains(label), "{label}");
    }
    assert!(!output.contains("1 Logs"));
    assert!(ui.hits.iter().any(|hit| hit.action == Action::Send));
    assert!(ui
        .hits
        .iter()
        .all(|hit| hit.area.right() <= 80 && hit.area.bottom() <= 24));
}
#[test]
fn masked_login_does_not_render_a_secret() {
    let mut ui = UiState::new("thread".into(), "client".into());
    ui.active_tab = AppTab::Settings;
    ui.login_token = Some("private-token-never-render".into());
    let output = rendered(80, 24, &TranscriptState::new("client"), &mut ui);
    assert!(!output.contains("private-token"));
    assert!(output.contains("••••"));
}
#[test]
fn overlay_removes_covered_controls() {
    let mut ui = UiState::new("thread".into(), "client".into());
    ui.overlay = Some(super::super::cockpit::Overlay::new(
        OverlayKind::Help,
        "Commands",
    ));
    rendered(80, 24, &TranscriptState::new("client"), &mut ui);
    assert!(!ui.hits.iter().any(|hit| hit.action == Action::Send));
}
#[test]
fn small_terminal_and_unicode_do_not_panic() {
    let mut ui = UiState::new("thread".into(), "client".into());
    ui.composer.set_text("界界e\u{301}\n".repeat(20));
    for (width, height) in [(20, 5), (40, 12), (80, 24), (120, 40)] {
        rendered(width, height, &TranscriptState::new("client"), &mut ui);
    }
}

#[test]
fn composer_stays_above_status_on_resize_with_cursor_and_controls_inside_it() {
    let state = TranscriptState::new("client");
    let mut ui = UiState::new("thread".into(), "client".into());
    for draft in ["short", "/", "界e\u{301}\nlong draft\n".repeat(20).as_str()] {
        ui.composer.set_text(draft);
        for (width, height) in [(80, 24), (120, 40), (40, 12), (24, 8), (80, 24)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| draw(f, &state, &mut ui)).unwrap();
            let send = ui
                .hits
                .iter()
                .find(|hit| hit.action == Action::Send)
                .unwrap();
            assert_eq!(send.area.y, height - 3);
            assert!(ui.composer_area.bottom() <= send.area.y);
            assert!(ui.transcript_area.bottom() <= ui.composer_area.y);
            let (x, y) = terminal.backend_mut().get_cursor_position().unwrap().into();
            assert!(x >= ui.composer_area.x && x < ui.composer_area.right());
            assert!(y >= ui.composer_area.y && y < ui.composer_area.bottom());
            assert!(ui
                .hits
                .iter()
                .all(|hit| hit.area.right() <= width && hit.area.bottom() < height));
        }
    }
    rendered(20, 5, &state, &mut ui);
    assert_eq!(ui.composer_area, Rect::default());
    assert!(ui.hits.is_empty());
}

#[test]
fn long_selectors_do_not_overlap_send_or_stream_controls_in_narrow_terminals() {
    let mut state = TranscriptState::new("client");
    let mut ui = UiState::new("thread".into(), "client".into());
    ui.agent_name = "A very long named agent".repeat(4);
    ui.model_override = Some("A very long model".repeat(4));
    for streaming in [false, true] {
        if streaming {
            state.begin_user_turn("work");
        }
        for width in [24, 40, 80, 120] {
            rendered(width, 24, &state, &mut ui);
            let controls: Vec<_> = ui.hits.iter().filter(|hit| hit.area.y == 21).collect();
            for (index, left) in controls.iter().enumerate() {
                for right in controls.iter().skip(index + 1) {
                    assert!(
                        left.area.intersection(right.area).is_empty(),
                        "overlapping actions {:?} and {:?} at width {width}",
                        left.action,
                        right.action
                    );
                }
            }
        }
    }
}
#[test]
fn pending_browser_signin_has_mouse_actions_without_rendering_its_link() {
    let mut ui = UiState::new("thread".into(), "client".into());
    ui.active_tab = AppTab::Settings;
    ui.auth_pending = true;
    ui.login_url = Some(zeroize::Zeroizing::new(
        "https://example.test/private-nonce".into(),
    ));
    let output = rendered(80, 24, &TranscriptState::new("client"), &mut ui);
    assert!(!output.contains("private-nonce"));
    for action in ["login-open", "login-copy", "login-cancel"] {
        assert!(ui
            .hits
            .iter()
            .any(|hit| hit.action == Action::Command(action)));
    }
}
