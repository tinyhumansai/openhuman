//! Unified keyring fallback policy gate.
//!
//! All code paths that read or write secrets should call [`check_secret_access`]
//! instead of raw `keyring::is_available()`. This centralises the consent check
//! so the app never silently falls back to local encrypted storage without the
//! user's explicit agreement.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use log::{debug, info, warn};
use parking_lot::RwLock;

use super::types::{
    ConsentPreference, KeyringFailureReason, KeyringStatus, PolicyDecision, StorageMode,
};

const LOG_PREFIX: &str = "[keyring_consent]";

static CONSENT_EVENT_PUBLISHED: AtomicBool = AtomicBool::new(false);

/// Process-wide cached consent preference. Updated by [`record_consent`] and
/// [`initialize`]. Read by [`check_secret_access`] and [`current_status`] so
/// they never touch disk on the hot path.
static CONSENT_CACHE: RwLock<Option<ConsentPreference>> = RwLock::new(None);

/// Populate the consent cache from persisted app state.
///
/// Called from the per-request `app_state_snapshot` path, so it runs many times
/// over a session — not once at startup as the name suggests. It is therefore
/// change-gated: it writes and logs only when the persisted consent actually
/// differs from what is already cached. A repeat call with the same value (the
/// common case on every snapshot) is a silent no-op, keeping boot logs clean.
///
/// Returns `true` when the cache was updated (the INFO log fired) and `false`
/// on the no-op path — this lets callers/tests observe the suppressed side
/// effect directly rather than only the (identical) resulting cache value.
pub fn initialize(consent: Option<ConsentPreference>) -> bool {
    // Hold the write lock across the compare + set so concurrent snapshots
    // can't both observe a change and double-log / double-write.
    let mut cache = CONSENT_CACHE.write();
    if *cache == consent {
        // No-op path (every app_state_snapshot with unchanged consent). Trace so
        // it stays diagnosable without the INFO noise this change removes.
        log::trace!("{LOG_PREFIX} initialize no-op: cached consent unchanged");
        return false;
    }
    info!(
        "{LOG_PREFIX} initialize cached_consent={}",
        consent.as_ref().map_or("none", |p| p.storage_mode.as_str()),
    );
    *cache = consent;
    true
}

/// Check whether the caller is allowed to proceed with secret storage.
pub fn check_secret_access() -> PolicyDecision {
    if crate::openhuman::security::keyring::is_available() {
        return PolicyDecision::Proceed;
    }

    let cached = CONSENT_CACHE.read().clone();
    match cached {
        Some(ref pref) if pref.storage_mode == "local_encrypted" => {
            debug!("{LOG_PREFIX} check_secret_access: consent=local_encrypted, proceeding");
            PolicyDecision::Proceed
        }
        Some(ref pref) if pref.storage_mode == "declined" => {
            debug!("{LOG_PREFIX} check_secret_access: consent=declined");
            PolicyDecision::Declined
        }
        _ => {
            debug!("{LOG_PREFIX} check_secret_access: keyring unavailable, no consent recorded");
            if !CONSENT_EVENT_PUBLISHED.swap(true, Ordering::SeqCst) {
                info!("{LOG_PREFIX} publishing KeyringConsentRequired event");
                crate::core::bus::BUS
                    .publish(crate::core::events::DomainEvent::KeyringConsentRequired);
            }
            PolicyDecision::ConsentRequired
        }
    }
}

/// Backend identifiers as [`crate::openhuman::security::keyring::backend_name`]
/// reports them. Kept here rather than matched as bare literals so the mapping
/// below reads as a table and a rename upstream fails in one place.
const BACKEND_OS: &str = "os";
const BACKEND_ENCRYPTED_FILE: &str = "encrypted_file";
const BACKEND_FILE: &str = "file";
const BACKEND_MOCK: &str = "mock";

/// Translate a recorded consent decision into the mode it selected.
///
/// Only meaningful on the `os` path: consent is asked for exactly when the OS
/// keyring was the intended store and could not be used.
fn consent_mode(cached: Option<&ConsentPreference>) -> StorageMode {
    match cached {
        Some(p) if p.storage_mode == "local_encrypted" => StorageMode::LocalEncrypted,
        Some(p) if p.storage_mode == "declined" => StorageMode::Declined,
        _ => StorageMode::ConsentPending,
    }
}

/// Decide what [`StorageMode`] describes this process, from the **backend
/// identity** first and availability second.
///
/// Split out of [`current_status`] as a pure function so every combination can
/// be asserted without touching the process-global backend `OnceLock` (which
/// `force_backend_for_test` can only set once per test binary).
///
/// This used to branch on `available` alone, which was wrong for every
/// non-`os` backend: `probe_availability` short-circuits to `true` for `file`,
/// `mock` and `encrypted_file`, so all three reported `os_keyring` beside a
/// `backend_name` that said otherwise, and a recorded `declined` decision could
/// not move it (#6076). In staging and production — where `encrypted_file` is
/// the default — that told the user their secrets were in the OS keychain while
/// they were in `{workspace}/secrets.enc`.
fn active_mode_for(
    available: bool,
    backend_name: &str,
    cached: Option<&ConsentPreference>,
) -> StorageMode {
    match backend_name {
        // The only backend that actually stores secrets in the OS credential
        // store — and only while its probe passes.
        BACKEND_OS => {
            if available {
                StorageMode::OsKeyring
            } else {
                consent_mode(cached)
            }
        }
        // Operator-configured backends. No consent was ever asked for, so the
        // consent cache says nothing about where these secrets are.
        BACKEND_ENCRYPTED_FILE => StorageMode::LocalEncryptedFile,
        BACKEND_FILE | BACKEND_MOCK => StorageMode::LocalPlaintextFile,
        // A backend added without extending this table. Reporting `os_keyring`
        // is exactly the bug above, so claim nothing instead: `backend_name`
        // still ships in the payload and names it.
        other => {
            warn!(
                "{LOG_PREFIX} unrecognised keyring backend '{other}': reporting \
                 active_mode=consent_pending rather than guessing where secrets live"
            );
            StorageMode::ConsentPending
        }
    }
}

