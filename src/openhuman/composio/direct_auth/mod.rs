//! Direct-mode Composio API-key health tracking.
//!
//! Invalid/revoked BYO keys used to make every 5s poll hit Composio v3 and
//! fail again. This module keeps a process-local consecutive-failure counter
//! keyed by a non-logged key fingerprint so repeated `401 Invalid API key`
//! responses open a short-circuit gate until the user re-enters the key.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, Mutex, MutexGuard};

pub(crate) const DIRECT_INVALID_API_KEY_THRESHOLD: u32 = 3;

static DIRECT_AUTH_FAILURES: LazyLock<Mutex<HashMap<u64, u32>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectAuthFailureDecision {
    NotAuthFailure,
    RetryAllowed { consecutive: u32 },
    CircuitOpened { consecutive: u32 },
}

fn failure_counts() -> MutexGuard<'static, HashMap<u64, u32>> {
    DIRECT_AUTH_FAILURES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn fingerprint_api_key(api_key: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    api_key.trim().hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn is_invalid_api_key_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("invalid api key")
        || (lower.contains("401") && lower.contains("api key") && lower.contains("invalid"))
}

pub(crate) fn record_direct_auth_success(key_id: u64) {
    failure_counts().remove(&key_id);
}

pub(crate) fn reset_direct_auth_failure(key_id: u64) {
    failure_counts().remove(&key_id);
}

pub(crate) fn reset_all_direct_auth_failures() {
    failure_counts().clear();
}

pub(crate) fn record_direct_auth_failure(key_id: u64, message: &str) -> DirectAuthFailureDecision {
    if !is_invalid_api_key_error(message) {
        reset_direct_auth_failure(key_id);
        return DirectAuthFailureDecision::NotAuthFailure;
    }

    let mut counts = failure_counts();
    let consecutive = counts.entry(key_id).or_insert(0);
    *consecutive = consecutive.saturating_add(1);
    if *consecutive >= DIRECT_INVALID_API_KEY_THRESHOLD {
        DirectAuthFailureDecision::CircuitOpened {
            consecutive: *consecutive,
        }
    } else {
        DirectAuthFailureDecision::RetryAllowed {
            consecutive: *consecutive,
        }
    }
}

pub(crate) fn direct_auth_backoff_error(key_id: u64) -> Option<String> {
    let counts = failure_counts();
    let consecutive = counts.get(&key_id).copied().unwrap_or_default();
    (consecutive >= DIRECT_INVALID_API_KEY_THRESHOLD)
        .then(|| invalid_api_key_backoff_message(consecutive))
}

pub(crate) fn invalid_api_key_backoff_message(consecutive: u32) -> String {
    format!(
        "Direct-mode Composio API key was rejected {consecutive} consecutive times with HTTP 401 Invalid API key; re-enter a valid key in Settings > Connections > Composio to resume polling."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_http_401_is_not_classified_as_invalid_api_key() {
        assert!(!is_invalid_api_key_error("HTTP 401"));
        assert!(is_invalid_api_key_error("HTTP 401: Invalid API key"));
    }

    #[test]
    fn non_auth_failure_resets_invalid_key_streak() {
        let key_id = fingerprint_api_key("ck_test_streak_reset");
        reset_direct_auth_failure(key_id);

        assert_eq!(
            record_direct_auth_failure(key_id, "HTTP 401: Invalid API key"),
            DirectAuthFailureDecision::RetryAllowed { consecutive: 1 }
        );
        assert_eq!(
            record_direct_auth_failure(key_id, "HTTP 401: Invalid API key"),
            DirectAuthFailureDecision::RetryAllowed { consecutive: 2 }
        );
        assert_eq!(
            record_direct_auth_failure(key_id, "HTTP 500: upstream unavailable"),
            DirectAuthFailureDecision::NotAuthFailure
        );
        assert!(
            direct_auth_backoff_error(key_id).is_none(),
            "non-auth failures must clear stale invalid-key counts"
        );
        assert_eq!(
            record_direct_auth_failure(key_id, "HTTP 401: Invalid API key"),
            DirectAuthFailureDecision::RetryAllowed { consecutive: 1 }
        );

        reset_direct_auth_failure(key_id);
    }
}
