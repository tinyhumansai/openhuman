//! Telegram channel — constructor, configuration, auth/pairing, and API plumbing helpers.

use super::channel_types::{
    TelegramChannel, TelegramUpdateWindow, TELEGRAM_RECENT_UPDATE_CACHE_SIZE,
};
use super::text::TELEGRAM_BIND_COMMAND;
use crate::openhuman::config::{Config, StreamMode};
use crate::openhuman::security::pairing::PairingGuard;
use anyhow::Context;
use directories::UserDirs;
use std::sync::{Arc, RwLock};
use tokio::fs;

impl TelegramChannel {
    pub fn new(bot_token: String, allowed_users: Vec<String>, mention_only: bool) -> Self {
        let normalized_allowed = Self::normalize_allowed_users(allowed_users);
        let pairing = if normalized_allowed.is_empty() {
            let guard = PairingGuard::new(true, &[]);
            if let Some(code) = guard.pairing_code() {
                println!("  🔐 Telegram pairing required. One-time bind code: {code}");
                println!("     Send `{TELEGRAM_BIND_COMMAND} <code>` from your Telegram account.");
            }
            Some(guard)
        } else {
            None
        };

        Self {
            bot_token,
            allowed_users: Arc::new(RwLock::new(normalized_allowed)),
            pairing,
            client: reqwest::Client::new(),
            stream_mode: StreamMode::Off,
            draft_update_interval_ms: 1000,
            silent_streaming: true,
            last_draft_edit: parking_lot::Mutex::new(std::collections::HashMap::new()),
            typing_handle: parking_lot::Mutex::new(None),
            mention_only,
            bot_username: parking_lot::Mutex::new(None),
            recent_updates: parking_lot::Mutex::new(TelegramUpdateWindow::default()),
        }
    }

    /// Configure streaming mode for progressive draft updates.
    /// Configure streaming mode for progressive draft updates.
    pub fn with_streaming(
        mut self,
        stream_mode: StreamMode,
        draft_update_interval_ms: u64,
        silent_streaming: bool,
    ) -> Self {
        self.stream_mode = stream_mode;
        self.draft_update_interval_ms = draft_update_interval_ms;
        self.silent_streaming = silent_streaming;
        self
    }

    /// Parse reply_target into (chat_id, optional thread_id).
    pub(crate) fn parse_reply_target(reply_target: &str) -> (String, Option<String>) {
        if let Some((chat_id, thread_id)) = reply_target.split_once(':') {
            (chat_id.to_string(), Some(thread_id.to_string()))
        } else {
            (reply_target.to_string(), None)
        }
    }

    pub(crate) fn parse_message_id(value: Option<&str>) -> Option<i64> {
        value.and_then(|raw| raw.trim().parse::<i64>().ok())
    }

    pub(crate) fn http_client(&self) -> reqwest::Client {
        crate::openhuman::config::build_runtime_proxy_client("channel.telegram")
    }

    pub(crate) fn normalize_identity(value: &str) -> String {
        value.trim().trim_start_matches('@').to_string()
    }

    pub(crate) fn normalize_allowed_users(allowed_users: Vec<String>) -> Vec<String> {
        allowed_users
            .into_iter()
            .map(|entry| Self::normalize_identity(&entry))
            .filter(|entry| !entry.is_empty())
            .collect()
    }

