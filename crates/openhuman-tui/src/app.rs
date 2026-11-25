//! Terminal chat event loop.
//!
//! Bridges three async sources over `tokio::select!`:
//!   * **keyboard** — a blocking crossterm reader thread forwards `Event`s over
//!     an mpsc channel (crossterm's own async `EventStream` needs the
//!     `event-stream` feature; the poll+forward thread keeps the dep surface
//!     minimal and exits promptly via the shared `shutdown` flag),
//!   * **web-channel broadcast** — the same `web_chat` event stream the desktop
//!     app consumes, folded into [`TranscriptState`] by its reducer,
//!   * **a spinner ticker** — animates the streaming indicator.
//!
//! All state transitions are logged with the `[tui]` prefix to the file-only
//! subscriber (see `logging::init_for_tui`); nothing is ever `println!`'d.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent};
use serde_json::json;
use tokio::sync::broadcast;

use openhuman_rpc::embed::chat_surface as web_chat;
use openhuman_rpc::embed::chat_surface::ScopedWebEvent;
use openhuman_rpc::embed::CoreRuntime;

use super::actions::Action;
use super::cockpit::{Overlay, OverlayKind, OverlayRow, PendingApproval, PendingPlanReview};
use super::render;
use super::state::TranscriptState;
use super::terminal::TerminalGuard;
use super::ui_state::{AppTab, UiState};
mod artifacts;
mod commands;
mod events;
mod files;
mod overlays;
mod thread_actions;
use artifacts::*;
use commands::execute_command;
use events::handle_web_event;
use files::*;
use overlays::*;
use thread_actions::*;

#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub initial_prompt: Option<String>,
    pub resume_picker: bool,
    pub no_alt_screen: bool,
    pub no_mouse: bool,
}

