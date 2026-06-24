//! Process-lived registry of BYO provider auth failures (invalid / revoked
//! API key, HTTP 401 / 403).
//!
//! Single source of truth feeding both user-facing surfaces for a rejected
//! BYO key, so the two can never drift:
//!   - the **notification center** — a one-shot [`crate::core::event_bus::
//!     DomainEvent::ProviderApiKeyRejected`] published the first time a
//!     provider starts failing, and
//!   - the **AI-settings provider-error notice** — read live via the
//!     `openhuman.inference_provider_auth_errors` RPC ([`snapshot`]).
//!
//! Entries are recorded at the demote site
//! ([`super::ops::http_error::log_byo_provider_auth_failure`]) and cleared
//! when the user updates or removes that provider's key
//! (`credentials::ops`). The [`record`] latch is what makes the notification
//! fire **once per failure episode** rather than once per retry: the
//! triggering 401 repeats thousands of times (the memory-summarization loop
//! re-attempts on every scoring pass — TAURI-RUST-4RC: ~9k events / 6 users),
//! and an unguarded publish would re-flood the notification center the same
//! way the raw error flooded Sentry.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// A recorded BYO provider auth failure. Serialized verbatim onto the
/// `inference_provider_auth_errors` RPC snapshot the AI settings panel reads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderAuthError {
    /// Provider slug as used by the chat factory / classifier, e.g.
    /// `"openrouter"`.
    pub provider: String,
    /// The rejecting HTTP status — always `401` or `403` (the classifier
    /// gate), kept for fidelity in the surfaced copy.
    pub status: u16,
    /// Pre-formatted, user-facing, actionable message — safe to show in a
    /// notification or settings notice as-is. See [`auth_error_message`].
    pub message: String,
    /// Wall-clock milliseconds since the unix epoch when last recorded.
    pub timestamp_ms: u64,
}

fn registry() -> &'static RwLock<HashMap<String, ProviderAuthError>> {
    static REG: OnceLock<RwLock<HashMap<String, ProviderAuthError>>> = OnceLock::new();
    REG.get_or_init(|| RwLock::new(HashMap::new()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Build the actionable, user-facing message for a BYO key the provider
/// rejected. Phrased so it reads the same in the notification center and the
/// AI-settings notice, and points the user at the one fix that resolves it
/// (the third-party key is invalid / revoked — OpenHuman has no other lever).
pub fn auth_error_message(provider: &str, status: u16) -> String {
    format!(
        "{provider} rejected the API key (HTTP {status}). Update your {provider} \
         API key in Settings → AI to restore it."
    )
}

/// Record a BYO provider auth failure.
///
/// Returns `true` when this is a **new** failure episode for `provider` (no
/// prior entry) — callers publish the one-shot notification only on `true` so
/// a 401 that repeats per-retry doesn't re-notify. An existing entry is
/// refreshed (status / timestamp) and returns `false`.
pub fn record(provider: &str, status: u16) -> bool {
    let entry = ProviderAuthError {
        provider: provider.to_string(),
        status,
        message: auth_error_message(provider, status),
        timestamp_ms: now_ms(),
    };
    let mut map = registry().write().unwrap_or_else(|e| e.into_inner());
    let is_new = !map.contains_key(provider);
    map.insert(provider.to_string(), entry);
    is_new
}

/// Clear any recorded auth error for `provider` (the user updated / removed
/// the key, or a call newly succeeded). Returns `true` if an entry was
/// removed. Re-arms the [`record`] latch so a fresh failure re-notifies.
pub fn clear(provider: &str) -> bool {
    registry()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(provider)
        .is_some()
}

/// Snapshot all currently-recorded provider auth errors, sorted by provider
/// slug for a stable RPC payload.
pub fn snapshot() -> Vec<ProviderAuthError> {
    let map = registry().read().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<ProviderAuthError> = map.values().cloned().collect();
    out.sort_by(|a, b| a.provider.cmp(&b.provider));
    out
}

/// Test-only: wipe the registry so a test can't be flaked by an entry an
/// earlier test left behind (the map is process-global).
#[cfg(any(test, debug_assertions))]
pub fn reset_for_tests() {
    registry()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // The registry is process-global, so these tests would otherwise race:
    // one test's `reset_for_tests` can wipe another's entries mid-run. Serialize
    // them on a shared lock (poison-tolerant) so each runs against a clean,
    // exclusive registry.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn record_returns_true_only_on_first_episode() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_tests();
        assert!(record("openrouter", 401), "first record is a new episode");
        assert!(
            !record("openrouter", 401),
            "repeat record of same provider does not re-notify"
        );
        // A different provider is its own episode.
        assert!(record("openai", 403));
        reset_for_tests();
    }

    #[test]
    fn clear_removes_entry_and_rearms_latch() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_tests();
        assert!(record("openrouter", 401));
        assert!(clear("openrouter"), "entry was present");
        assert!(!clear("openrouter"), "second clear is a no-op");
        // After clear, the next failure is a new episode again.
        assert!(record("openrouter", 401), "latch re-armed after clear");
        reset_for_tests();
    }

    #[test]
    fn snapshot_is_sorted_and_carries_actionable_message() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_tests();
        record("openrouter", 401);
        record("anthropic", 401);
        let snap = snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].provider, "anthropic");
        assert_eq!(snap[1].provider, "openrouter");
        assert!(snap[1].message.contains("openrouter"));
        assert!(snap[1].message.contains("Settings"));
        reset_for_tests();
    }
}
