use super::*;
use crate::state::EntryKind;

fn ui() -> UiState {
    UiState::new("preview".into(), "preview".into())
}

#[test]
fn sample_transcript_has_user_turn_tools_subagent_and_final_answer() {
    let state = sample();
    let kinds: Vec<_> = state.entries().iter().map(|e| e.kind).collect();
    assert_eq!(kinds.first(), Some(&EntryKind::User));
    assert!(kinds.contains(&EntryKind::Tool));
    assert!(state
        .last_assistant()
        .unwrap()
        .contains("offline preview; no model or account calls"));
    assert!(!state.is_streaming());
    assert!(state
        .entries()
        .iter()
        .any(|e| e.activity.as_ref().is_some_and(|a| a.child)));
    assert!(state
        .entries()
        .iter()
        .any(|e| e.activity.as_ref().is_some_and(|a| !a.child)));
}

#[test]
fn sample_renders_on_the_production_renderer() {
    let state = sample();
    let mut ui = ui();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|f| crate::render::draw(f, &state, &mut ui))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Make the terminal UI simpler"));
}

#[test]
fn sending_in_preview_explains_it_is_offline_and_clears_the_composer() {
    let mut state = sample();
    let before = state.entries().len();
    let mut ui = ui();
    ui.scroll_from_bottom = 5;
    ui.composer.insert_str("hello there");
    assert!(!act(Action::Send, &mut state, &mut ui));
    assert_eq!(state.entries().len(), before + 1);
    assert!(state
        .entries()
        .last()
        .unwrap()
        .text
        .contains("Offline preview"));
    assert_eq!(ui.composer.text(), "");
    assert_eq!(ui.scroll_from_bottom, 0);
    // An empty composer sends nothing.
    assert!(!act(Action::Send, &mut state, &mut ui));
    assert_eq!(state.entries().len(), before + 1);
}

#[test]
fn slash_command_in_composer_runs_as_command_and_quit_exits() {
    let mut state = sample();
    let mut ui = ui();
    ui.composer.insert_str("/config");
    assert!(!act(Action::Send, &mut state, &mut ui));
    assert_eq!(ui.active_tab, AppTab::Config);
    ui.composer.insert_str("/quit");
    assert!(act(Action::Send, &mut state, &mut ui));
}

#[test]
fn simple_actions_update_view_state() {
    let mut state = sample();
    let mut ui = ui();
    ui.focus = 3;
    ui.scroll_from_bottom = 9;
    assert!(!act(Action::View(AppTab::Logs), &mut state, &mut ui));
    assert_eq!((ui.active_tab, ui.focus), (AppTab::Logs, 0));
    act(Action::Latest, &mut state, &mut ui);
    assert_eq!(ui.scroll_from_bottom, 0);
    ui.focus = 2;
    act(Action::Composer, &mut state, &mut ui);
    assert_eq!(ui.focus, 0);
    act(Action::Complete("help"), &mut state, &mut ui);
    assert_eq!(ui.composer.text(), "/help ");
    act(Action::SettingsRow(0), &mut state, &mut ui);
    assert!(ui.settings_status.contains("Offline preview"));
    let was = state.entries()[1].expanded;
    act(Action::Entry(1), &mut state, &mut ui);
    assert_ne!(state.entries()[1].expanded, was);
    act(Action::Stop, &mut state, &mut ui);
}

#[test]
fn commands_open_overlays_tabs_and_reset_the_conversation() {
    let mut state = sample();
    let mut ui = ui();
    for (cmd, kind) in [
        ("agents", OverlayKind::Agents),
        ("model", OverlayKind::Model),
        ("sessions", OverlayKind::Threads),
        ("resume", OverlayKind::Threads),
    ] {
        assert!(!command(cmd, &mut state, &mut ui));
        let overlay = ui.overlay.take().expect(cmd);
        assert_eq!(overlay.kind, kind);
        assert_eq!(overlay.rows.len(), 1);
    }
    for cmd in ["settings", "login", "logout"] {
        ui.active_tab = AppTab::Chat;
        command(cmd, &mut state, &mut ui);
        assert_eq!(ui.active_tab, AppTab::Settings);
        assert!(ui.settings_status.contains("sign in"));
    }
    command(" logs ", &mut state, &mut ui);
    assert_eq!(ui.active_tab, AppTab::Logs);
    command("help", &mut state, &mut ui);
    assert_eq!(ui.overlay.as_ref().unwrap().kind, OverlayKind::Help);
    let before = state.entries().len();
    command("compact", &mut state, &mut ui);
    assert_eq!(state.entries().len(), before + 1);
    assert!(state
        .entries()
        .last()
        .unwrap()
        .text
        .contains("needs live mode"));
    command("new", &mut state, &mut ui);
    assert!(state.entries().is_empty());
}

#[test]
fn overlay_rows_activate_by_kind() {
    let mut state = sample();
    let mut ui = ui();
    // No overlay: nothing happens.
    assert!(!activate_overlay(&mut state, &mut ui));

    // Help rows run their command and close the overlay; quit exits.
    command("help", &mut state, &mut ui);
    let quit = ui
        .overlay
        .as_ref()
        .unwrap()
        .visible_rows()
        .iter()
        .position(|r| r.id == "quit")
        .expect("quit listed in help");
    ui.overlay.as_mut().unwrap().selected = quit;
    assert!(activate_overlay(&mut state, &mut ui));
    assert!(ui.overlay.is_none());

    // Theme rows select the theme and close.
    command("themes", &mut state, &mut ui);
    let second = crate::theme::Theme::ALL[1];
    ui.overlay.as_mut().unwrap().selected = 1;
    assert!(!activate_overlay(&mut state, &mut ui));
    assert_eq!(ui.theme, second);
    assert!(ui.overlay.is_none());

    // Tool rows inspect into a detail overlay.
    command("tools", &mut state, &mut ui);
    assert!(!activate_overlay(&mut state, &mut ui));
    assert_eq!(ui.overlay.as_ref().unwrap().kind, OverlayKind::ToolDetail);

    // Other kinds just close.
    command("agents", &mut state, &mut ui);
    assert!(!act(Action::OverlayRow(0), &mut state, &mut ui));
    assert!(ui.overlay.is_none());
}

#[test]
fn overlay_key_actions_close_or_activate() {
    let mut state = sample();
    let mut ui = ui();
    command("agents", &mut state, &mut ui);
    act(Action::OverlayKey(KeyCode::Esc), &mut state, &mut ui);
    assert!(ui.overlay.is_none());
    command("agents", &mut state, &mut ui);
    act(Action::OverlayKey(KeyCode::Enter), &mut state, &mut ui);
    assert!(ui.overlay.is_none());
}