/// Run the tabbed terminal loop until the user quits (Ctrl+C / Ctrl+D) or the
/// web-channel bus closes. The [`TerminalGuard`] restores the terminal on every
/// exit path, including panics.
pub async fn run(
    runtime: Arc<CoreRuntime>,
    client_id: String,
    thread_id: String,
    mut web_rx: broadcast::Receiver<ScopedWebEvent>,
    options: LaunchOptions,
) -> anyhow::Result<()> {
    let mut state = TranscriptState::new(client_id.clone());
    state.set_thread(thread_id.clone());
    let mut ui = UiState::new(thread_id, client_id.clone());
    let manager = super::session::session_manager(&runtime);
    let mut session_rx = manager.subscribe();
    let (overlay_tx, mut overlay_rx) = tokio::sync::mpsc::unbounded_channel();
    ui.overlay_tx = Some(overlay_tx);
    let (auth_tx, mut auth_rx) = tokio::sync::mpsc::unbounded_channel();
    ui.auth_tx = Some(auth_tx);
    ui.mouse_enabled = !options.no_mouse && !options.no_alt_screen;
    load_transcript(&runtime, &mut state, &ui.thread_id).await;
    if state.entries().is_empty() {
        state.push_system("OpenHuman is ready. Type /help for the agent cockpit.".to_string());
    }
    super::controls::refresh_config(&runtime, &mut ui).await;
    super::controls::refresh_auth(&runtime, &mut ui).await;
    ui.identity_changed = false; // Startup is already bound to this persisted identity.
    refresh_agent_paths(&runtime, &mut ui).await;
    if options.resume_picker {
        open_rpc_overlay(
            &runtime,
            &mut ui,
            OverlayKind::Threads,
            "Saved threads",
            "openhuman.threads_list",
            json!({}),
            &["threads", "items"],
            &["id", "thread_id"],
            &["title", "name", "id"],
        )
        .await;
    }
    // Resolve local startup state before taking over the terminal. A slow or
    // locked config must never strand the user on a blank raw-mode screen.
    let mut guard = TerminalGuard::enter_with_options(!options.no_alt_screen)?;
    guard.set_mouse(ui.mouse_enabled)?;
    super::account::refresh(&runtime, &mut ui);

    if let Some(prompt) = options.initial_prompt {
        ui.composer.set_text(prompt);
        send_message(&runtime, &client_id, &mut state, &mut ui, "interrupt");
    }

    // Blocking crossterm reader → async channel.
    let (input_tx, mut input_rx) = tokio::sync::mpsc::channel::<Event>(256);
    let shutdown = Arc::new(AtomicBool::new(false));
    let reader_shutdown = shutdown.clone();
    let reader = std::thread::spawn(move || {
        while !reader_shutdown.load(Ordering::Relaxed) {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(ev) => {
                        if input_tx.blocking_send(ev).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
    });

    let mut ticker = tokio::time::interval(Duration::from_millis(120));
    let mut last_animation = std::time::Instant::now();
    let mut quit = false;
    let mut dirty = true;
    let mut last_draw = std::time::Instant::now() - Duration::from_secs(1);

    while !quit {
        if dirty && last_draw.elapsed() >= Duration::from_millis(16) {
            guard.set_mouse(ui.mouse_enabled)?;
            guard
                .terminal()
                .draw(|f| render::draw(f, &state, &mut ui))?;
            dirty = false;
            last_draw = std::time::Instant::now();
        }

        tokio::select! {
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(last_draw+Duration::from_millis(16))), if dirty => {},
            Some(message)=auth_rx.recv()=>{super::account::apply(message,&mut ui);dirty=true;},
            event=session_rx.recv()=>{
                match event {
                    Ok(openhuman_rpc::tinyhumans::SessionEvent::Changed(session))=>super::account::apply_session(&session,&mut ui),
                    Ok(openhuman_rpc::tinyhumans::SessionEvent::Expired {..})=>{ui.auth_summary="Session expired".into();ui.auth_user_id=None;ui.auth_profile_id=None;ui.authenticated=false;ui.account_detail.clear();ui.identity_changed=true;ui.settings_status="Your saved session was rejected. Sign in again.".into();},
                    Err(broadcast::error::RecvError::Lagged(_))=>super::account::refresh(&runtime,&mut ui),
                    Err(broadcast::error::RecvError::Closed)=>{},
                } dirty=true;
            },
            Some(reply)=overlay_rx.recv()=>{super::effects::apply(reply,&mut ui);dirty=true;},
            maybe_ev = input_rx.recv() => match maybe_ev {
                Some(Event::Key(key)) => {
                    dirty = true;
                    if handle_key(key, &runtime, &client_id, &mut state, &mut ui).await {
                        quit = true;
                    }
                }
                Some(Event::Paste(text)) => { handle_paste(&text, &mut ui); dirty = true; },
                Some(Event::Mouse(mouse)) => {
                    if ui.mouse_enabled { quit = handle_mouse(mouse, &runtime, &client_id, &mut state, &mut ui).await; dirty = true; }
                }
                Some(Event::Resize(_, _)) => { ui.hits.clear(); dirty = true; },
                Some(_) => {},
                None => quit = true, // reader thread gone
            },
            recv = web_rx.recv() => match recv {
                Ok(ev) => {
                    let before=ui.viewport.total_rows();handle_web_event(ev.wire_event(), &mut state, &mut ui);
                    if ui.scroll_from_bottom>0 {ui.viewport.rows(&state,ui.transcript_area.width.saturating_sub(2),ui.transcript_area.height,ui.scroll_from_bottom);ui.scroll_from_bottom=ui.scroll_from_bottom.saturating_add(ui.viewport.total_rows().saturating_sub(before));}
                    dirty = true;
                },
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    log::warn!("[tui] web-channel lagged, dropped {n} events");
                    state.push_system("Some activity events were missed; /resume reloads stored history."); dirty = true;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    log::warn!("[tui] web-channel closed — exiting");
                    quit = true;
                }
            },
            _ = ticker.tick(), if state.is_streaming() => {
                if state.is_streaming() && last_animation.elapsed()>=Duration::from_millis(120) { ui.spinner_tick = ui.spinner_tick.wrapping_add(1); dirty = true; last_animation=std::time::Instant::now(); }
            }
        }
        if ui.identity_changed {
            ui.identity_changed = false;
            ui.drafts.clear();
            ui.composer.clear();
            ui.pending_approvals.clear();
            ui.pending_plan_review = None;
            ui.model_override = None;
            ui.viewport = Default::default();
            ui.overlay = None;
            ui.thread_id.clear();
            ui.action_dir.clear();
            ui.scroll_from_bottom = 0;
            state = TranscriptState::new(client_id.clone());
            super::controls::refresh_config(&runtime, &mut ui).await;
            refresh_agent_paths(&runtime, &mut ui).await;
            new_thread(&runtime, &mut state, &mut ui).await;
            dirty = true;
        }
    }

    shutdown.store(true, Ordering::Relaxed);
    drop(input_rx); // Release a reader blocked on a full input queue before joining.
    let _ = reader.join();
    log::info!("[tui] event loop exited");
    Ok(())
}

