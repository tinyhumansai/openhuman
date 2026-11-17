//! Asynchronous account UI over the shared TinyHumans session owner.
use super::{
    cockpit::{Overlay, OverlayKind},
    session::{BrowserLogin, LoginCancellation, LoginProvider},
    ui_state::{AppTab, UiState},
};
use openhuman_rpc::{embed::CoreRuntime, tinyhumans::SessionState};
use std::{sync::Arc, time::Duration};
use zeroize::Zeroizing;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Login,
    Logout,
    Refresh,
}

/// Messages carry display state only; credentials stay in SessionManager.
#[allow(clippy::large_enum_variant)] // Keep the infrequent login result inline.
pub enum Message {
    BrowserReady {
        generation: u64,
        url: Zeroizing<String>,
        port: u16,
        cancel: LoginCancellation,
    },
    Finished {
        generation: u64,
        operation: Operation,
        result: Result<SessionState, String>,
    },
}

pub fn picker(ui: &mut UiState) {
    if ui.auth_pending {
        ui.active_tab = AppTab::Settings;
        return;
    }
    let mut overlay = Overlay::new(OverlayKind::Login, "Sign in to TinyHumans");
    overlay.rows = [
        ("google", "Google"),
        ("github", "GitHub"),
        ("twitter", "Twitter / X"),
        ("discord", "Discord"),
    ]
    .into_iter()
    .map(|(id, label)| {
        super::presentation::row(
            id,
            format!("Continue with {label}"),
            "Opens your browser; returns securely to this TUI",
        )
    })
    .collect();
    overlay.status = "Select a provider · /login-token is the manual fallback · Esc closes".into();
    ui.overlay = Some(overlay);
    ui.focus = 0;
}

pub fn provider(id: &str) -> Option<LoginProvider> {
    match id {
        "google" => Some(LoginProvider::Google),
        "github" => Some(LoginProvider::Github),
        "twitter" => Some(LoginProvider::Twitter),
        "discord" => Some(LoginProvider::Discord),
        _ => None,
    }
}

/// Open one account form without covering controls for an in-flight session update.
pub fn form(ui: &mut UiState, logout: bool) {
    ui.active_tab = AppTab::Settings;
    if ui.auth_pending {
        return;
    }
    ui.overlay = None;
    ui.logout_confirm = logout;
    ui.login_token = (!logout).then(String::new);
}

fn begin(
    ui: &mut UiState,
    operation: Operation,
) -> Option<(u64, tokio::sync::mpsc::UnboundedSender<Message>)> {
    if ui.auth_pending {
        return None;
    }
    let tx = ui.auth_tx.clone()?;
    ui.auth_generation = ui.auth_generation.wrapping_add(1);
    if operation != Operation::Refresh {
        ui.auth_pending = true;
        ui.auth_browser = false;
        ui.auth_operation = Some(operation);
    }
    Some((ui.auth_generation, tx))
}

pub fn browser(runtime: &Arc<CoreRuntime>, ui: &mut UiState, provider: LoginProvider) {
    let Some((generation, tx)) = begin(ui, Operation::Login) else {
        return;
    };
    ui.auth_browser = true;
    ui.active_tab = AppTab::Settings;
    ui.overlay = None;
    ui.settings_status = "Starting secure browser sign-in…".into();
    let manager = super::session::session_manager(runtime);
    tokio::spawn(async move {
        let result = match BrowserLogin::start(manager, provider).await {
            Ok(login) => {
                let cancel = login.cancellation();
                if tx
                    .send(Message::BrowserReady {
                        generation,
                        url: Zeroizing::new(login.login_url().to_string()),
                        port: login.callback_port(),
                        cancel,
                    })
                    .is_err()
                {
                    return;
                }
                login.wait(Duration::from_secs(300)).await
            }
            Err(error) => Err(error),
        };
        let _ = tx.send(Message::Finished {
            generation,
            operation: Operation::Login,
            result,
        });
    });
}

pub fn token(runtime: &Arc<CoreRuntime>, ui: &mut UiState, token: Zeroizing<String>) {
    let Some((generation, tx)) = begin(ui, Operation::Login) else {
        return;
    };
    ui.settings_status = "Verifying sign-in…".into();
    let manager = super::session::session_manager(runtime);
    tokio::spawn(async move {
        let result = manager
            .login_with_token(token.trim())
            .await
            .map_err(|_| "The login token was not accepted. Please try again.".into());
        let _ = tx.send(Message::Finished {
            generation,
            operation: Operation::Login,
            result,
        });
    });
}

