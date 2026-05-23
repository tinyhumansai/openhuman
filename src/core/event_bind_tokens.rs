//! Per-subscription bind tokens for the SSE `/events` endpoint.
//!
//! Browser `EventSource` clients cannot attach an `Authorization` header,
//! so the `/events` stream cannot ride on the same bearer-token middleware
//! that protects `POST /rpc`. Instead, an authenticated holder of the
//! per-process RPC bearer first calls
//! `core.events_subscribe_token { client_id }` to mint a short-lived,
//! single-purpose bind token, then opens
//! `/events?client_id=<id>&token=<bind>`.
//!
//! Properties of the bind token:
//! - 256 bits of CSPRNG randomness (hex-encoded; 64 chars on the wire).
//! - Bound to one `client_id` — verifying with any other id rejects.
//! - Single-shot by default: the connect-time validate step removes the
//!   token from the store, so a leaked URL cannot be reused.
//! - Time-bounded: minted tokens carry a `valid_until` instant and a
//!   small purge pass runs on each lookup to bound store size.
//!
//! This module owns only the in-memory store; the RPC handler that mints
//! tokens lives in `src/core/dispatch.rs` (the `core.*` namespace),
//! and the `/events` handler in `src/core/jsonrpc.rs` consumes them.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

/// Default lifetime of a freshly issued bind token if the caller does not
/// specify one. Long enough for normal subscribe latency, short enough that
/// an accidentally-logged URL stops working before useful exfil. The RPC
/// caller can shorten this with the `ttl_secs` field.
const DEFAULT_TTL: Duration = Duration::from_secs(60);

/// Upper bound the caller can request. Anything larger collapses to this so
/// a misbehaving (or compromised) caller cannot mint long-lived tokens.
const MAX_TTL: Duration = Duration::from_secs(60 * 30);

/// Maximum live tokens in the store. Each token is ~80 bytes plus the
/// `client_id` String; this is a defensive ceiling, not a normal-load cap.
/// When the store is full, the oldest expired entries are evicted; if none
/// are expired, a fresh issue request is rejected so the store cannot grow
/// without bound.
const MAX_TOKENS: usize = 4096;

#[derive(Debug, Clone)]
struct BindEntry {
    client_id: String,
    valid_until: Instant,
}

static STORE: Lazy<RwLock<HashMap<String, BindEntry>>> =
    Lazy::new(|| RwLock::new(HashMap::with_capacity(64)));

/// A freshly-minted bind token plus its expiry. Returned to the RPC caller
/// so the UI can pass both to `/events?client_id=…&token=…`.
#[derive(Debug, Clone)]
pub struct BindToken {
    pub token: String,
    pub valid_until: Instant,
}

/// Mint a new bind token tied to `client_id`.
///
/// `ttl_override` lets the caller request a shorter lifetime than the
/// default; anything above `MAX_TTL` is clamped down. Returns `None` if the
/// store is at capacity and no expired entries can be reclaimed — callers
/// should surface this as a transient error rather than retrying in a
/// tight loop.
pub fn issue(client_id: impl Into<String>, ttl_override: Option<Duration>) -> Option<BindToken> {
    let ttl = ttl_override.map(|d| d.min(MAX_TTL)).unwrap_or(DEFAULT_TTL);
    let client_id = client_id.into();
    let valid_until = Instant::now() + ttl;
    let token = generate_token();
    let entry = BindEntry {
        client_id,
        valid_until,
    };

    let mut store = STORE.write().ok()?;
    purge_expired_locked(&mut store);
    if store.len() >= MAX_TOKENS {
        log::warn!(
            "[events-bind] capacity reached ({} entries) — refusing to mint",
            store.len()
        );
        return None;
    }
    store.insert(token.clone(), entry);
    Some(BindToken { token, valid_until })
}

/// Validate a supplied `(client_id, token)` pair and remove the token from
/// the store on success.
///
/// Returns `true` only when the token exists, is not expired, and the
/// bound `client_id` matches what was supplied. The remove-on-success
/// behaviour is what gives the token its single-shot semantics — an
/// attacker who replays the URL after the legitimate UI has connected
/// gets nothing.
pub fn consume(client_id: &str, token: &str) -> bool {
    let Ok(mut store) = STORE.write() else {
        return false;
    };
    purge_expired_locked(&mut store);
    // Peek before removing: a wrong `client_id` must NOT consume the token,
    // or a single guessed-id request can DoS the legitimate subscriber by
    // racing them to the consume.
    let Some(entry) = store.get(token) else {
        log::debug!("[events-bind] consume: token not found");
        return false;
    };
    if entry.client_id != client_id {
        log::warn!("[events-bind] consume: client_id mismatch (token bound to other id)");
        return false;
    }
    let entry = store
        .remove(token)
        .expect("token was present in the binding check above");
    log::debug!(
        "[events-bind] consume: ok (client_id_len={} ttl_remaining_ms={})",
        entry.client_id.len(),
        entry
            .valid_until
            .checked_duration_since(Instant::now())
            .unwrap_or_default()
            .as_millis()
    );
    true
}

fn purge_expired_locked(store: &mut HashMap<String, BindEntry>) {
    let now = Instant::now();
    store.retain(|_, entry| entry.valid_until > now);
}

fn generate_token() -> String {
    use rand::RngExt as _;
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_token_validates_for_matching_client_id() {
        let issued = issue("cli-test-1", None).expect("issue");
        assert!(consume("cli-test-1", &issued.token));
    }

    #[test]
    fn issued_token_rejects_wrong_client_id() {
        let issued = issue("cli-test-2", None).expect("issue");
        assert!(!consume("attacker-id", &issued.token));
    }

    #[test]
    fn wrong_client_id_does_not_consume_token() {
        // Mismatched consume must leave the token intact so the legitimate
        // subscriber can still validate after the failed probe — otherwise
        // a wrong-id request becomes a one-shot DoS.
        let issued = issue("cli-test-mismatch", None).expect("issue");
        assert!(!consume("attacker-id", &issued.token));
        assert!(
            consume("cli-test-mismatch", &issued.token),
            "legitimate consume must still succeed after a mismatched probe"
        );
    }

    #[test]
    fn consumed_token_cannot_be_reused() {
        let issued = issue("cli-test-3", None).expect("issue");
        assert!(consume("cli-test-3", &issued.token));
        assert!(
            !consume("cli-test-3", &issued.token),
            "tokens must be single-shot"
        );
    }

    #[test]
    fn expired_token_is_rejected() {
        let issued = issue("cli-test-4", Some(Duration::from_millis(1))).expect("issue");
        std::thread::sleep(Duration::from_millis(20));
        assert!(!consume("cli-test-4", &issued.token));
    }

    #[test]
    fn unknown_token_is_rejected() {
        assert!(!consume("any-id", "f00ba1"));
    }

    #[test]
    fn ttl_override_is_clamped_to_max() {
        // Any caller asking for more than `MAX_TTL` collapses to the cap;
        // confirm the issue path does not panic and the resulting token
        // still validates.
        let issued =
            issue("cli-test-clamp", Some(Duration::from_secs(60 * 60 * 24))).expect("issue");
        assert!(issued.valid_until <= Instant::now() + MAX_TTL + Duration::from_secs(1));
        assert!(consume("cli-test-clamp", &issued.token));
    }
}
