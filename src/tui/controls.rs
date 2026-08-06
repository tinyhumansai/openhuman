//! Config and account actions for the tabbed terminal UI.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use zeroize::Zeroize;

use crate::core::runtime::CoreRuntime;

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
    match (client, autonomy, privacy) {
        (Ok(client), Ok(autonomy), Ok(privacy)) => {
            let client = rpc_payload(&client);
            let autonomy = rpc_payload(&autonomy);
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
            KeyCode::Char('y') | KeyCode::Enter => logout(runtime, ui).await,
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
            SettingsAction::ViewAccount => view_account(runtime, ui).await,
            SettingsAction::Login => ui.login_token = Some(String::new()),
            SettingsAction::Logout => ui.logout_confirm = true,
        },
        _ => {}
    }
}

pub async fn refresh_auth(runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    match runtime.invoke("openhuman.auth_get_state", json!({})).await {
        Ok(value) => {
            let state = rpc_payload(&value);
            if state
                .get("isAuthenticated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                let identity = string_at(state, &["userId"]);
                ui.auth_summary = if identity.is_empty() {
                    "Signed in".to_string()
                } else {
                    format!("Signed in · {identity}")
                };
            } else {
                ui.auth_summary = "Signed out".to_string();
                ui.account_detail.clear();
            }
        }
        Err(err) => ui.auth_summary = format!("Account status unavailable: {err}"),
    }
}

async fn view_account(runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    ui.settings_status = "Refreshing account…".to_string();
    match runtime.invoke("openhuman.auth_get_me", json!({})).await {
        Ok(value) => {
            let user = rpc_payload(&value);
            ui.account_detail = account_detail(user);
            ui.settings_status = "Account refreshed.".to_string();
            refresh_auth(runtime, ui).await;
        }
        Err(err) => ui.settings_status = format!("Account refresh failed: {err}"),
    }
}

async fn login_with_token(runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    let mut token = ui.login_token.take().unwrap_or_default();
    if token.trim().is_empty() {
        ui.settings_status = "Login token cannot be empty.".to_string();
        token.zeroize();
        return;
    }
    ui.settings_status = "Signing in…".to_string();
    let consumed = runtime
        .invoke(
            "openhuman.auth_consume_login_token",
            json!({"loginToken": token.trim()}),
        )
        .await;
    token.zeroize();
    let result = match consumed {
        Ok(value) => value,
        Err(err) => {
            ui.settings_status = format!("Login failed: {err}");
            return;
        }
    };
    let mut jwt = string_at(rpc_payload(&result), &["jwtToken"]);
    if jwt.is_empty() {
        ui.settings_status = "Login failed: backend returned no session token.".to_string();
        return;
    }
    let stored = runtime
        .invoke("openhuman.auth_store_session", json!({"token": jwt}))
        .await;
    jwt.zeroize();
    match stored {
        Ok(_) => {
            ui.settings_status = "Signed in.".to_string();
            refresh_auth(runtime, ui).await;
            ui.identity_changed = true;
        }
        Err(err) => ui.settings_status = format!("Could not store session: {err}"),
    }
}

async fn logout(runtime: &Arc<CoreRuntime>, ui: &mut UiState) {
    ui.logout_confirm = false;
    match runtime
        .invoke("openhuman.auth_clear_session", json!({}))
        .await
    {
        Ok(_) => {
            ui.settings_status = "Signed out.".to_string();
            refresh_auth(runtime, ui).await;
            ui.identity_changed = true;
        }
        Err(err) => ui.settings_status = format!("Logout failed: {err}"),
    }
}

fn rpc_payload(value: &serde_json::Value) -> &serde_json::Value {
    value
        .get("result")
        .or_else(|| value.get("data"))
        .map(rpc_payload)
        .unwrap_or(value)
}

fn string_at(value: &serde_json::Value, path: &[&str]) -> String {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn account_detail(user: &serde_json::Value) -> String {
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
mod tests {
    use super::*;

    #[test]
    fn rpc_payload_unwraps_runtime_and_api_envelopes() {
        let value = json!({"result": {"data": {"mode": "standard"}}, "logs": []});
        assert_eq!(rpc_payload(&value), &json!({"mode": "standard"}));
        assert_eq!(string_at(rpc_payload(&value), &["mode"]), "standard");
    }

    #[test]
    fn string_at_never_coerces_non_string_or_missing_values() {
        let value = json!({"enabled": true});
        assert_eq!(string_at(&value, &["enabled"]), "");
        assert_eq!(string_at(&value, &["missing"]), "");
    }

    #[test]
    fn account_detail_uses_canonical_backend_user_fields() {
        let user = json!({
            "firstName": "Ada",
            "lastName": "Lovelace",
            "email": "ada@example.test",
            "username": "ada"
        });
        assert_eq!(account_detail(&user), "Ada Lovelace · ada@example.test");
    }

    #[test]
    fn curated_config_fields_map_to_safe_specific_updates() {
        let cases = [
            (
                ConfigKey::ApiUrl,
                "openhuman.config_update_model_settings",
                json!({"api_url": "value"}),
            ),
            (
                ConfigKey::InferenceUrl,
                "openhuman.config_update_model_settings",
                json!({"inference_url": "value"}),
            ),
            (
                ConfigKey::DefaultModel,
                "openhuman.config_update_model_settings",
                json!({"default_model": "value"}),
            ),
            (
                ConfigKey::AutonomyLevel,
                "openhuman.config_update_autonomy_settings",
                json!({"level": "value"}),
            ),
            (
                ConfigKey::PrivacyMode,
                "openhuman.config_set_privacy_mode",
                json!({"mode": "value"}),
            ),
        ];
        for (key, expected_method, expected_params) in cases {
            let (method, params) = config_update(key, "value".to_string());
            assert_eq!(method, expected_method);
            assert_eq!(params, expected_params);
        }
    }
}
