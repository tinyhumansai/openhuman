//! Offline preview: the production renderer and input geometry over sample events.
use super::actions::Action;
use super::cockpit::{Overlay, OverlayKind};
use super::state::TranscriptState;
use super::ui_state::{AppTab, UiState};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use openhuman_rpc::embed::chat_surface::{WebChannelEvent, WebEventContext};

pub fn sample() -> TranscriptState {
    let mut state = TranscriptState::new("preview");
    state.set_thread("preview");
    state.begin_user_turn("Make the terminal UI simpler, with mouse controls.");
    let context = || WebEventContext {
        client_id: "preview".into(),
        thread_id: "preview".into(),
        request_id: "sample-turn".into(),
        ..Default::default()
    };
    state.apply_event(&WebChannelEvent::TextDelta {
        context: context(),
        delta: Some("I’ll keep the conversation central and make the work easy to inspect.".into()),
        delta_kind: None,
        round: None,
    });
    state.apply_event(&WebChannelEvent::ToolCall {
        context: context(),
        tool_name: Some("Read source".into()),
        skill_id: None,
        args: Some(serde_json::json!({"path":"src/render.rs"})),
        round: None,
        tool_call_id: Some("read-1".into()),
        tool_display_label: None,
        tool_display_detail: None,
    });
    state.apply_event(&WebChannelEvent::ToolResult {
        context: context(),
        tool_name: Some("Read source".into()),
        skill_id: None,
        output: Some("Header → conversation → composer → status\nCached viewport rendering\nShared mouse and keyboard actions".into()),
        args: None,
        success: Some(true),
        round: None,
        tool_call_id: Some("read-1".into()),
        failure: None,
        elapsed_ms: Some(12),
        tool_display_label: None,
        tool_display_detail: None,
        structured: None,
    });
    state.apply_event(&WebChannelEvent::SubagentSpawned {
        context: context(),
        message: None,
        tool_name: Some("Explore".into()),
        skill_id: Some("child-1".into()),
        round: None,
        subagent: None,
    });
    state.apply_event(&WebChannelEvent::SubagentTextDelta {
        context: context(),
        delta: Some("Checked the command, login, and tool-call surfaces.\nMouse targets use the displayed layout.".into()),
        delta_kind: None,
        skill_id: Some("child-1".into()),
        round: None,
        subagent: Some(openhuman_rpc::embed::chat_surface::SubagentProgressDetail {
            task_id: Some("child-1".into()),
            display_name: Some("Explore".into()),
            ..Default::default()
        }),
    });
    state.apply_event(&WebChannelEvent::SubagentCompleted {
        context: context(),
        message: None,
        tool_name: None,
        skill_id: Some("child-1".into()),
        success: Some(true),
        round: None,
        subagent: Some(openhuman_rpc::embed::chat_surface::SubagentProgressDetail {
            task_id: Some("child-1".into()),
            display_name: Some("Explore".into()),
            elapsed_ms: Some(840),
            ..Default::default()
        }),
    });
    state.apply_event(&WebChannelEvent::ChatDone {
        context: context(),
        full_response: Some("Ready to inspect. Click a tool row to expand it, use Tools or /subagents for details, and choose a theme in Settings.\n\nThis is an offline preview; no model or account calls are made.".into()),
        citations: None,
        usage: None,
        timing: None,
        parts: None,
    });
    state
}

pub fn run(no_mouse: bool) -> anyhow::Result<()> {
    let mut state = sample();
    let mut ui = UiState::new("preview".into(), "preview".into());
    ui.demo = true;
    ui.mouse_enabled = !no_mouse;
    ui.auth_summary = "Offline preview".into();
    ui.model_override = Some("Preview model".into());
    ui.action_dir = "preview".into();
    let mut terminal = super::terminal::TerminalGuard::enter_with_options(true)?;
    terminal.set_mouse(ui.mouse_enabled)?;
    let mut dirty = true;
    loop {
        if dirty {
            terminal.set_mouse(ui.mouse_enabled)?;
            terminal
                .terminal()
                .draw(|f| super::render::draw(f, &state, &mut ui))?;
            dirty = false;
        }
        if !event::poll(std::time::Duration::from_millis(100))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) => {
                if key.kind == crossterm::event::KeyEventKind::Release {
                    continue;
                }
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                if ctrl && key.code == KeyCode::Char('d') {
                    break;
                }
                if ctrl && key.code == KeyCode::Char('p') {
                    command("help", &mut state, &mut ui);
                } else if ui.overlay.is_some() {
                    match key.code {
                        KeyCode::Esc => ui.overlay = None,
                        KeyCode::Enter => {
                            if activate_overlay(&mut state, &mut ui) {
                                break;
                            }
                        }
                        KeyCode::Up => {
                            let o = ui.overlay.as_mut().unwrap();
                            o.selected = o.selected.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            let o = ui.overlay.as_mut().unwrap();
                            o.selected =
                                (o.selected + 1).min(o.visible_rows().len().saturating_sub(1));
                        }
                        KeyCode::Backspace => {
                            let o = ui.overlay.as_mut().unwrap();
                            o.filter.pop();
                            o.clamp_selection();
                        }
                        KeyCode::Char(c) if !ctrl => {
                            let o = ui.overlay.as_mut().unwrap();
                            o.filter.push(c);
                            o.clamp_selection();
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char('c') if ctrl => ui.composer.clear(),
                        KeyCode::Char('j') if ctrl => ui.composer.newline(),
                        KeyCode::Tab => {
                            if !ui.composer.complete_command() {
                                ui.focus = (ui.focus + 1) % (ui.hits.len() + 1);
                            }
                        }
                        KeyCode::BackTab => ui.focus = ui.focus.saturating_sub(1),
                        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                            ui.composer.newline()
                        }
                        KeyCode::Enter => {
                            let action = if ui.focus > 0 {
                                ui.hits
                                    .get(ui.focus - 1)
                                    .map(|hit| hit.action.clone())
                                    .unwrap_or(Action::Send)
                            } else {
                                Action::Send
                            };
                            if act(action, &mut state, &mut ui) {
                                break;
                            }
                        }
                        KeyCode::Esc => {
                            ui.active_tab = AppTab::Chat;
                            ui.focus = 0;
                        }
                        KeyCode::PageUp => {
                            ui.scroll_from_bottom = ui.scroll_from_bottom.saturating_add(10)
                        }
                        KeyCode::PageDown => {
                            ui.scroll_from_bottom = ui.scroll_from_bottom.saturating_sub(10)
                        }
                        KeyCode::Backspace => ui.composer.backspace(),
                        KeyCode::Delete => ui.composer.delete(),
                        KeyCode::Left => ui.composer.move_left(),
                        KeyCode::Right => ui.composer.move_right(),
                        KeyCode::Home => ui.composer.move_home(),
                        KeyCode::End => ui.composer.move_end(),
                        KeyCode::Char(c) if !ctrl => ui.composer.insert_char(c),
                        _ => {}
                    }
                }
                dirty = true;
            }
            Event::Mouse(mouse) if ui.mouse_enabled => {
                if let Some(action) = super::input::mouse_action(mouse, &mut ui) {
                    if act(action, &mut state, &mut ui) {
                        break;
                    }
                }
                dirty = true;
            }
            Event::Resize(_, _) => {
                ui.hits.clear();
                dirty = true;
            }
            Event::Paste(text) => {
                if let Some(o) = &mut ui.overlay {
                    o.filter.push_str(&text);
                    o.clamp_selection();
                } else {
                    ui.composer.insert_str(&text);
                }
                dirty = true;
            }
            _ => {}
        }
    }
    Ok(())
}