/// Handle a key event. Returns `true` when the app should quit.
async fn handle_key(
    key: KeyEvent,
    runtime: &Arc<CoreRuntime>,
    client_id: &str,
    state: &mut TranscriptState,
    ui: &mut UiState,
) -> bool {
    // Ignore key-release events (Windows / kitty report both edges).
    if key.kind == KeyEventKind::Release {
        return false;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ui.auth_pending && key.code == KeyCode::Esc {
        super::account::cancel(ui);
        return false;
    }

    if ui.active_tab == AppTab::Chat
        && ui.overlay.is_none()
        && ui.composer.command_query().is_some()
    {
        let matches = ui.composer.command_matches();
        match key.code {
            KeyCode::Up => {
                ui.suggestion_selected = ui.suggestion_selected.saturating_sub(1);
                return false;
            }
            KeyCode::Down => {
                ui.suggestion_selected =
                    (ui.suggestion_selected + 1).min(matches.len().saturating_sub(1));
                return false;
            }
            KeyCode::Enter | KeyCode::Tab
                if !key.modifiers.intersects(
                    KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL,
                ) =>
            {
                if let Some((name, _)) =
                    matches.get(ui.suggestion_selected.min(matches.len().saturating_sub(1)))
                {
                    ui.composer.set_text(format!("/{name} "));
                    ui.suggestion_selected = 0;
                    if key.code == KeyCode::Enter {
                        return send_or_command(runtime, client_id, state, ui).await;
                    }
                    return false;
                }
            }
            KeyCode::Char(_) | KeyCode::Backspace => ui.suggestion_selected = 0,
            _ => {}
        }
    }

    if matches!(key.code, KeyCode::Char('c')) && ctrl {
        if ui.login_token.is_some() {
            super::controls::handle_settings_key(
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                runtime,
                ui,
            )
            .await;
        } else if ui.overlay.is_some() {
            ui.overlay = None;
        } else if !ui.composer.is_empty() {
            ui.composer.clear();
        } else if state.is_streaming() {
            cancel_turn(runtime, client_id, &ui.thread_id, state);
            ui.stopping = true;
        } else {
            state.push_system(
                "Ctrl+D or /quit exits. Closing this TUI shuts down its local runtime.",
            );
        }
        return false;
    }
    if matches!(key.code, KeyCode::Char('d')) && ctrl {
        log::info!("[tui] quit via Ctrl+D");
        return true;
    }

    if ctrl
        && key.code == KeyCode::Char('p')
        && ui.login_token.is_none()
        && ui.config_edit.is_none()
    {
        return execute_command("help", runtime, client_id, state, ui).await;
    }
    if key.code == KeyCode::Tab
        && !ctrl
        && ui.composer.command_query().is_none()
        && !ui.is_editing()
    {
        ui.focus = (ui.focus + 1) % (ui.hits.len() + 1);
        return false;
    }
    if key.code == KeyCode::BackTab && !ctrl && !ui.is_editing() {
        ui.focus = if ui.focus == 0 {
            ui.hits.len()
        } else {
            ui.focus - 1
        };
        return false;
    }
    if ui.focus > 0 && key.code == KeyCode::Enter && ui.overlay.is_none() && !ui.is_editing() {
        if let Some(hit) = ui.hits.get(ui.focus - 1) {
            let action = hit.action.clone();
            return dispatch_action(action, runtime, client_id, state, ui).await;
        }
    }

    if ui.overlay.is_some() {
        let previous_kind = ui.overlay.as_ref().map(|overlay| overlay.kind);
        let should_quit = handle_overlay_key(key, runtime, client_id, state, ui).await;
        if previous_kind != Some(OverlayKind::PlanReview) && ui.overlay.is_none() {
            present_pending_plan_review(ui);
        }
        return should_quit;
    }

    if !ui.is_editing() {
        if let Some(tab) = tab_shortcut(key, ui.active_tab) {
            ui.active_tab = tab;
        } else {
            return handle_tab_key(key, runtime, client_id, state, ui).await;
        }
        return false;
    }

    handle_tab_key(key, runtime, client_id, state, ui).await
}

fn tab_shortcut(key: KeyEvent, current: AppTab) -> Option<AppTab> {
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Tab if ctrl && shift => Some(current.previous()),
        KeyCode::Tab if ctrl => Some(current.next()),
        KeyCode::BackTab if ctrl => Some(current.previous()),
        KeyCode::Char('1') if alt => Some(AppTab::Logs),
        KeyCode::Char('2') if alt => Some(AppTab::Chat),
        KeyCode::Char('3') if alt => Some(AppTab::Config),
        KeyCode::Char('4') if alt => Some(AppTab::Settings),
        _ => None,
    }
}

