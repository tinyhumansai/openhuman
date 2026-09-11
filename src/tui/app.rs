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
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde_json::json;
use tokio::sync::broadcast;

use crate::core::runtime::CoreRuntime;
use crate::core::socketio::WebChannelEvent;
use crate::openhuman::web_chat;

use super::cockpit::{
    array_at, row_from_value, Overlay, OverlayKind, OverlayRow, PendingApproval, PendingPlanReview,
};
use super::render;
use super::state::TranscriptState;
use super::terminal::TerminalGuard;
use super::ui_state::{AppTab, UiState};

#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub initial_prompt: Option<String>,
    pub resume_picker: bool,
    pub no_alt_screen: bool,
}

/// Run the tabbed terminal loop until the user quits (Ctrl+C / Ctrl+D) or the
/// web-channel bus closes. The [`TerminalGuard`] restores the terminal on every
/// exit path, including panics.
pub async fn run(
    runtime: Arc<CoreRuntime>,
    client_id: String,
    thread_id: String,
    mut web_rx: broadcast::Receiver<WebChannelEvent>,
    options: LaunchOptions,
) -> anyhow::Result<()> {
    let mut state = TranscriptState::new(client_id.clone());
    state.set_thread(thread_id.clone());
    let mut ui = UiState::new(thread_id, client_id.clone());
    load_transcript(&runtime, &mut state, &ui.thread_id).await;
    if state.entries().is_empty() {
        state.push_system("OpenHuman is ready. Type /help for the agent cockpit.".to_string());
    }
    super::controls::refresh_config(&runtime, &mut ui).await;
    super::controls::refresh_auth(&runtime, &mut ui).await;
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

    if let Some(prompt) = options.initial_prompt {
        ui.composer.set_text(prompt);
        send_message(&runtime, &client_id, &mut state, &mut ui, "interrupt");
    }

    // Blocking crossterm reader → async channel.
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    let shutdown = Arc::new(AtomicBool::new(false));
    let reader_shutdown = shutdown.clone();
    let reader = std::thread::spawn(move || {
        while !reader_shutdown.load(Ordering::Relaxed) {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(ev) => {
                        if input_tx.send(ev).is_err() {
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
    let mut quit = false;

    while !quit {
        guard.terminal().draw(|f| render::draw(f, &state, &ui))?;

        tokio::select! {
            maybe_ev = input_rx.recv() => match maybe_ev {
                Some(Event::Key(key)) => {
                    if handle_key(key, &runtime, &client_id, &mut state, &mut ui).await {
                        quit = true;
                    } else if ui.identity_changed {
                        ui.identity_changed = false;
                        new_thread(&runtime, &mut state, &mut ui).await;
                    }
                }
                Some(Event::Paste(text)) => handle_paste(&text, &mut ui),
                Some(_) => {} // resize / mouse / paste — redraw next iteration
                None => quit = true, // reader thread gone
            },
            recv = web_rx.recv() => match recv {
                Ok(ev) => handle_web_event(&ev, &mut state, &mut ui),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    log::warn!("[tui] web-channel lagged, dropped {n} events");
                }
                Err(broadcast::error::RecvError::Closed) => {
                    log::warn!("[tui] web-channel closed — exiting");
                    quit = true;
                }
            },
            _ = ticker.tick() => {
                ui.spinner_tick = ui.spinner_tick.wrapping_add(1);
            }
        }
    }

    shutdown.store(true, Ordering::Relaxed);
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

    if matches!(key.code, KeyCode::Char('c')) && ctrl {
        log::info!("[tui] quit via Ctrl+C");
        return true;
    }
    if matches!(key.code, KeyCode::Char('d')) && ctrl {
        log::info!("[tui] quit via Ctrl+D");
        return true;
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
            KeyCode::Esc => cancel_turn(runtime, client_id, &ui.thread_id, state),
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
            KeyCode::Tab if state.is_streaming() => {
                send_message(runtime, client_id, state, ui, "followup")
            }
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

/// Queue a chat turn on the current thread. Fire-and-forget: the reply streams
/// back over the web-channel bus and is folded in by the reducer.
fn send_message(
    runtime: &Arc<CoreRuntime>,
    client_id: &str,
    state: &mut TranscriptState,
    ui: &mut UiState,
    queue_mode: &str,
) {
    let Some(message) = ui.composer.take_for_send() else {
        return;
    };
    ui.scroll_from_bottom = 0;
    state.begin_user_turn(&message);
    log::info!(
        "[tui] send message len={} thread={}",
        message.len(),
        ui.thread_id
    );

    let rt = runtime.clone();
    let cid = client_id.to_string();
    let tid = ui.thread_id.clone();
    let mode = queue_mode.to_string();
    let model_override = ui.model_override.clone();
    let profile_id = ui.profile_id.clone();
    tokio::spawn(async move {
        let params = json!({
            "client_id": cid,
            "thread_id": tid,
            "message": message,
            "source": "type",
            "queue_mode": mode,
            "model_override": model_override,
            "profile_id": profile_id,
        });
        if let Err(e) = rt.invoke("openhuman.channel_web_chat", params).await {
            log::error!("[tui] openhuman.channel_web_chat failed: {e}");
            // Surface the failure in-transcript via a synthetic chat_error so
            // the reducer clears the streaming state and shows the reason.
            web_chat::publish_web_channel_event(WebChannelEvent {
                event: "chat_error".to_string(),
                client_id: cid,
                thread_id: tid,
                message: Some(format!("Failed to send: {e}")),
                error_type: Some("transport".to_string()),
                ..Default::default()
            });
        }
    });
}

/// Cancel the in-flight turn on the current thread. The core emits a
/// `chat_error` ("Cancelled") which the reducer renders.
fn cancel_turn(
    runtime: &Arc<CoreRuntime>,
    client_id: &str,
    thread_id: &str,
    state: &TranscriptState,
) {
    if !state.is_streaming() {
        return;
    }
    log::info!("[tui] cancel turn thread={thread_id}");
    let rt = runtime.clone();
    let cid = client_id.to_string();
    let tid = thread_id.to_string();
    tokio::spawn(async move {
        // Omit `request_id` → stop whatever is running on the thread.
        let params = json!({ "client_id": cid, "thread_id": tid });
        if let Err(e) = rt.invoke("openhuman.channel_web_cancel", params).await {
            log::error!("[tui] openhuman.channel_web_cancel failed: {e}");
        }
    });
}

/// Create a fresh thread and switch the UI to it. Awaited inline (fast, local
/// SQLite write) so `ui.thread_id` can be updated with the result.
async fn new_thread(runtime: &Arc<CoreRuntime>, state: &mut TranscriptState, ui: &mut UiState) {
    log::info!("[tui] creating new thread");
    match runtime
        .invoke("openhuman.threads_create_new", json!({}))
        .await
        .ok()
        .and_then(|v| super::runner::extract_thread_id(&v))
    {
        Some(new_id) => {
            let client_id = state.client_id().to_string();
            *state = TranscriptState::new(client_id);
            ui.thread_id = new_id.clone();
            state.set_thread(new_id.clone());
            ui.scroll_from_bottom = 0;
            state.push_system(format!("Started a new thread · {new_id}"));
            log::info!("[tui] switched to new thread {new_id}");
        }
        None => {
            state.push_system("Could not create a new thread (see logs).".to_string());
            log::error!("[tui] threads.create_new returned no thread id");
        }
    }
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
        let _ = ui.composer.take_for_send();
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

async fn execute_command(
    command_line: &str,
    runtime: &Arc<CoreRuntime>,
    client_id: &str,
    state: &mut TranscriptState,
    ui: &mut UiState,
) -> bool {
    let mut parts = command_line.split_whitespace();
    let command = parts.next().unwrap_or("help");
    let argument = parts.collect::<Vec<_>>().join(" ");
    match command {
        "quit" => return true,
        "new" => new_thread(runtime, state, ui).await,
        "help" => {
            let mut overlay = Overlay::new(OverlayKind::Help, "Agent cockpit help");
            overlay.status = "Type to filter · Esc closes".into();
            overlay.rows = super::composer::COMMANDS
                .iter()
                .map(|(name, description)| OverlayRow {
                    id: (*name).into(),
                    label: format!("/{name}"),
                    detail: (*description).into(),
                    payload: serde_json::Value::Null,
                })
                .collect();
            ui.overlay = Some(overlay);
        }
        "resume" => {
            open_rpc_overlay(
                runtime,
                ui,
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
        "rename" => {
            let mut overlay = Overlay::new(OverlayKind::Rename, "Rename thread");
            overlay.input = Some(argument);
            overlay.status = "Enter saves · Esc cancels".into();
            ui.overlay = Some(overlay);
        }
        "delete" => {
            let mut overlay = Overlay::new(OverlayKind::ConfirmDelete, "Delete thread?");
            overlay.status = "Press y to permanently delete this conversation, or Esc.".into();
            ui.overlay = Some(overlay);
        }
        "model" => {
            let mut overlay = Overlay::new(OverlayKind::Model, "Model override");
            overlay.input = Some(argument);
            overlay.status = "Enter applies a model id to subsequent turns · empty clears".into();
            ui.overlay = Some(overlay);
        }
        "permissions" => {
            let mut overlay = Overlay::new(OverlayKind::Permissions, "Agent access");
            overlay.status = "Choose an access tier · Enter applies".into();
            overlay.rows = [
                (
                    "readonly",
                    "Read-only",
                    "Writes, network, and installs are blocked",
                ),
                ("supervised", "Supervised", "Risky actions ask for approval"),
                (
                    "full",
                    "Full access",
                    "Allowed actions run without prompting",
                ),
            ]
            .into_iter()
            .map(|(id, label, detail)| OverlayRow {
                id: id.into(),
                label: label.into(),
                detail: detail.into(),
                payload: serde_json::Value::Null,
            })
            .collect();
            ui.overlay = Some(overlay);
        }
        "status" => {
            let mut overlay = Overlay::new(OverlayKind::Status, "Session status");
            refresh_agent_paths(runtime, ui).await;
            if let Ok(value) = runtime
                .invoke(
                    "openhuman.channel_web_queue_status",
                    json!({"thread_id": ui.thread_id}),
                )
                .await
            {
                ui.queue_status = serde_json::to_string(super::cockpit::unwrap_rpc(&value))
                    .unwrap_or_else(|_| "unavailable".into());
            }
            overlay.rows = vec![
                text_row("thread", "Thread", &ui.thread_id),
                text_row(
                    "model",
                    "Model",
                    ui.model_override.as_deref().unwrap_or("configured default"),
                ),
                text_row(
                    "profile",
                    "Profile",
                    ui.profile_id.as_deref().unwrap_or("active default"),
                ),
                text_row(
                    "cwd",
                    "Action directory",
                    if ui.action_dir.is_empty() {
                        "unavailable"
                    } else {
                        &ui.action_dir
                    },
                ),
                text_row(
                    "queue",
                    "Run queue",
                    if ui.queue_status.is_empty() {
                        "idle"
                    } else {
                        &ui.queue_status
                    },
                ),
            ];
            ui.overlay = Some(overlay);
        }
        "usage" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Usage,
                "Token and cost usage",
                "openhuman.threads_token_usage",
                json!({"thread_id": ui.thread_id}),
                &[],
                &[],
                &[],
            )
            .await
        }
        "goal" if !argument.trim().is_empty() => {
            match runtime
                .invoke(
                    "openhuman.thread_goals_set",
                    json!({"thread_id": ui.thread_id, "objective": argument.trim()}),
                )
                .await
            {
                Ok(_) => state.push_system("Thread goal updated."),
                Err(error) => state.push_system(format!("Could not update goal: {error}")),
            }
        }
        "goal" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Goal,
                "Thread goal",
                "openhuman.thread_goals_get",
                json!({"thread_id": ui.thread_id}),
                &[],
                &[],
                &[],
            )
            .await;
            if let Some(overlay) = &mut ui.overlay {
                overlay.status =
                    "Use /goal <objective> to create or replace the goal · Esc closes".into();
            }
        }
        "tasks" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Tasks,
                "Task board",
                "openhuman.threads_task_board_get",
                json!({"thread_id": ui.thread_id}),
                &["cards", "items"],
                &["id"],
                &["title", "objective", "id"],
            )
            .await
        }
        "agents" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Agents,
                "Agents",
                "openhuman.profiles_list",
                json!({}),
                &["profiles", "items"],
                &["id", "profile_id"],
                &["name", "display_name", "id"],
            )
            .await
        }
        "skills" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Skills,
                "Skills",
                "openhuman.skills_list",
                json!({"include_skills": true}),
                &["skills", "items"],
                &["id"],
                &["name", "title", "id"],
            )
            .await
        }
        "mcp" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Mcp,
                "MCP servers",
                "openhuman.mcp_clients_installed_list",
                json!({}),
                &["installed", "servers"],
                &["id", "server_id"],
                &["name", "display_name", "id"],
            )
            .await
        }
        "artifacts" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Artifacts,
                "Artifacts",
                "openhuman.ai_list_artifacts",
                json!({"thread_id": ui.thread_id, "limit": 100}),
                &["artifacts", "items"],
                &["id", "artifact_id"],
                &["title", "name", "filename", "id"],
            )
            .await
        }
        "approvals" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Approvals,
                "Pending approvals",
                "openhuman.approval_list_pending",
                json!({}),
                &["pending", "approvals"],
                &["request_id", "id"],
                &["tool_name", "message", "request_id"],
            )
            .await
        }
        "diff" => open_git_diff(ui).await,
        "review" => {
            ui.composer.set_text("Review the current Git working tree. Explain correctness risks, regressions, and missing tests with file and line references.");
            send_message(
                runtime,
                client_id,
                state,
                ui,
                if state.is_streaming() {
                    "followup"
                } else {
                    "interrupt"
                },
            );
        }
        "copy" => copy_latest_answer(state),
        "export" => export_transcript(state, ui, &argument),
        "clear" => state.clear(),
        "logs" => ui.active_tab = AppTab::Logs,
        "config" => ui.active_tab = AppTab::Config,
        "settings" => ui.active_tab = AppTab::Settings,
        "logout" => {
            ui.active_tab = AppTab::Settings;
            ui.logout_confirm = true;
        }
        _ => state.push_system(format!("Unknown command /{command}. Type /help.")),
    }
    false
}