/// Build the current keyring status for RPC / snapshot consumption.
pub fn current_status() -> KeyringStatus {
    let available = crate::openhuman::security::keyring::is_available();
    let backend_name = crate::openhuman::security::keyring::backend_name();
    let cached = CONSENT_CACHE.read().clone();

    let active_mode = active_mode_for(available, &backend_name, cached.as_ref());
    // `failure_reason` stays tied to the probe, not to the mode: a file backend
    // is genuinely available, it just is not the OS keyring.
    let failure_reason = (!available).then(|| classify_failure_reason(&backend_name));

    KeyringStatus {
        available,
        failure_reason,
        active_mode,
        backend_name,
    }
}

/// Build a consent preference value without touching the in-memory cache.
///
/// Callers that need to persist before caching should use this together with
/// [`apply_consent`]: build → persist → apply. This ordering ensures the cache
/// and disk never diverge (if persistence fails the cache is not updated).
pub fn build_consent_preference(mode: &str) -> ConsentPreference {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    ConsentPreference {
        storage_mode: mode.to_string(),
        consented_at_ms: Some(now_ms),
    }
}

/// Apply a previously-built consent preference to the in-memory cache.
///
/// Call this only after the preference has been successfully persisted to disk.
pub fn apply_consent(pref: &ConsentPreference) {
    info!(
        "{LOG_PREFIX} apply_consent mode={} at_ms={}",
        pref.storage_mode,
        pref.consented_at_ms.unwrap_or(0),
    );
    *CONSENT_CACHE.write() = Some(pref.clone());
    CONSENT_EVENT_PUBLISHED.store(false, Ordering::SeqCst);
}

/// Record the user's consent decision: update the in-memory cache and return
/// the preference for the RPC caller to persist via `update_local_state`.
///
/// Prefer the [`build_consent_preference`] + [`apply_consent`] pair when you
/// need to guarantee persistence happens before the cache is updated.
pub fn record_consent(mode: &str) -> ConsentPreference {
    let pref = build_consent_preference(mode);
    info!(
        "{LOG_PREFIX} record_consent mode={mode} at_ms={}",
        pref.consented_at_ms.unwrap_or(0)
    );
    apply_consent(&pref);
    pref
}

/// Reset the cached keyring probe and re-run it.
pub fn retry_probe() -> KeyringStatus {
    info!("{LOG_PREFIX} retry_probe: resetting availability cache");
    crate::openhuman::security::keyring::reset_availability_cache();
    CONSENT_EVENT_PUBLISHED.store(false, Ordering::SeqCst);
    current_status()
}

/// Surface a master-key load failure (e.g. OS keychain access denied after an
/// app update) to the frontend by publishing the consent-required event.
///
/// Unlike [`check_secret_access`], this is called proactively at core startup
/// when the encrypted-file backend cannot load its master key — so the user is
/// warned *before* any secret read silently returns empty, rather than letting
/// the failure pass unnoticed (the #3311 symptom: keys "wiped" with no warning).
/// It reuses the same `CONSENT_EVENT_PUBLISHED` dedup flag as the lazy gate so
/// we never double-publish if a secret op also hits the gate this session.
pub fn notify_master_key_unavailable(reason: &str) {
    warn!("{LOG_PREFIX} master key unavailable: {reason}");
    if !CONSENT_EVENT_PUBLISHED.swap(true, Ordering::SeqCst) {
        info!("{LOG_PREFIX} publishing KeyringConsentRequired event (master key unavailable)");
        crate::core::bus::BUS.publish(crate::core::events::DomainEvent::KeyringConsentRequired);
    }
}

/// Publish a decrypt-failure event for frontend notification.
pub fn notify_decrypt_failure(field_name: &str, reason: &str) {
    warn!("{LOG_PREFIX} decrypt failure field={field_name} reason={reason}");
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::KeyringDecryptFailed {
        field_name: field_name.to_string(),
        reason: reason.to_string(),
    });
}

fn classify_failure_reason(backend_name: &str) -> KeyringFailureReason {
    match backend_name {
        "os" => {
            if cfg!(target_os = "linux") {
                KeyringFailureReason::NoSecretService
            } else if cfg!(target_os = "macos") {
                KeyringFailureReason::AccessDenied
            } else {
                KeyringFailureReason::Unknown("OS keyring probe failed".to_string())
            }
        }
        "encrypted_file" => KeyringFailureReason::MasterKeyUnavailable,
        _ => KeyringFailureReason::Unknown(format!("Backend '{backend_name}' unavailable")),
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
