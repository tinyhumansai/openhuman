//! Pure navigation and form state for the four terminal pages.

use zeroize::Zeroize;

/// Top-level terminal pages. The order is part of the CLI UX contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTab {
    Logs,
    Chat,
    Config,
    Settings,
}

impl AppTab {
    pub const ALL: [Self; 4] = [Self::Logs, Self::Chat, Self::Config, Self::Settings];

    pub fn title(self) -> &'static str {
        match self {
            Self::Logs => "Logs",
            Self::Chat => "Chat",
            Self::Config => "Config",
            Self::Settings => "Settings",
        }
    }

    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        Self::ALL[(self as usize + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKey {
    ApiUrl,
    InferenceUrl,
    DefaultModel,
    AutonomyLevel,
    PrivacyMode,
}

#[derive(Debug, Clone)]
pub struct ConfigItem {
    pub key: ConfigKey,
    pub label: &'static str,
    pub value: String,
    pub hint: &'static str,
}

impl ConfigItem {
    fn new(key: ConfigKey, label: &'static str, hint: &'static str) -> Self {
        Self {
            key,
            label,
            value: String::new(),
            hint,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    ViewAccount,
    Login,
    Logout,
}

impl SettingsAction {
    pub const ALL: [Self; 3] = [Self::ViewAccount, Self::Login, Self::Logout];

    pub fn label(self) -> &'static str {
        match self {
            Self::ViewAccount => "View account",
            Self::Login => "Log in with one-time token",
            Self::Logout => "Log out",
        }
    }
}

/// UI-only state owned by the event loop and read by the renderer.
pub struct UiState {
    pub active_tab: AppTab,
    pub input: String,
    pub scroll_from_bottom: u16,
    pub spinner_tick: usize,
    pub thread_id: String,
    pub log_scroll_from_bottom: u16,
    pub config_items: Vec<ConfigItem>,
    pub config_selected: usize,
    pub config_edit: Option<String>,
    pub config_status: String,
    pub settings_selected: usize,
    pub auth_summary: String,
    pub account_detail: String,
    pub login_token: Option<String>,
    pub logout_confirm: bool,
    pub settings_status: String,
    pub identity_changed: bool,
}

impl UiState {
    pub fn new(thread_id: String, _client_id: String) -> Self {
        Self {
            active_tab: AppTab::Logs,
            input: String::new(),
            scroll_from_bottom: 0,
            spinner_tick: 0,
            thread_id,
            log_scroll_from_bottom: 0,
            config_items: vec![
                ConfigItem::new(
                    ConfigKey::ApiUrl,
                    "Backend URL",
                    "OpenHuman auth and billing backend",
                ),
                ConfigItem::new(
                    ConfigKey::InferenceUrl,
                    "Inference URL",
                    "Custom OpenAI-compatible endpoint",
                ),
                ConfigItem::new(
                    ConfigKey::DefaultModel,
                    "Default model",
                    "Model id used when no route overrides it",
                ),
                ConfigItem::new(
                    ConfigKey::AutonomyLevel,
                    "Agent access",
                    "readonly, supervised, or full",
                ),
                ConfigItem::new(
                    ConfigKey::PrivacyMode,
                    "Privacy mode",
                    "local_only, standard, or sensitive",
                ),
            ],
            config_selected: 0,
            config_edit: None,
            config_status: "Loading safe configuration…".to_string(),
            settings_selected: 0,
            auth_summary: "Checking account…".to_string(),
            account_detail: String::new(),
            login_token: None,
            logout_confirm: false,
            settings_status: "Select an account action and press Enter.".to_string(),
            identity_changed: false,
        }
    }

    pub fn is_editing(&self) -> bool {
        self.config_edit.is_some() || self.login_token.is_some() || self.logout_confirm
    }
}

impl Drop for UiState {
    fn drop(&mut self) {
        if let Some(token) = &mut self.login_token {
            token.zeroize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_starts_on_logs_and_tabs_wrap_in_product_order() {
        let ui = UiState::new("thread".into(), "client".into());
        assert_eq!(ui.active_tab, AppTab::Logs);
        assert_eq!(AppTab::Logs.next(), AppTab::Chat);
        assert_eq!(AppTab::Chat.next(), AppTab::Config);
        assert_eq!(AppTab::Config.next(), AppTab::Settings);
        assert_eq!(AppTab::Settings.next(), AppTab::Logs);
        assert_eq!(AppTab::Logs.previous(), AppTab::Settings);
    }
}