fn text_row(id: &str, label: &str, detail: &str) -> OverlayRow {
    OverlayRow {
        id: id.into(),
        label: label.into(),
        detail: detail.into(),
        payload: serde_json::Value::Null,
    }
}

fn plan_review_overlay(review: &PendingPlanReview) -> Overlay {
    let mut overlay = Overlay::new(OverlayKind::PlanReview, "Plan review");
    overlay.rows = review
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| text_row(&index.to_string(), &format!("{}. {step}", index + 1), ""))
        .collect();
    overlay
        .rows
        .insert(0, text_row("summary", &review.summary, ""));
    overlay.status = "a approve · r reject · e request revision · Esc leaves pending".into();
    overlay
}

fn present_pending_plan_review(ui: &mut UiState) {
    if let Some(review) = ui.pending_plan_review.as_ref() {
        ui.overlay = Some(plan_review_overlay(review));
    }
}

async fn open_rpc_overlay(
    runtime: &Arc<CoreRuntime>,
    ui: &mut UiState,
    kind: OverlayKind,
    title: &str,
    method: &str,
    params: serde_json::Value,
    array_keys: &[&str],
    id_keys: &[&str],
    label_keys: &[&str],
) {
    let mut overlay = Overlay::new(kind, title);
    match runtime.invoke(method, params).await {
        Ok(value) => {
            let items = array_at(&value, array_keys);
            if items.is_empty() {
                let detail = serde_json::to_string_pretty(super::cockpit::unwrap_rpc(&value))
                    .unwrap_or_else(|_| "No data".into());
                overlay.rows.push(text_row("result", "Result", &detail));
            } else {
                overlay.rows = items
                    .iter()
                    .map(|item| row_from_value(item, id_keys, label_keys))
                    .collect();
            }
            overlay.status = format!(
                "{} item(s) · type to filter · Esc closes",
                overlay.rows.len()
            );
        }
        Err(error) => overlay.status = format!("Could not load: {error}"),
    }
    ui.overlay = Some(overlay);
}