async fn handle_mouse(
    mouse: MouseEvent,
    runtime: &Arc<CoreRuntime>,
    client_id: &str,
    state: &mut TranscriptState,
    ui: &mut UiState,
) -> bool {
    let action = super::input::mouse_action(mouse, ui);
    if let Some(action) = action {
        return dispatch_action(action, runtime, client_id, state, ui).await;
    }
    false
}

async fn dispatch_action(
    action: Action,
    runtime: &Arc<CoreRuntime>,
    client_id: &str,
    state: &mut TranscriptState,
    ui: &mut UiState,
) -> bool {
    if (ui.login_token.is_some() || ui.config_edit.is_some() || ui.logout_confirm)
        && !matches!(action, Action::OverlayKey(_))
    {
        return false;
    }
    match action {
        Action::Command(command) => {
            return execute_command(command, runtime, client_id, state, ui).await;
        }
        Action::View(tab) => {
            ui.active_tab = tab;
            ui.focus = 0;
        }
        Action::Send => return send_or_command(runtime, client_id, state, ui).await,
        Action::Queue => send_message(runtime, client_id, state, ui, "followup"),
        Action::Stop => {
            cancel_turn(runtime, client_id, &ui.thread_id, state);
            ui.stopping = state.is_streaming();
        }
        Action::Latest => ui.scroll_from_bottom = 0,
        Action::Entry(index) => state.toggle_entry(index),
        Action::Composer => ui.focus = 0,
        Action::Complete(command) => {
            ui.composer.set_text(format!("/{command} "));
            ui.focus = 0;
        }
        Action::OverlayRow(index) => {
            if let Some(overlay) = &mut ui.overlay {
                overlay.selected = index;
            }
            return handle_overlay_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                runtime,
                client_id,
                state,
                ui,
            )
            .await;
        }
        Action::OverlayKey(code) => {
            let key = KeyEvent::new(code, KeyModifiers::NONE);
            if ui.overlay.is_some() {
                return handle_overlay_key(key, runtime, client_id, state, ui).await;
            }
            return handle_tab_key(key, runtime, client_id, state, ui).await;
        }
        Action::ConfigRow(index) => {
            ui.config_selected = index;
            super::controls::handle_config_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                runtime,
                ui,
            )
            .await;
        }
        Action::SettingsRow(index) => {
            ui.settings_selected = index;
            super::controls::handle_settings_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                runtime,
                ui,
            )
            .await;
        }
    }
    false
}