    pub(crate) fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{method}", self.bot_token)
    }

    pub(crate) fn pairing_code_active(&self) -> bool {
        self.pairing
            .as_ref()
            .and_then(PairingGuard::pairing_code)
            .is_some()
    }

    pub(crate) fn extract_bind_code(text: &str) -> Option<&str> {
        let mut parts = text.split_whitespace();
        let command = parts.next()?;
        let base_command = command.split('@').next().unwrap_or(command);
        if base_command != TELEGRAM_BIND_COMMAND {
            return None;
        }
        parts.next().map(str::trim).filter(|code| !code.is_empty())
    }

    pub(crate) fn track_update_id(&self, update_id: i64) -> bool {
        let mut window = self.recent_updates.lock();
        if window.recent_lookup.contains(&update_id) {
            tracing::debug!(
                update_id,
                "Telegram update dedupe hit: duplicate update skipped"
            );
            return false;
        }

        if update_id < window.max_seen_update_id {
            tracing::debug!(
                update_id,
                max_seen = window.max_seen_update_id,
                "Telegram update ordering safeguard: stale update skipped"
            );
            return false;
        }

        if update_id > window.max_seen_update_id {
            window.max_seen_update_id = update_id;
        }

        window.recent_lookup.insert(update_id);
        window.recent_order.push_back(update_id);
        if window.recent_order.len() > TELEGRAM_RECENT_UPDATE_CACHE_SIZE {
            if let Some(evicted) = window.recent_order.pop_front() {
                window.recent_lookup.remove(&evicted);
            }
        }
        true
    }

    /// Clears Bot API webhook mode so `getUpdates` long polling can run.
    pub(crate) async fn delete_webhook_for_long_polling(&self) -> bool {
        let url = self.api_url("deleteWebhook");
        let body = serde_json::json!({ "drop_pending_updates": false });
        tracing::info!(
            "[telegram] deleteWebhook: enabling getUpdates polling (drop_pending_updates=false)"
        );
        match self.http_client().post(&url).json(&body).send().await {
            Ok(resp) => Self::telegram_api_ok(resp).await,
            Err(e) => {
                tracing::warn!(error = %e, "[telegram] deleteWebhook HTTP request failed");
                false
            }
        }
    }

    pub(crate) async fn telegram_api_ok(resp: reqwest::Response) -> bool {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            tracing::warn!(status = ?status, body, "Telegram API request failed");
            return false;
        }

        match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(payload) => {
                if payload
                    .get("ok")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    true
                } else {
                    let error_code = payload
                        .get("error_code")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or_default();
                    let description = payload
                        .get("description")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown Telegram API error");
                    tracing::warn!(
                        status = ?status,
                        error_code,
                        description,
                        body,
                        "Telegram API responded with ok=false"
                    );
                    false
                }
            }
            Err(error) => {
                tracing::warn!(
                    status = ?status,
                    %error,
                    body,
                    "Telegram API returned non-JSON body"
                );
                false
            }
        }
    }

    pub(crate) async fn fetch_bot_username(&self) -> anyhow::Result<String> {
        let resp = self.http_client().get(self.api_url("getMe")).send().await?;

        if !resp.status().is_success() {
            anyhow::bail!("Failed to fetch bot info: {}", resp.status());
        }

        let data: serde_json::Value = resp.json().await?;
        let username = data
            .get("result")
            .and_then(|r| r.get("username"))
            .and_then(|u| u.as_str())
            .context("Bot username not found in response")?;

        Ok(username.to_string())
    }

    pub(crate) async fn get_bot_username(&self) -> Option<String> {
        {
            let cache = self.bot_username.lock();
            if let Some(ref username) = *cache {
                return Some(username.clone());
            }
        }

        match self.fetch_bot_username().await {
            Ok(username) => {
                let mut cache = self.bot_username.lock();
                *cache = Some(username.clone());
                Some(username)
            }
            Err(e) => {
                tracing::warn!("Failed to fetch bot username: {e}");
                None
            }
        }
    }

    async fn load_config_without_env() -> anyhow::Result<Config> {
        let home = UserDirs::new()
            .map(|u| u.home_dir().to_path_buf())
            .context("Could not find home directory")?;
        let openhuman_dir = home.join(".openhuman");
        let config_path = openhuman_dir.join("config.toml");

        let contents = fs::read_to_string(&config_path)
            .await
            .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
        let mut config: Config = toml::from_str(&contents)
            .context("Failed to parse config file for Telegram binding")?;
        config.config_path = config_path;
        config.workspace_dir = openhuman_dir.join("workspace");
        Ok(config)
    }

    pub(crate) async fn persist_allowed_identity(&self, identity: &str) -> anyhow::Result<()> {
        let mut config = Self::load_config_without_env().await?;
        let Some(telegram) = config.channels_config.telegram.as_mut() else {
            anyhow::bail!("Telegram channel config is missing in config.toml");
        };

        let normalized = Self::normalize_identity(identity);
        if normalized.is_empty() {
            anyhow::bail!("Cannot persist empty Telegram identity");
        }

        if !telegram.allowed_users.iter().any(|u| u == &normalized) {
            telegram.allowed_users.push(normalized);
            config
                .save()
                .await
                .context("Failed to persist Telegram allowlist to config.toml")?;
        }

        Ok(())
    }

    pub(crate) fn add_allowed_identity_runtime(&self, identity: &str) {
        let normalized = Self::normalize_identity(identity);
        if normalized.is_empty() {
            return;
        }
        if let Ok(mut users) = self.allowed_users.write() {
            if !users.iter().any(|u| u == &normalized) {
                users.push(normalized);
            }
        }
    }
}