fn open_history_search(ui: &mut UiState) {
    let mut overlay = Overlay::new(OverlayKind::HistorySearch, "Composer history");
    overlay.rows = ui
        .composer
        .history_search("")
        .into_iter()
        .enumerate()
        .map(|(index, text)| text_row(&index.to_string(), &text, "Enter restores this prompt"))
        .collect();
    overlay.status = "Type to filter · Enter restores · Esc closes".into();
    ui.overlay = Some(overlay);
}

async fn handle_overlay_key(
    key: KeyEvent,
    runtime: &Arc<CoreRuntime>,
    _client_id: &str,
    state: &mut TranscriptState,
    ui: &mut UiState,
) -> bool {
    let kind = ui.overlay.as_ref().map(|overlay| overlay.kind).unwrap();
    if key.code == KeyCode::Esc {
        ui.overlay = None;
        return false;
    }
    if kind == OverlayKind::ConfirmDelete && matches!(key.code, KeyCode::Char('y' | 'Y')) {
        let result = runtime
            .invoke(
                "openhuman.threads_delete",
                json!({
                    "thread_id": ui.thread_id,
                    "deleted_at": chrono::Utc::now().to_rfc3339(),
                }),
            )
            .await;
        ui.overlay = None;
        match result {
            Ok(_) => new_thread(runtime, state, ui).await,
            Err(error) => state.push_system(format!("Could not delete thread: {error}")),
        }
        return false;
    }
    let typing = ui
        .overlay
        .as_ref()
        .is_some_and(|overlay| overlay.input.is_some());
    if kind == OverlayKind::Approvals {
        let decision = decision_shortcut(kind, typing, key.code);
        if let Some(decision) = decision {
            let request_id = selected_overlay_row(ui)
                .map(|row| row.id)
                .unwrap_or_default();
            decide_approval(runtime, &request_id, decision, state, ui).await;
            return false;
        }
    }
    if kind == OverlayKind::PlanReview && !typing {
        if matches!(key.code, KeyCode::Char('e' | 'E')) {
            if let Some(overlay) = &mut ui.overlay {
                overlay.input = Some(String::new());
                overlay.status = "Describe the needed revision, then press Enter".into();
            }
            return false;
        }
        let decision = decision_shortcut(kind, typing, key.code);
        if let Some(decision) = decision {
            if let Some(review) = ui.pending_plan_review.take() {
                match runtime
                    .invoke(
                        "openhuman.plan_review_decide",
                        json!({"request_id": review.request_id, "decision": decision}),
                    )
                    .await
                {
                    Ok(_) => state.push_system(format!("Plan {decision}d.")),
                    Err(error) => state.push_system(format!("Could not decide plan: {error}")),
                }
            }
            ui.overlay = None;
            return false;
        }
    }

    let Some(overlay) = &mut ui.overlay else {
        return false;
    };
    match key.code {
        KeyCode::Up => overlay.selected = overlay.selected.saturating_sub(1),
        KeyCode::Down => {
            let max = overlay.visible_rows().len().saturating_sub(1);
            overlay.selected = (overlay.selected + 1).min(max);
        }
        KeyCode::Backspace => {
            if let Some(input) = &mut overlay.input {
                input.pop();
            } else {
                overlay.filter.pop();
                overlay.clamp_selection();
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(input) = &mut overlay.input {
                input.push(c);
            } else {
                overlay.filter.push(c);
                overlay.clamp_selection();
            }
        }
        KeyCode::Enter => {
            let input = overlay.input.clone().unwrap_or_default();
            let selected = overlay
                .visible_rows()
                .get(overlay.selected)
                .cloned()
                .cloned();
            match kind {
                OverlayKind::Threads => {
                    if let Some(row) = selected {
                        switch_thread(runtime, state, ui, row.id).await;
                    }
                }
                OverlayKind::HistorySearch => {
                    if let Some(row) = selected {
                        ui.composer.set_text(row.label);
                    }
                    ui.overlay = None;
                }
                OverlayKind::Rename => {
                    if !input.trim().is_empty() {
                        match runtime
                            .invoke(
                                "openhuman.threads_update_title",
                                json!({"thread_id": ui.thread_id, "title": input.trim()}),
                            )
                            .await
                        {
                            Ok(_) => {
                                state.push_system(format!("Thread renamed to {}.", input.trim()))
                            }
                            Err(error) => {
                                state.push_system(format!("Could not rename thread: {error}"))
                            }
                        }
                    }
                    ui.overlay = None;
                }
                OverlayKind::Model => {
                    ui.model_override =
                        (!input.trim().is_empty()).then(|| input.trim().to_string());
                    state.push_system(match &ui.model_override {
                        Some(model) => format!("Using model override {model}."),
                        None => "Using the configured default model.".into(),
                    });
                    ui.overlay = None;
                }
                OverlayKind::Permissions => {
                    if let Some(row) = selected {
                        match runtime
                            .invoke(
                                "openhuman.config_update_autonomy_settings",
                                json!({"level": row.id}),
                            )
                            .await
                        {
                            Ok(_) => {
                                state.push_system(format!("Agent access set to {}.", row.label))
                            }
                            Err(error) => {
                                state.push_system(format!("Could not update access: {error}"))
                            }
                        }
                    }
                    ui.overlay = None;
                }
                OverlayKind::Agents => {
                    if let Some(row) = selected {
                        match runtime
                            .invoke("openhuman.profiles_select", json!({"profile_id": row.id}))
                            .await
                        {
                            Ok(_) => {
                                ui.profile_id = Some(row.id);
                                state.push_system(format!("Agent profile set to {}.", row.label));
                            }
                            Err(error) => state
                                .push_system(format!("Could not select agent profile: {error}")),
                        }
                    }
                    ui.overlay = None;
                }
                OverlayKind::Files => {
                    if let Some(row) = selected {
                        ui.composer.replace_current_token(&format!("@{}", row.id));
                    }
                    ui.overlay = None;
                }
                OverlayKind::PlanReview if !input.trim().is_empty() => {
                    if let Some(review) = ui.pending_plan_review.take() {
                        match runtime
                            .invoke(
                                "openhuman.plan_review_decide",
                                json!({
                                    "request_id": review.request_id,
                                    "decision": "revise",
                                    "feedback": input.trim(),
                                }),
                            )
                            .await
                        {
                            Ok(_) => state.push_system("Plan sent back for revision."),
                            Err(error) => {
                                state.push_system(format!("Could not revise plan: {error}"))
                            }
                        }
                    }
                    ui.overlay = None;
                }
                _ => {}
            }
        }
        _ => {}
    }
    false
}

fn decision_shortcut(kind: OverlayKind, typing: bool, code: KeyCode) -> Option<&'static str> {
    if typing {
        return None;
    }
    match (kind, code) {
        (OverlayKind::Approvals, KeyCode::Char('1')) => Some("approve_once"),
        (OverlayKind::Approvals, KeyCode::Char('2')) => Some("approve_always_for_tool"),
        (OverlayKind::Approvals, KeyCode::Char('3')) => Some("approve_always_for_flow"),
        (OverlayKind::Approvals, KeyCode::Delete) => Some("deny"),
        (OverlayKind::PlanReview, KeyCode::Char('a' | 'A')) => Some("approve"),
        (OverlayKind::PlanReview, KeyCode::Char('r' | 'R')) => Some("reject"),
        _ => None,
    }
}

