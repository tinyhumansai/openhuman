//! Config and account actions for the tabbed terminal UI.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use zeroize::Zeroize;

use openhuman_rpc::embed::CoreRuntime;

use super::ui_state::{ConfigKey, SettingsAction, UiState};

pub async fn handle_config_key(key: KeyEvent, runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    if let Some(input) = ui.config_edit.as_mut() {
        match key.code {
            KeyCode::Esc => ui.config_edit = None,
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Enter => save_config(runtime, ui).await,
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => input.push(c),
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Up => ui.config_selected = ui.config_selected.saturating_sub(1),
        KeyCode::Down => {
            ui.config_selected = (ui.config_selected + 1).min(ui.config_items.len() - 1)
        }
        KeyCode::Enter => {
            ui.config_edit = Some(ui.config_items[ui.config_selected].value.clone());
        }
        _ => {}
    }
}

async fn save_config(runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    let Some(value) = ui.config_edit.take() else {
        return;
    };
    let key = ui.config_items[ui.config_selected].key;
    let (method, params) = config_update(key, value);
    ui.config_status = "Saving…".to_string();
    match runtime.invoke(method, params).await {
        Ok(_) => {
            ui.config_status = "Saved.".to_string();
            refresh_config(runtime, ui).await;
        }
        Err(err) => ui.config_status = format!("Save failed: {err}"),
    }
}

fn config_update(key: ConfigKey, value: String) -> (&'static str, serde_json::Value) {
    match key {
        ConfigKey::ApiUrl => (
            "openhuman.config_update_model_settings",
            json!({"api_url": value}),
        ),
        ConfigKey::InferenceUrl => (
            "openhuman.config_update_model_settings",
            json!({"inference_url": value}),
        ),
        ConfigKey::DefaultModel => (
            "openhuman.config_update_model_settings",
            json!({"default_model": value}),
        ),
        ConfigKey::AutonomyLevel => (
            "openhuman.config_update_autonomy_settings",
            json!({"level": value}),
        ),
        ConfigKey::PrivacyMode => ("openhuman.config_set_privacy_mode", json!({"mode": value})),
    }
}

pub async fn refresh_config(runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    let client = runtime
        .invoke("openhuman.config_get_client_config", json!({}))
        .await;
    let autonomy = runtime
        .invoke("openhuman.config_get_autonomy_settings", json!({}))
        .await;
    let privacy = runtime
        .invoke("openhuman.config_get_privacy_mode", json!({}))
        .await;
    if let Ok(snapshot) = runtime.invoke("openhuman.config_get", json!({})).await {
        ui.agent_name = rpc_payload(&snapshot)
            .pointer("/config/agent/chat_agent_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("orchestrator")
            .to_string();
    }
    match (client, autonomy, privacy) {
        (Ok(client), Ok(autonomy), Ok(privacy)) => {
            let client = rpc_payload(&client);
            ui.provider_id = catalog_provider(client);
            ui.effective_model = client
                .get("default_model")
                .and_then(serde_json::Value::as_str)
                .filter(|model| !model.is_empty())
                .unwrap_or("Configured model")
                .to_string();
            let autonomy = rpc_payload(&autonomy);
            ui.policy_enabled = autonomy
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let privacy = rpc_payload(&privacy);
            for item in &mut ui.config_items {
                item.value = match item.key {
                    ConfigKey::ApiUrl => string_at(client, &["api_url"]),
                    ConfigKey::InferenceUrl => string_at(client, &["inference_url"]),
                    ConfigKey::DefaultModel => string_at(client, &["default_model"]),
                    ConfigKey::AutonomyLevel => string_at(autonomy, &["level"]),
                    ConfigKey::PrivacyMode => string_at(privacy, &["mode"]),
                };
            }
            ui.config_status = "Select a field and press Enter to edit.".to_string();
        }
        _ => ui.config_status = "Could not load one or more config sections; see Logs.".to_string(),
    }
}

pub async fn handle_settings_key(key: KeyEvent, runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    if ui.auth_pending {
        match key.code {
            KeyCode::Esc => super::account::cancel(ui),
            KeyCode::Char('o' | 'O') => super::account::reopen(ui),
            KeyCode::Char('c' | 'C') => copy_login_link(ui),
            _ => {}
        }
        return;
    }
    if let Some(token) = ui.login_token.as_mut() {
        match key.code {
            KeyCode::Esc => {
                token.zeroize();
                ui.login_token = None;
            }
            KeyCode::Backspace => {
                token.pop();
            }
            KeyCode::Enter => login_with_token(runtime, ui).await,
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => token.push(c),
            _ => {}
        }
        return;
    }
    if ui.logout_confirm {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') => ui.logout_confirm = false,
            KeyCode::Char('y') | KeyCode::Enter => super::account::logout(runtime, ui),
            _ => {}
        }
        return;
    }
    match key.code {
        KeyCode::Up => ui.settings_selected = ui.settings_selected.saturating_sub(1),
        KeyCode::Down => {
            ui.settings_selected = (ui.settings_selected + 1).min(SettingsAction::ALL.len() - 1)
        }
        KeyCode::Enter => match SettingsAction::ALL[ui.settings_selected] {
            SettingsAction::ViewAccount => super::account::refresh(runtime, ui),
            SettingsAction::Login => super::account::picker(ui),
            SettingsAction::LoginToken => ui.login_token = Some(String::new()),
            SettingsAction::Logout => ui.logout_confirm = true,
        },
        _ => {}
    }
}

pub async fn refresh_auth(runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    match super::session::session_manager(runtime).core_state().await {
        Ok(core) => {
            let stale = core.credential.as_deref() == Some("session");
            let state = openhuman_rpc::tinyhumans::SessionState {
                current_user: core.user.clone(),
                current_user_stale: stale,
                core,
                ..Default::default()
            };
            super::account::apply_session(&state, ui);
        }
        Err(_) => ui.auth_summary = "Account status unavailable".into(),
    }
}

async fn login_with_token(runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    let token = zeroize::Zeroizing::new(ui.login_token.take().unwrap_or_default());
    if token.trim().is_empty() {
        ui.settings_status = "Login token cannot be empty.".into();
        ui.login_token = Some(String::new());
        return;
    }
    super::account::token(runtime, ui, token);
}

pub(crate) fn copy_login_link(ui: &mut UiState) {
    use std::io::Write;
    if let Some(url) = &ui.login_url {
        match std::io::stdout().write_all(login_clipboard_sequence(url).as_bytes()).and_then(|_|std::io::stdout().flush()) {
            Ok(())=>ui.settings_status=format!("Terminal clipboard copy requested. Forward port {} when remote. O reopens · Esc cancels.",ui.login_port.unwrap_or(0)),
            Err(_)=>ui.settings_status="The terminal clipboard is unavailable. Use O to open the browser.".into()
        }
    }
}

fn login_clipboard_sequence(url: &str) -> String {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(url.as_bytes());
    format!("\u{1b}]52;c;{encoded}\u{7}")
}

fn rpc_payload(value: &serde_json::Value) -> &serde_json::Value {
    openhuman_rpc::unwrap_rpc(value)
}

fn catalog_provider(client: &serde_json::Value) -> String {
    let route = client
        .get("chat_provider")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("cloud")
        .trim();
    if let Some(id) = route.strip_prefix("pid:") {
        return id.split(':').next().unwrap_or("openhuman").to_string();
    }
    let prefix = route.split(':').next().unwrap_or("cloud");
    if matches!(prefix, "cloud" | "primary" | "") {
        client
            .get("primary_cloud")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .unwrap_or("openhuman")
            .to_string()
    } else {
        prefix.to_string()
    }
}

fn string_at(value: &serde_json::Value, path: &[&str]) -> String {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn account_detail(user: &serde_json::Value) -> String {
    let user = user.get("data").unwrap_or(user);
    let user = user.get("user").unwrap_or(user);
    let name = [
        string_at(user, &["firstName"]),
        string_at(user, &["lastName"]),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    let identity = [string_at(user, &["email"]), string_at(user, &["username"])]
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap_or_default();
    [name, identity]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
#[path = "controls_tests.rs"]
mod tests;
