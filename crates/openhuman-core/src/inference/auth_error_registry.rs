//! Process-lived registry of BYO provider auth failures (invalid / revoked
//! API key, HTTP 401 / 403).
//!
//! Single source of truth feeding both user-facing surfaces for a rejected
//! BYO key, so the two can never drift:
//!   - the **notification center** — a one-shot [`crate::core::bus::
//!     DomainEvent::ProviderApiKeyRejected`] published the first time a
//!     provider starts failing, and
//!   - the **AI-settings provider-error notice** — read live via the
//!     `openhuman.inference_provider_auth_errors` RPC ([`snapshot`]).
//!
//! Entries are recorded at the demote site
//! ([`provider::ops::log_byo_provider_auth_failure`](super::provider::ops::log_byo_provider_auth_failure)) and cleared
//! when the user updates or removes that provider's key
//! (`credentials::ops`). The [`record`] latch is what makes the notification
//! fire **once per failure episode** rather than once per retry: the
//! triggering 401 repeats thousands of times (the memory-summarization loop
//! re-attempts on every scoring pass — TAURI-RUST-4RC: ~9k events / 6 users),
//! and an unguarded publish would re-flood the notification center the same
//! way the raw error flooded Sentry.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use tinyinference_providers::auth_registry::ProviderAuthError;
use tinyinference_providers::auth_registry::ProviderAuthErrorRegistry;

/// The process-lived registry. Episode tracking lives in
/// `tinyinference-providers`; this module supplies the process singleton, the
/// user-facing copy and the clock.
fn registry() -> &'static ProviderAuthErrorRegistry {
    static REG: OnceLock<ProviderAuthErrorRegistry> = OnceLock::new();
    REG.get_or_init(ProviderAuthErrorRegistry::new)
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
         API key in Connections → API keys → LLM to restore it."
    )
}

/// Record a BYO provider auth failure.
///
/// Returns `true` when this is a **new** failure episode for `provider` (no
/// prior entry) — callers publish the one-shot notification only on `true` so
/// a 401 that repeats per-retry doesn't re-notify. An existing entry is
/// refreshed (status / timestamp) and returns `false`.
pub fn record(provider: &str, status: u16) -> bool {
    registry().record(ProviderAuthError {
        provider: provider.to_string(),
        status,
        message: auth_error_message(provider, status),
        timestamp_ms: now_ms(),
    })
}

/// Clear any recorded auth error for `provider` (the user updated / removed
/// the key, or a call newly succeeded). Returns `true` if an entry was
/// removed. Re-arms the [`record`] latch so a fresh failure re-notifies.
pub fn clear(provider: &str) -> bool {
    registry().clear(provider)
}

/// Snapshot all currently-recorded provider auth errors, sorted by provider
/// slug for a stable RPC payload.
pub fn snapshot() -> Vec<ProviderAuthError> {
    registry().snapshot()
}

/// Test-only: wipe the registry so a test can't be flaked by an entry an
/// earlier test left behind (the map is process-global).
#[cfg(any(test, debug_assertions))]
pub fn reset_for_tests() {
    registry().clear_all();
}

#[cfg(test)]
#[path = "auth_error_registry_tests.rs"]
mod tests;
