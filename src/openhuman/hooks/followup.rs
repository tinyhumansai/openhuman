//! Follow-up messages injected by `stop` hooks.
//!
//! A `stop` hook can answer with `followup_message`, asking for one more turn —
//! "you did not run the tests, run them". The hook fires *after* the turn has
//! already returned to its caller, so the message cannot re-enter the turn that
//! produced it. It is queued here instead, and the entrypoint that owns the
//! conversation decides whether to spend another turn on it.
//!
//! That indirection is not an implementation shortcut. Only the entrypoint
//! knows whether there is still a user attached: a chat turn can be extended,
//! a cron run that has already reported its result cannot, and a channel turn
//! that has flushed its reply would surprise the recipient. A queue lets each
//! answer for itself, and lets a host that answers "no" simply never drain it.
//!
//! [`HookEngine`](super::engine::HookEngine)'s `loop_limit` accounting has
//! already been charged by the time a message lands here, so a drain loop
//! cannot spin: a hook that keeps asking runs out of budget.

use std::collections::HashMap;
use tokio::sync::{broadcast, RwLock};

/// A queued follow-up.
#[derive(Debug, Clone)]
pub struct Followup {
    /// Session the message belongs to, when the hook fired inside one.
    pub session_id: Option<String>,
    /// The message to send as the next user turn.
    pub message: String,
}

/// Messages queued per session, plus one bucket for sessionless turns.
static PENDING: std::sync::LazyLock<RwLock<HashMap<String, Vec<String>>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

/// Key used for follow-ups that arrived without a session id.
const SESSIONLESS: &str = "";

static CHANNEL: std::sync::LazyLock<broadcast::Sender<Followup>> = std::sync::LazyLock::new(|| {
    // A small buffer: a listener that falls this far behind has stopped caring,
    // and dropping old follow-ups is better than holding a turn's worth of
    // stale instructions.
    broadcast::channel(64).0
});

/// Queue a follow-up and notify any live listener.
pub async fn publish(session_id: Option<String>, message: String) {
    let key = session_id
        .clone()
        .unwrap_or_else(|| SESSIONLESS.to_string());
    log::info!(
        "[hooks] stop hook queued a follow-up for session {:?} ({} chars)",
        session_id,
        message.chars().count()
    );
    PENDING
        .write()
        .await
        .entry(key)
        .or_default()
        .push(message.clone());
    // A send with no subscribers is not an error here — the queue is the
    // durable half, the channel only wakes a listener that already exists.
    let _ = CHANNEL.send(Followup {
        session_id,
        message,
    });
}

/// Listen for follow-ups as they are queued.
pub fn subscribe() -> broadcast::Receiver<Followup> {
    CHANNEL.subscribe()
}

/// Take everything queued for a session, clearing it.
pub async fn take(session_id: Option<&str>) -> Vec<String> {
    let key = session_id.unwrap_or(SESSIONLESS);
    PENDING.write().await.remove(key).unwrap_or_default()
}

/// Drop anything still queued for a finished session.
pub async fn forget(session_id: &str) {
    PENDING.write().await.remove(session_id);
}

#[cfg(test)]
#[path = "followup_tests.rs"]
mod tests;
