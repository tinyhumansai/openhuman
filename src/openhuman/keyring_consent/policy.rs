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

/// Pre-populate the consent cache from persisted app state. Call once at core
/// startup after config is loadable.
pub fn initialize(consent: Option<ConsentPreference>) {
    info!(
        "{LOG_PREFIX} initialize cached_consent={}",
        consent.as_ref().map_or("none", |p| p.storage_mode.as_str()),
    );
    *CONSENT_CACHE.write() = consent;
}

/// Check whether the caller is allowed to proceed with secret storage.
pub fn check_secret_access() -> PolicyDecision {
    if crate::openhuman::keyring::is_available() {
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
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::KeyringConsentRequired,
                );
            }
            PolicyDecision::ConsentRequired
        }
    }
}

/// Build the current keyring status for RPC / snapshot consumption.
pub fn current_status() -> KeyringStatus {
    let available = crate::openhuman::keyring::is_available();
    let backend_name = crate::openhuman::keyring::backend_name();

    let (active_mode, failure_reason) = if available {
        (StorageMode::OsKeyring, None)
    } else {
        let reason = classify_failure_reason(&backend_name);
        let cached = CONSENT_CACHE.read().clone();
        let mode = match cached {
            Some(ref p) if p.storage_mode == "local_encrypted" => StorageMode::LocalEncrypted,
            Some(ref p) if p.storage_mode == "declined" => StorageMode::Declined,
            _ => StorageMode::ConsentPending,
        };
        (mode, Some(reason))
    };

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
    crate::openhuman::keyring::reset_availability_cache();
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
        crate::core::event_bus::publish_global(
            crate::core::event_bus::DomainEvent::KeyringConsentRequired,
        );
    }
}

/// Publish a decrypt-failure event for frontend notification.
pub fn notify_decrypt_failure(field_name: &str, reason: &str) {
    warn!("{LOG_PREFIX} decrypt failure field={field_name} reason={reason}");
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::KeyringDecryptFailed {
            field_name: field_name.to_string(),
            reason: reason.to_string(),
        },
    );
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
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn cache_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("keyring consent cache test lock")
    }

    #[test]
    fn classify_failure_linux() {
        if cfg!(target_os = "linux") {
            let reason = classify_failure_reason("os");
            assert_eq!(reason, KeyringFailureReason::NoSecretService);
        }
    }

    #[test]
    fn classify_failure_macos() {
        if cfg!(target_os = "macos") {
            let reason = classify_failure_reason("os");
            assert_eq!(reason, KeyringFailureReason::AccessDenied);
        }
    }

    #[test]
    fn classify_failure_encrypted_file() {
        let reason = classify_failure_reason("encrypted_file");
        assert_eq!(reason, KeyringFailureReason::MasterKeyUnavailable);
    }

    #[test]
    fn classify_failure_unknown() {
        let reason = classify_failure_reason("weird_backend");
        assert!(matches!(reason, KeyringFailureReason::Unknown(_)));
    }

    #[test]
    fn record_consent_updates_cache() {
        let _lock = cache_test_lock();
        let pref = record_consent("local_encrypted");
        assert_eq!(pref.storage_mode, "local_encrypted");
        assert!(pref.consented_at_ms.is_some());

        let cached = CONSENT_CACHE.read().clone();
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().storage_mode, "local_encrypted");
    }

    #[test]
    fn initialize_populates_cache() {
        let _lock = cache_test_lock();
        let pref = ConsentPreference {
            storage_mode: "declined".to_string(),
            consented_at_ms: Some(12345),
        };
        initialize(Some(pref.clone()));
        let cached = CONSENT_CACHE.read().clone();
        assert_eq!(cached.unwrap().storage_mode, "declined");
    }
}