fn handle_paste(text: &str, ui: &mut UiState) {
    if let Some(overlay) = &mut ui.overlay {
        if let Some(input) = &mut overlay.input {
            input.push_str(text);
        } else {
            overlay.filter.push_str(text);
            overlay.clamp_selection();
        }
        return;
    }
    match ui.active_tab {
        AppTab::Chat => ui.composer.insert_str(text),
        AppTab::Config => {
            if let Some(input) = &mut ui.config_edit {
                input.push_str(text);
            }
        }
        AppTab::Settings => {
            if let Some(token) = &mut ui.login_token {
                token.push_str(text);
            }
        }
        AppTab::Logs => {}
    }
}

async fn handle_tab_key(
    key: KeyEvent,
    runtime: &Arc<CoreRuntime>,
    client_id: &str,
    state: &mut TranscriptState,
    ui: &mut UiState,
) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match ui.active_tab {
        AppTab::Logs => match key.code {
            KeyCode::PageUp | KeyCode::Up => {
                ui.log_scroll_from_bottom = ui.log_scroll_from_bottom.saturating_add(5)
            }
            KeyCode::PageDown | KeyCode::Down => {
                ui.log_scroll_from_bottom = ui.log_scroll_from_bottom.saturating_sub(5)
            }
            _ => {}
        },
        AppTab::Chat => match key.code {
            KeyCode::Char('c') if ctrl => {
                log::info!("[tui] quit via Ctrl+C");
                return true;
            }
            KeyCode::Char('d') if ctrl => {
                log::info!("[tui] quit via Ctrl+D");
                return true;
            }
            KeyCode::Char('n') if ctrl => new_thread(runtime, state, ui).await,
            KeyCode::Esc => {
                cancel_turn(runtime, client_id, &ui.thread_id, state);
                ui.stopping = state.is_streaming();
                ui.focus = 0;
            }
            KeyCode::PageUp => {
                ui.scroll_from_bottom = ui.scroll_from_bottom.saturating_add(5);
            }
            KeyCode::PageDown => {
                ui.scroll_from_bottom = ui.scroll_from_bottom.saturating_sub(5);
            }
            KeyCode::Enter
                if key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                ui.composer.newline()
            }
            KeyCode::Enter => return send_or_command(runtime, client_id, state, ui).await,
            KeyCode::Char('q') if ctrl && state.is_streaming() => {
                send_message(runtime, client_id, state, ui, "followup")
            }
            KeyCode::Char('j') if ctrl => ui.composer.newline(),
            KeyCode::Tab => {
                if !ui.composer.complete_command() && ui.composer.file_query().is_some() {
                    open_file_picker(ui).await;
                }
            }
            KeyCode::Backspace => ui.composer.backspace(),
            KeyCode::Delete => ui.composer.delete(),
            KeyCode::Left => ui.composer.move_left(),
            KeyCode::Right => ui.composer.move_right(),
            KeyCode::Home => ui.composer.move_home(),
            KeyCode::End => ui.composer.move_end(),
            KeyCode::Up if ui.composer.text().lines().count() <= 1 => {
                ui.composer.history_previous()
            }
            KeyCode::Down if ui.composer.text().lines().count() <= 1 => ui.composer.history_next(),
            KeyCode::Char('w') if ctrl => ui.composer.delete_word_back(),
            KeyCode::Char('r') if ctrl => open_history_search(ui),
            KeyCode::Char(c) if !ctrl => ui.composer.insert_char(c),
            _ => {}
        },
        AppTab::Config => super::controls::handle_config_key(key, runtime, ui).await,
        AppTab::Settings => super::controls::handle_settings_key(key, runtime, ui).await,
    }
    false
}

async fn send_or_command(
    runtime: &Arc<CoreRuntime>,
    client_id: &str,
    state: &mut TranscriptState,
    ui: &mut UiState,
) -> bool {
    if ui.composer.is_empty() {
        return false;
    }
    let text = ui.composer.text().trim().to_string();
    if let Some(command) = text.strip_prefix('/') {
        if command.starts_with("login") {
            ui.composer.clear();
        } else {
            let _ = ui.composer.take_for_send();
        }
        return execute_command(command, runtime, client_id, state, ui).await;
    }
    let mode = if state.is_streaming() {
        "steer"
    } else {
        "interrupt"
    };
    send_message(runtime, client_id, state, ui, mode);
    false
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
