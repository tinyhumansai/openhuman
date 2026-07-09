//! In-memory peer presence for orchestration contacts / instances.
//!
//! Mirrors the devices-tunnel [`PEER_STATUS`](crate::openhuman::devices) pattern:
//! a process-local map keyed by peer `agent_id`, updated whenever we hear from a
//! peer (any inbound envelope, or a heartbeat `pong`), and read back at
//! `sessions_list` time to overlay a live `peerOnline` / `lastSeenAt` onto each
//! session summary. Not persisted — presence is a runtime property.
//!
//! Today the only feeder is inbound traffic (`mark_seen` on every ingested peer
//! envelope), so `is_online` reports a **confident online** (`Some(true)`) within
//! [`PRESENCE_TTL_MS`] and `None` (unknown → UI falls back to the recency
//! heuristic) otherwise. Once the cross-session heartbeat ping/pong lands, a
//! known-but-stale peer becomes a confident **offline** (`Some(false)`); the map
//! + TTL are already shaped for that (see `mark_seen` callers).

use std::collections::HashMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;

/// A peer seen within this window is reported online. Sized to sit comfortably
/// above the planned heartbeat cadence (~30s) so a single missed ping doesn't
/// flap the indicator.
pub const PRESENCE_TTL_MS: i64 = 120_000; // 2 minutes

#[derive(Clone, Copy, Debug)]
struct PeerPresence {
    last_seen_ms: i64,
}

/// Live peer presence keyed by canonical peer `agent_id`. Process-local; a
/// restart starts empty and re-learns from the first inbound / heartbeat.
static PEER_STATUS: Lazy<Mutex<HashMap<String, PeerPresence>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Record that we just heard from `agent_id` — call on every inbound peer
/// envelope and on a heartbeat `pong`. No-op for an empty id.
pub fn mark_seen(agent_id: &str) {
    if agent_id.trim().is_empty() {
        return;
    }
    log::debug!(target: "orchestration", "[presence] mark_seen agent_id={agent_id}");
    PEER_STATUS.lock().unwrap().insert(
        agent_id.to_string(),
        PeerPresence {
            last_seen_ms: now_ms(),
        },
    );
}

/// Whether `agent_id` is currently online.
///
/// - `Some(true)` — heard from within [`PRESENCE_TTL_MS`].
/// - `None` — never heard from, or last seen longer ago than the TTL. The UI
///   treats `None` as "unknown" and falls back to the recency-based `active`
///   heuristic, so this source can only ever *upgrade* to a confident online
///   (never assert a false offline before the heartbeat exists).
pub fn is_online(agent_id: &str) -> Option<bool> {
    let map = PEER_STATUS.lock().unwrap();
    let seen = map.get(agent_id)?;
    (now_ms() - seen.last_seen_ms <= PRESENCE_TTL_MS).then_some(true)
}

/// ISO-8601 (RFC 3339) last-seen timestamp for `agent_id`, if we've ever heard
/// from it. `None` when unknown.
pub fn last_seen_iso(agent_id: &str) -> Option<String> {
    let map = PEER_STATUS.lock().unwrap();
    let seen = map.get(agent_id)?;
    chrono::DateTime::from_timestamp_millis(seen.last_seen_ms).map(|dt| dt.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_peer_is_none() {
        assert_eq!(is_online("@nobody-xyz"), None);
        assert_eq!(last_seen_iso("@nobody-xyz"), None);
    }

    #[test]
    fn mark_seen_marks_online_with_timestamp() {
        let id = "@presence-test-peer";
        mark_seen(id);
        assert_eq!(is_online(id), Some(true));
        assert!(last_seen_iso(id).is_some(), "last-seen recorded");
    }

    #[test]
    fn empty_id_is_noop() {
        mark_seen("");
        assert_eq!(is_online(""), None);
    }
}
