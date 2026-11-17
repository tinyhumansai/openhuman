//! Pure navigation and form state for the four terminal pages.

use zeroize::Zeroize;

use super::cockpit::{Overlay, PendingApproval, PendingPlanReview};
use super::composer::Composer;

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
    LoginToken,
    Logout,
}

impl SettingsAction {
    pub const ALL: [Self; 4] = [
        Self::ViewAccount,
        Self::Login,
        Self::LoginToken,
        Self::Logout,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ViewAccount => "View account",
            Self::Login => "Sign in with your browser",
            Self::LoginToken => "Paste a one-time login token",
            Self::Logout => "Log out",
        }
    }
}

/// UI-only state owned by the event loop and read by the renderer.
pub struct UiState {
    pub active_tab: AppTab,
    pub composer: Composer,
    pub scroll_from_bottom: usize,
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
    pub overlay: Option<Overlay>,
    pub pending_approvals: Vec<PendingApproval>,
    pub pending_plan_review: Option<PendingPlanReview>,
    pub model_override: Option<String>,
    pub action_dir: String,
    pub queue_status: String,
    pub theme: super::theme::Theme,
    pub hits: Vec<super::actions::Hit>,
    pub viewport: super::viewport::ViewportCache,
    pub transcript_area: ratatui::layout::Rect,
    pub composer_area: ratatui::layout::Rect,
    pub composer_first_row: usize,
    pub focus: usize,
    pub agent_name: String,
    pub mouse_enabled: bool,
    pub stopping: bool,
    pub demo: bool,
    pub drafts: std::collections::HashMap<String, String>,
    pub provider_id: String,
    pub effective_model: String,
    pub overlay_generation: u64,
    pub overlay_tx: Option<tokio::sync::mpsc::UnboundedSender<super::effects::OverlayReply>>,
    pub auth_tx: Option<tokio::sync::mpsc::UnboundedSender<super::account::Message>>,
    pub auth_pending: bool,
    pub auth_browser: bool,
    pub policy_enabled: bool,
    pub overlay_area: ratatui::layout::Rect,
    pub suggestion_selected: usize,
    pub auth_generation: u64,
    pub auth_user_id: Option<String>,
    pub auth_profile_id: Option<String>,
    pub authenticated: bool,
    pub auth_operation: Option<super::account::Operation>,
    pub login_url: Option<zeroize::Zeroizing<String>>,
    pub login_port: Option<u16>,
    pub login_cancel: Option<super::session::LoginCancellation>,
}

impl UiState {
    pub fn new(thread_id: String, _client_id: String) -> Self {
        Self {
            active_tab: AppTab::Chat,
            composer: Composer::default(),
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
            overlay: None,
            pending_approvals: Vec::new(),
            pending_plan_review: None,
            model_override: None,
            action_dir: String::new(),
            queue_status: String::new(),
            theme: if std::env::var_os("NO_COLOR").is_some() {
                super::theme::Theme::Mono
            } else {
                super::theme::Theme::System
            },
            hits: Vec::new(),
            viewport: Default::default(),
            transcript_area: Default::default(),
            composer_area: Default::default(),
            composer_first_row: 0,
            focus: 0,
            agent_name: "orchestrator".into(),
            mouse_enabled: true,
            stopping: false,
            demo: false,
            drafts: Default::default(),
            provider_id: "openhuman".into(),
            effective_model: "Model".into(),
            overlay_generation: 0,
            overlay_tx: None,
            auth_tx: None,
            auth_pending: false,
            auth_browser: false,
            policy_enabled: false,
            overlay_area: Default::default(),
            suggestion_selected: 0,
            auth_generation: 0,
            auth_user_id: None,
            auth_profile_id: None,
            authenticated: false,
            auth_operation: None,
            login_url: None,
            login_port: None,
            login_cancel: None,
        }
    }

    pub fn is_editing(&self) -> bool {
        self.overlay.is_some()
            || self.config_edit.is_some()
            || self.login_token.is_some()
            || self.logout_confirm
    }
}

impl Drop for UiState {
    fn drop(&mut self) {
        if let Some(cancel) = &self.login_cancel {
            cancel.cancel();
        }
        if let Some(token) = &mut self.login_token {
            token.zeroize();
        }
    }
}

#[cfg(test)]
#[path = "ui_state_tests.rs"]
mod tests;
