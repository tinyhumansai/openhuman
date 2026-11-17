//! Shared browser sessions across turns in one conversation.

use crate::modules::browser::BrowserClient;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tinycomputer_bus::browser::SessionId;
use tokio::sync::Mutex;

pub(super) const MAX_THREAD_SESSIONS: usize = 6;
pub(super) const SESSION_IDLE_TTL: Duration = Duration::from_secs(30 * 60);

pub(super) struct ThreadSession {
    pub(super) id: SessionId,
    pub(super) client: Arc<BrowserClient>,
    pub(super) last_used: Instant,
    pub(super) config_fingerprint: String,
    pub(super) bound_origin: Option<String>,
    /// A keychain password was typed in this session: it is never pictured
    /// again, whichever tool picks it up next (`BrowserTool::capture`).
    pub(super) password_typed: bool,
}

static THREAD_SESSIONS: OnceLock<Mutex<HashMap<String, ThreadSession>>> = OnceLock::new();

pub(super) fn thread_sessions() -> &'static Mutex<HashMap<String, ThreadSession>> {
    THREAD_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn browser_session_fingerprint(client: &BrowserClient) -> String {
    let config = client.config();
    // A session opened with older settings or website policy cannot be reused.
    let allow_all = matches!(
        std::env::var("OPENHUMAN_BROWSER_ALLOW_ALL").ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    );
    let bytes = serde_json::to_vec(&json!({
        "browser": &config.browser,
        "allowed_domains": &config.http_request.allowed_domains,
        "allow_all": allow_all,
    }))
    .expect("browser configuration is JSON serializable");
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn requires_rebind(bound: Option<&str>, requested: Option<&str>) -> bool {
    requested.is_some_and(|origin| bound != Some(origin))
}

pub(super) fn evict_thread_sessions(
    sessions: &mut HashMap<String, ThreadSession>,
    now: Instant,
    reserve_slot: bool,
) -> Vec<ThreadSession> {
    let mut removed = Vec::new();
    let expired = sessions
        .iter()
        .filter(|(_, value)| now.duration_since(value.last_used) >= SESSION_IDLE_TTL)
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    for key in expired {
        if let Some(value) = sessions.remove(&key) {
            removed.push(value);
        }
    }
    while reserve_slot && sessions.len() >= MAX_THREAD_SESSIONS {
        let Some(oldest) = sessions
            .iter()
            .min_by_key(|(_, value)| value.last_used)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        if let Some(value) = sessions.remove(&oldest) {
            removed.push(value);
        }
    }
    removed
}