fn act(action: Action, state: &mut TranscriptState, ui: &mut UiState) -> bool {
    match action {
        Action::Command(cmd) => return command(cmd, state, ui),
        Action::View(tab) => {
            ui.active_tab = tab;
            ui.focus = 0;
        }
        Action::Send => {
            if let Some(message) = ui.composer.take_for_send() {
                if let Some(cmd) = message.strip_prefix('/') {
                    return command(cmd, state, ui);
                }
                state
                    .push_system("Offline preview: use the live TUI to send messages to an agent.");
                ui.scroll_from_bottom = 0;
            }
        }
        Action::Entry(index) => state.toggle_entry(index),
        Action::Latest => ui.scroll_from_bottom = 0,
        Action::Complete(cmd) => {
            ui.composer.set_text(format!("/{cmd} "));
            ui.focus = 0;
        }
        Action::Composer => ui.focus = 0,
        Action::OverlayRow(index) => {
            if let Some(o) = &mut ui.overlay {
                o.selected = index;
            }
            return activate_overlay(state, ui);
        }
        Action::OverlayKey(KeyCode::Esc) => ui.overlay = None,
        Action::OverlayKey(KeyCode::Enter) => return activate_overlay(state, ui),
        Action::SettingsRow(_) => {
            ui.settings_status =
                "Offline preview. Start without --demo for real account login.".into()
        }
        _ => {}
    }
    false
}
fn command(cmd: &str, state: &mut TranscriptState, ui: &mut UiState) -> bool {
    let cmd = cmd.trim();
    if super::presentation::open(cmd, state, ui) {
        return false;
    }
    match cmd {
        "quit" => return true,
        "new" => {
            *state = TranscriptState::new("preview");
            ui.viewport = Default::default();
            ui.scroll_from_bottom = 0;
        }
        "settings" | "login" | "logout" => {
            ui.active_tab = AppTab::Settings;
            ui.settings_status = "Offline preview. Start without --demo to sign in.".into();
        }
        "config" => ui.active_tab = AppTab::Config,
        "logs" => ui.active_tab = AppTab::Logs,
        "agents" | "sessions" | "resume" | "model" => {
            let kind = match cmd {
                "agents" => OverlayKind::Agents,
                "model" => OverlayKind::Model,
                _ => OverlayKind::Threads,
            };
            let mut o = Overlay::new(kind, "Preview choices");
            o.rows = vec![super::presentation::row(
                "preview",
                "Preview conversation / agent / model",
                "Sample data; live mode reads the runtime catalog",
            )];
            ui.overlay = Some(o);
        }
        _ => state.push_system("This action needs live mode. Start the TUI without --demo."),
    }
    ui.focus = 0;
    false
}
fn activate_overlay(state: &mut TranscriptState, ui: &mut UiState) -> bool {
    let Some(o) = &ui.overlay else {
        return false;
    };
    let kind = o.kind;
    let selected = o.visible_rows().get(o.selected).map(|row| row.id.clone());
    if let Some(id) = selected {
        match kind {
            OverlayKind::Help => {
                ui.overlay = None;
                return command(&id, state, ui);
            }
            OverlayKind::Themes => super::presentation::select_theme(&id, ui),
            OverlayKind::Tools | OverlayKind::Subagents => {
                super::presentation::inspect(&id, state, ui)
            }
            _ => ui.overlay = None,
        }
    }
    false
}

#[cfg(test)]
#[path = "demo_tests.rs"]
mod tests;