fn selected_overlay_row(ui: &UiState) -> Option<OverlayRow> {
    let overlay = ui.overlay.as_ref()?;
    overlay
        .visible_rows()
        .get(overlay.selected)
        .cloned()
        .cloned()
}

async fn decide_approval(
    runtime: &Arc<CoreRuntime>,
    request_id: &str,
    decision: &str,
    state: &mut TranscriptState,
    ui: &mut UiState,
) {
    if request_id.is_empty() {
        return;
    }
    match runtime
        .invoke(
            "openhuman.approval_decide",
            json!({"request_id": request_id, "decision": decision}),
        )
        .await
    {
        Ok(_) => {
            ui.pending_approvals
                .retain(|approval| approval.request_id != request_id);
            state.push_system(format!("Approval decision: {decision}."));
            ui.overlay = None;
        }
        Err(error) => state.push_system(format!("Could not decide approval: {error}")),
    }
}

async fn switch_thread(
    runtime: &Arc<CoreRuntime>,
    state: &mut TranscriptState,
    ui: &mut UiState,
    thread_id: String,
) {
    let client_id = state.client_id().to_string();
    *state = TranscriptState::new(client_id);
    state.set_thread(thread_id.clone());
    ui.thread_id = thread_id;
    ui.scroll_from_bottom = 0;
    ui.overlay = None;
    load_transcript(runtime, state, &ui.thread_id).await;
}

