//! Telegram channel — private types and the main struct definition.

use crate::openhuman::config::StreamMode;
use crate::openhuman::security::pairing::PairingGuard;
use parking_lot::Mutex;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::Instant;

pub(crate) const TELEGRAM_RECENT_UPDATE_CACHE_SIZE: usize = 4096;

/// De-bounce window for approval prompts: suppress duplicate prompts sent to the
/// same chat+sender within this duration (prevents restart-race and rapid-fire spam).
pub(crate) const APPROVAL_PROMPT_DEBOUNCE_SECS: u64 = 60;

pub(crate) struct TelegramTypingTask {
    pub(crate) recipient: String,
    pub(crate) handle: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
pub(crate) struct TelegramUpdateWindow {
    pub(crate) max_seen_update_id: i64,
    pub(crate) recent_order: VecDeque<i64>,
    pub(crate) recent_lookup: HashSet<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct TelegramReactionEvent {
    pub(crate) sender: String,
    pub(crate) reply_target: String,
    pub(crate) target_message_id: String,
    pub(crate) emoji: String,
}

/// Telegram channel — long-polls the Bot API for updates
pub struct TelegramChannel {
    pub(crate) bot_token: String,
    /// Base URL for the Telegram Bot API. Defaults to `https://api.telegram.org`.
    /// Override via `OPENHUMAN_TELEGRAM_BOT_API_BASE` for E2E testing against a
    /// mock server. The legacy `OPENHUMAN_TELEGRAM_API_BASE` alias is still accepted.
    pub(crate) api_base: String,
    pub(crate) allowed_users: Arc<RwLock<Vec<String>>>,
    pub(crate) pairing: Option<PairingGuard>,
    pub(crate) client: reqwest::Client,
    pub(crate) typing_handle: Mutex<Option<TelegramTypingTask>>,
    pub(crate) stream_mode: StreamMode,
    pub(crate) draft_update_interval_ms: u64,
    pub(crate) silent_streaming: bool,
    pub(crate) last_draft_edit: Mutex<std::collections::HashMap<String, Instant>>,
    pub(crate) mention_only: bool,
    pub(crate) bot_username: Mutex<Option<String>>,
    pub(crate) recent_updates: Mutex<TelegramUpdateWindow>,
    /// Tracks the last time an approval prompt was sent to a given "chat_id:sender" key.
    /// Prevents duplicate prompts during restart-overlap races and rapid re-sends.
    /// Mirrors the `last_draft_edit` pattern.
    pub(crate) recent_approval_prompts: Mutex<std::collections::HashMap<String, Instant>>,
}