pub fn refresh(runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    let Some((generation, tx)) = begin(ui, Operation::Refresh) else {
        return;
    };
    ui.settings_status = "Refreshing the saved account in the background…".into();
    let manager = super::session::session_manager(runtime);
    tokio::spawn(async move {
        let result = super::session::refresh_session(&manager).await;
        let _ = tx.send(Message::Finished {
            generation,
            operation: Operation::Refresh,
            result,
        });
    });
}

pub fn logout(runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    let Some((generation, tx)) = begin(ui, Operation::Logout) else {
        return;
    };
    ui.logout_confirm = false;
    ui.settings_status = "Signing out…".into();
    let manager = super::session::session_manager(runtime);
    tokio::spawn(async move {
        let result = manager
            .logout()
            .await
            .map_err(|_| "Could not clear the saved session. Please retry.".into());
        let _ = tx.send(Message::Finished {
            generation,
            operation: Operation::Logout,
            result,
        });
    });
}

pub fn cancel(ui: &mut UiState) {
    if let Some(cancel) = &ui.login_cancel {
        cancel.cancel();
        ui.settings_status =
            "Cancellation requested. An already accepted callback finishes its session update."
                .into();
    } else if ui.auth_pending
        && ui.auth_browser
        && ui.auth_operation == Some(Operation::Login)
        && ui.login_url.is_none()
    {
        ui.auth_generation = ui.auth_generation.wrapping_add(1);
        ui.auth_pending = false;
        ui.auth_operation = None;
        ui.settings_status = "Sign-in cancelled.".into();
    } else if ui.auth_pending {
        ui.settings_status = "Session update already started; waiting for it to finish.".into();
    }
}

pub fn apply_session(state: &SessionState, ui: &mut UiState) {
    let changed = ui.auth_user_id != state.core.user_id
        || ui.auth_profile_id != state.core.profile_id
        || ui.authenticated != state.core.is_authenticated;
    ui.auth_user_id = state.core.user_id.clone();
    ui.auth_profile_id = state.core.profile_id.clone();
    ui.authenticated = state.core.is_authenticated;
    ui.identity_changed |= changed;
    ui.auth_summary = if state.core.is_authenticated {
        match state.core.credential.as_deref() {
            Some("api-key") => "API key configured",
            Some("local") => "Local session",
            _ => {
                if pending_validation(state) {
                    "Session saved · validation pending"
                } else if state.current_user_stale {
                    "Signed in · cached profile"
                } else {
                    "Signed in"
                }
            }
        }
    } else {
        "Signed out"
    }
    .into();
    let user = state
        .core
        .is_authenticated
        .then(|| state.current_user.as_ref().or(state.core.user.as_ref()))
        .flatten();
    ui.account_detail = user
        .map(super::controls::account_detail)
        .unwrap_or_default();
}

pub fn apply(message: Message, ui: &mut UiState) {
    match message {
        Message::BrowserReady {
            generation,
            url,
            port,
            cancel,
        } => {
            if generation != ui.auth_generation || !ui.auth_pending {
                cancel.cancel();
                return;
            }
            ui.login_url = Some(url);
            ui.login_port = Some(port);
            ui.login_cancel = Some(cancel);
            ui.settings_status=format!("Waiting for your browser (up to 5 minutes).\nCallback port: {port} on this machine. Over SSH, forward this port before opening the copied link.\nO opens the browser · C copies the link · Esc cancels.");
            reopen(ui);
        }
        Message::Finished {
            generation,
            operation,
            result,
        } => {
            if generation != ui.auth_generation {
                return;
            }
            if operation != Operation::Refresh {
                ui.auth_pending = false;
                ui.auth_operation = None;
                ui.login_url = None;
                ui.login_port = None;
                ui.login_cancel = None;
            }
            match result {
                Ok(state) => {
                    apply_session(&state, ui);
                    ui.settings_status = if operation == Operation::Logout {
                        "Signed out. Your session was cleared.".into()
                    } else if pending_validation(&state) {
                        "Session saved. Backend validation will retry in the background.".into()
                    } else if state.current_user_stale {
                        "Using the saved account while the backend is unreachable.".into()
                    } else if state.core.is_authenticated {
                        "Account ready.".into()
                    } else {
                        "Sign in with a browser or use your configured local/provider access."
                            .into()
                    };
                }
                Err(error) => ui.settings_status = error,
            }
        }
    }
}

fn pending_validation(state: &SessionState) -> bool {
    state
        .core
        .user
        .as_ref()
        .and_then(|user| user.get("pendingBackendValidation"))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

pub fn reopen(ui: &mut UiState) {
    if let Some(url) = &ui.login_url {
        if let Err(error) = super::session::open_browser(url) {
            ui.settings_status = format!(
                "{error}\nUse Copy link; if remote, forward callback port {}.",
                ui.login_port.unwrap_or(0)
            );
        }
    }
}

#[cfg(test)]
#[path = "account_tests.rs"]
mod tests;