async fn load_transcript(runtime: &Arc<CoreRuntime>, state: &mut TranscriptState, thread_id: &str) {
    match runtime
        .invoke(
            "openhuman.threads_transcript_get",
            json!({"thread_id": thread_id, "limit": 500}),
        )
        .await
    {
        Ok(value) => state.load_transcript(&value),
        Err(error) => log::warn!("[tui] could not load transcript thread={thread_id}: {error}"),
    }
}

async fn refresh_agent_paths(runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    if let Ok(value) = runtime
        .invoke("openhuman.config_get_agent_paths", json!({}))
        .await
    {
        let paths = super::cockpit::unwrap_rpc(&value);
        ui.action_dir = paths
            .get("action_dir")
            .or_else(|| paths.get("actionDir"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
}

async fn open_file_picker(ui: &mut UiState) {
    let root = ui.action_dir.clone();
    let query = ui
        .composer
        .file_query()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let result = tokio::task::spawn_blocking(move || collect_files(&root, &query, 500)).await;
    let mut overlay = Overlay::new(OverlayKind::Files, "Workspace files");
    match result {
        Ok(Ok(files)) => {
            overlay.rows = files
                .into_iter()
                .map(|path| text_row(&path, &path, "Enter inserts this path"))
                .collect();
            overlay.status = format!(
                "{} match(es) · type to filter · Esc closes",
                overlay.rows.len()
            );
        }
        Ok(Err(error)) => overlay.status = error,
        Err(error) => overlay.status = error.to_string(),
    }
    ui.overlay = Some(overlay);
}

fn collect_files(root: &str, query: &str, limit: usize) -> Result<Vec<String>, String> {
    const MAX_VISITED: usize = 25_000;
    const MAX_DEPTH: usize = 8;
    if root.is_empty() {
        return Err("Action directory is unavailable.".into());
    }
    let root_path = std::path::Path::new(root);
    let mut pending = vec![(root_path.to_path_buf(), 0usize)];
    let mut files = Vec::new();
    let mut visited = 0usize;
    while let Some((dir, depth)) = pending.pop() {
        if visited >= MAX_VISITED {
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if dir == root_path => {
                return Err(format!("{}: {error}", dir.display()));
            }
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_VISITED {
                break;
            }
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if depth < MAX_DEPTH
                    && !matches!(
                        name.as_ref(),
                        ".git" | "target" | "node_modules" | "worktrees"
                    )
                {
                    pending.push((path, depth + 1));
                }
                continue;
            }
            let relative = path
                .strip_prefix(root_path)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if query.is_empty() || relative.to_ascii_lowercase().contains(query) {
                files.push(relative);
                if files.len() >= limit {
                    files.sort();
                    return Ok(files);
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

fn handle_web_event(ev: &WebChannelEvent, state: &mut TranscriptState, ui: &mut UiState) {
    if ev.client_id != state.client_id() || ev.thread_id != ui.thread_id {
        return;
    }
    match ev.event.as_str() {
        "approval_request" => {
            let approval = PendingApproval {
                request_id: ev.request_id.clone(),
                tool_name: ev.tool_name.clone().unwrap_or_else(|| "tool".into()),
                summary: ev
                    .message
                    .clone()
                    .unwrap_or_else(|| "Approval required".into()),
                args: ev.args.clone().unwrap_or(serde_json::Value::Null),
            };
            ui.pending_approvals
                .retain(|item| item.request_id != approval.request_id);
            ui.pending_approvals.push(approval.clone());
            let preserves_typed_input = ui
                .overlay
                .as_ref()
                .is_some_and(|overlay| overlay.input.is_some());
            if !preserves_typed_input {
                let mut overlay = Overlay::new(OverlayKind::Approvals, "Approval required");
                overlay.rows.push(OverlayRow {
                    id: approval.request_id,
                    label: approval.tool_name,
                    detail: format!("{}\n{}", approval.summary, approval.args),
                    payload: approval.args,
                });
                overlay.status =
                    "1 approve once · 2 always for tool · 3 always for flow · Delete deny".into();
                ui.overlay = Some(overlay);
            }
        }
        "plan_review_request" => {
            let steps = ev
                .args
                .as_ref()
                .and_then(|args| args.get("steps"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|item| {
                            item.as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| item.to_string())
                        })
                        .collect()
                })
                .unwrap_or_default();
            let review = PendingPlanReview {
                request_id: ev.request_id.clone(),
                summary: ev
                    .message
                    .clone()
                    .unwrap_or_else(|| "Review the proposed plan".into()),
                steps,
            };
            let preserves_typed_input = ui
                .overlay
                .as_ref()
                .is_some_and(|overlay| overlay.input.is_some());
            ui.pending_plan_review = Some(review);
            if !preserves_typed_input {
                present_pending_plan_review(ui);
            }
        }
        "task_board_updated" => ui.queue_status = "task board updated".into(),
        _ => state.apply_event(ev),
    }
}

async fn open_git_diff(ui: &mut UiState) {
    let cwd = ui.action_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        if cwd.is_empty() {
            return Err("Action directory is unavailable; run /status first.".to_string());
        }
        let inside = std::process::Command::new("git")
            .args(["-C", &cwd, "rev-parse", "--is-inside-work-tree"])
            .output()
            .map_err(|error| error.to_string())?;
        if !inside.status.success() {
            return Err("The action directory is not a Git repository.".into());
        }
        let status = std::process::Command::new("git")
            .args(["-C", &cwd, "status", "--short", "--branch"])
            .output()
            .map_err(|error| error.to_string())?;
        let output = std::process::Command::new("git")
            .args(["-C", &cwd, "diff", "--no-ext-diff", "--stat", "--patch"])
            .output()
            .map_err(|error| error.to_string())?;
        Ok(format!(
            "{}\n{}",
            String::from_utf8_lossy(&status.stdout),
            String::from_utf8_lossy(&output.stdout)
        ))
    })
    .await
    .unwrap_or_else(|error| Err(error.to_string()));
    let mut overlay = Overlay::new(OverlayKind::Diff, "Working-tree diff");
    match result {
        Ok(diff) if diff.trim().is_empty() => overlay.rows.push(text_row(
            "clean",
            "Working tree is clean",
            "No tracked changes",
        )),
        Ok(diff) => {
            const MAX_DIFF_LINES: usize = 2_000;
            let line_count = diff.lines().count();
            overlay.rows = diff
                .lines()
                .take(MAX_DIFF_LINES)
                .enumerate()
                .map(|(index, line)| text_row(&index.to_string(), line, ""))
                .collect();
            if line_count > MAX_DIFF_LINES {
                overlay.status = format!(
                    "Showing {MAX_DIFF_LINES} of {line_count} lines; remaining lines omitted"
                );
            }
        }
        Err(error) => overlay.status = error,
    }
    ui.overlay = Some(overlay);
}

fn export_transcript(state: &mut TranscriptState, ui: &UiState, argument: &str) {
    let path = if argument.trim().is_empty() {
        let base = if ui.action_dir.is_empty() {
            std::env::current_dir().unwrap_or_default()
        } else {
            std::path::PathBuf::from(&ui.action_dir)
        };
        base.join(format!(
            "openhuman-{}.md",
            ui.thread_id.replace(['/', '\\'], "-")
        ))
    } else {
        let supplied = std::path::PathBuf::from(argument);
        if supplied.is_absolute() || ui.action_dir.is_empty() {
            supplied
        } else {
            std::path::PathBuf::from(&ui.action_dir).join(supplied)
        }
    };
    match std::fs::write(&path, state.export_markdown()) {
        Ok(()) => state.push_system(format!("Transcript exported to {}.", path.display())),
        Err(error) => state.push_system(format!("Could not export transcript: {error}")),
    }
}

fn copy_latest_answer(state: &mut TranscriptState) {
    let Some(answer) = state.last_assistant().map(str::to_string) else {
        state.push_system("There is no completed answer to copy.");
        return;
    };
    // OSC 52 works over local terminals and SSH without taking a platform GUI
    // clipboard dependency. Terminals that disable it safely ignore the code.
    let encoded = base64::engine::general_purpose::STANDARD.encode(answer.as_bytes());
    let result = std::io::stdout()
        .write_all(format!("\x1b]52;c;{encoded}\x07").as_bytes())
        .and_then(|_| std::io::stdout().flush());
    state.push_system(if result.is_ok() {
        "Latest answer copied to the terminal clipboard."
    } else {
        "The terminal clipboard could not be updated."
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
            &WebChannelEvent {
                event: "approval_request".into(),
                client_id: "client".into(),
                thread_id: "thread".into(),
                request_id: "approval-1".into(),
                tool_name: Some("shell".into()),
                ..Default::default()
            },
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
            &WebChannelEvent {
                event: "plan_review_request".into(),
                client_id: "client".into(),
                thread_id: "thread".into(),
                request_id: "review-1".into(),
                message: Some("Review this plan".into()),
                args: Some(json!({"steps": ["Inspect", "Implement"]})),
                ..Default::default()
            },
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
}
